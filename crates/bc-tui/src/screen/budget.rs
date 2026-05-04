//! Budget screen — envelope tree sidebar and status detail panel.
//!
//! This module owns the two components that make up the Budget tab:
//! - [`sidebar::EnvelopeSidebar`] — left panel showing the envelope hierarchy
//! - [`detail::EnvelopeDetail`] — right panel showing the selected envelope's budget status

pub mod detail;
pub mod forms;
pub mod sidebar;

use std::sync::Arc;

use bc_core::EnvelopeStatus;
use bc_models::Envelope;
use bc_models::EnvelopeId;
use tuirealm::application::Application;
use tuirealm::event::NoUserEvent;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Constraint;
use tuirealm::ratatui::layout::Direction;
use tuirealm::ratatui::layout::Layout;
use tuirealm::ratatui::layout::Rect;

use crate::context::TuiContext;
use crate::id::BudgetId;
use crate::id::Id;
use crate::mode::AppMode;
use crate::msg::BudgetMsg;
use crate::msg::Msg;
use crate::screen::KeyBinding;
use crate::screen::Screen;

/// The budget tab screen.
///
/// Owns the envelope sidebar, envelope status detail panel, and allocation form overlay.
/// Handles [`BudgetMsg`] variants delegated from `Model::update()`.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as budget::BudgetScreen; repetition is intentional"
)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "five independent state flags: loading, detail dirty tracking, focus-restore, form state — not reducible to an enum"
)]
#[non_exhaustive]
pub struct BudgetScreen {
    /// Shared bc-core services.
    ctx: Arc<TuiContext>,
    /// All active envelopes loaded from the database.
    envelopes: Vec<Envelope>,
    /// The envelope currently selected in the sidebar, if any.
    selected_envelope: Option<EnvelopeId>,
    /// Budget status for the currently selected envelope.
    selected_status: Option<EnvelopeStatus>,
    /// Whether envelopes are currently being loaded from the database.
    loading: bool,
    /// Whether the detail panel needs to be updated on the next `view()` call.
    detail_dirty: bool,
    /// Whether the allocation form should be mounted on the next render.
    pending_form: bool,
    /// Whether the allocation form is currently mounted.
    form_mounted: bool,
    /// Whether to move keyboard focus to the detail panel after the next detail remount.
    focus_detail_after_dirty: bool,
    /// Ordered list of period presets built when an envelope is selected.
    window_presets: Vec<bc_models::BudgetWindow>,
    /// Index into `window_presets` for the currently displayed period.
    selected_window_idx: usize,
}

impl BudgetScreen {
    /// Create a new `BudgetScreen` bound to the given context.
    ///
    /// Data is not loaded until [`Screen::mount`] is called.
    #[inline]
    #[must_use]
    pub fn new(ctx: Arc<TuiContext>) -> Self {
        Self {
            ctx,
            envelopes: Vec::new(),
            selected_envelope: None,
            selected_status: None,
            loading: false,
            detail_dirty: false,
            pending_form: false,
            form_mounted: false,
            focus_detail_after_dirty: false,
            window_presets: Vec::new(),
            selected_window_idx: 0,
        }
    }

    /// Load all active envelopes from the database into `self.envelopes`.
    #[inline]
    #[expect(
        clippy::print_stderr,
        reason = "load errors are logged to stderr since we are in raw terminal mode"
    )]
    fn load_envelopes(&mut self) {
        match self.ctx.block_on(self.ctx.envelopes.list()) {
            Ok(envelopes) => self.envelopes = envelopes,
            Err(e) => eprintln!("failed to load envelopes: {e}"),
        }
    }

    /// Returns the currently active [`bc_models::BudgetWindow`].
    ///
    /// Falls back to last month when no presets are loaded.
    #[inline]
    fn selected_window(&self) -> bc_models::BudgetWindow {
        let today = jiff::Zoned::now().date();
        self.window_presets
            .get(self.selected_window_idx)
            .cloned()
            .unwrap_or_else(|| bc_models::BudgetWindow::last_month(today))
    }

    /// Load the budget status for the currently selected envelope.
    ///
    /// Returns early if no envelope is selected. On success, stores the
    /// [`EnvelopeStatus`] in `self.selected_status`.
    #[inline]
    #[expect(
        clippy::print_stderr,
        reason = "load errors are logged to stderr since we are in raw terminal mode"
    )]
    fn load_status(&mut self) {
        let Some(ref id) = self.selected_envelope else {
            return;
        };
        let Some(envelope) = self.envelopes.iter().find(|e| e.id() == id).cloned() else {
            return;
        };
        let window = self.selected_window();
        match self
            .ctx
            .block_on(self.ctx.budget.status_for_window(&envelope, window))
        {
            Ok(status) => self.selected_status = Some(status),
            Err(e) => eprintln!("failed to load envelope status: {e}"),
        }
    }

    /// Parse the amount string and allocate funds to the selected envelope.
    ///
    /// Errors are logged to stderr — no attempt is made to surface them in the UI.
    #[inline]
    #[expect(
        clippy::print_stderr,
        reason = "allocation errors are logged to stderr since we are in raw terminal mode"
    )]
    fn allocate_to_envelope(&mut self, amount_str: &str) {
        let Some((value_str, commodity_str)) = amount_str.rsplit_once(' ') else {
            eprintln!("invalid amount '{amount_str}': expected 'VALUE COMMODITY'");
            return;
        };
        let value = match value_str.parse::<bc_models::Decimal>() {
            Ok(v) => v,
            Err(e) => {
                eprintln!("invalid amount value '{value_str}': {e}");
                return;
            }
        };
        let commodity = bc_models::CommodityCode::new(commodity_str);
        let amount = bc_models::Amount::new(value, commodity);
        let Some(ref envelope_id) = self.selected_envelope else {
            eprintln!("no envelope selected");
            return;
        };
        let today = jiff::Zoned::now().date();
        match self
            .ctx
            .block_on(self.ctx.envelopes.allocate(envelope_id, today, amount))
        {
            Ok(_) => {}
            Err(e) => eprintln!("failed to allocate: {e}"),
        }
    }

    /// Handle a [`BudgetMsg`], updating internal state and returning a follow-up [`Msg`] if needed.
    #[inline]
    fn handle_budget_msg(&mut self, msg: BudgetMsg) -> Option<Msg> {
        match msg {
            BudgetMsg::EnvelopeSelected(id) => {
                self.selected_envelope = Some(id);
                let today = jiff::Zoned::now().date();
                self.window_presets = bc_models::BudgetWindow::standard_presets(today);
                self.selected_window_idx = 0;
                self.load_status();
                // Fallback to "Last Month" (index 1) when the current month has no data.
                if let Some(ref s) = self.selected_status
                    && s.allocated.is_zero()
                    && s.actuals.is_zero()
                {
                    self.selected_window_idx = 1;
                    self.load_status();
                }
                self.detail_dirty = true;
                self.focus_detail_after_dirty = true;
                None
            }
            BudgetMsg::OpenAllocate => self.selected_envelope.is_some().then(|| {
                self.pending_form = true;
                Msg::ModeChange(AppMode::Insert)
            }),
            BudgetMsg::FormCancelled => {
                self.pending_form = false;
                // form_mounted stays true — view() will unmount and clear it
                Some(Msg::ModeChange(AppMode::Normal))
            }
            BudgetMsg::FormSubmitted { amount } => {
                self.pending_form = false;
                // form_mounted stays true — view() will unmount and clear it
                self.allocate_to_envelope(&amount);
                self.load_status();
                self.detail_dirty = true;
                Some(Msg::ModeChange(AppMode::Normal))
            }
            BudgetMsg::PeriodPrev => {
                if !self.window_presets.is_empty() {
                    self.selected_window_idx = self
                        .selected_window_idx
                        .checked_sub(1)
                        .unwrap_or_else(|| self.window_presets.len().saturating_sub(1));
                    self.load_status();
                    self.detail_dirty = true;
                }
                None
            }
            BudgetMsg::PeriodNext => {
                if !self.window_presets.is_empty() {
                    #[expect(
                        clippy::arithmetic_side_effects,
                        clippy::integer_division_remainder_used,
                        reason = "modulo wraps within bounds; presets.len() is non-zero"
                    )]
                    {
                        self.selected_window_idx =
                            (self.selected_window_idx + 1) % self.window_presets.len();
                    }
                    self.load_status();
                    self.detail_dirty = true;
                }
                None
            }
            BudgetMsg::FocusSidebar => Some(Msg::FocusChange(Id::Budget(BudgetId::Sidebar))),
        }
    }
}

impl Screen for BudgetScreen {
    /// Mount the budget screen components into the application.
    ///
    /// Loads envelopes from the database, then mounts the sidebar and detail components.
    ///
    /// # Errors
    ///
    /// Returns an error if any component fails to mount (e.g., duplicate ID).
    #[inline]
    fn mount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) -> anyhow::Result<()> {
        self.loading = true;
        self.load_envelopes();
        self.loading = false;
        app.mount(
            Id::Budget(BudgetId::Sidebar),
            Box::new(sidebar::EnvelopeSidebar::new(self.envelopes.clone())),
            vec![],
        )?;
        app.mount(
            Id::Budget(BudgetId::Detail),
            Box::new(detail::EnvelopeDetail::new(None)),
            vec![],
        )?;
        Ok(())
    }

    /// Unmount all budget screen components from the application.
    #[inline]
    #[expect(
        clippy::unused_result_ok,
        reason = "unmount errors are non-fatal; component may already be absent"
    )]
    fn unmount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) {
        app.umount(&Id::Budget(BudgetId::Sidebar)).ok();
        app.umount(&Id::Budget(BudgetId::Detail)).ok();
        app.umount(&Id::Budget(BudgetId::AllocationForm)).ok();
    }

    /// Render the budget screen: sidebar on the left (30%), detail panel on the right (70%).
    ///
    /// If the selected envelope changed since the last render, the detail panel is
    /// re-mounted with the updated [`EnvelopeStatus`] before rendering.
    ///
    /// The allocation form overlay is mounted/unmounted based on `pending_form` state,
    /// and rendered on top of the rest of the screen when `form_mounted` is true.
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 2 chunks to match the 2 constraints"
    )]
    #[expect(
        clippy::print_stderr,
        reason = "mount errors logged to stderr since we are in raw terminal mode"
    )]
    fn view(&mut self, app: &mut Application<Id, Msg, NoUserEvent>, frame: &mut Frame, area: Rect) {
        // Mount or unmount the form overlay based on pending_form state.
        if self.pending_form && !self.form_mounted {
            let envelope_name = self
                .selected_envelope
                .as_ref()
                .and_then(|id| self.envelopes.iter().find(|e| e.id() == id))
                .map(|e| e.name().to_owned())
                .unwrap_or_default();
            #[expect(
                clippy::unused_result_ok,
                reason = "pre-unmount is best-effort; component may not be present"
            )]
            {
                app.umount(&Id::Budget(BudgetId::AllocationForm)).ok();
            }
            match app.mount(
                Id::Budget(BudgetId::AllocationForm),
                Box::new(forms::AllocationForm::new(envelope_name)),
                vec![],
            ) {
                Ok(()) => {
                    #[expect(
                        clippy::unused_result_ok,
                        reason = "focus is best-effort; form is still visible if active() fails"
                    )]
                    {
                        app.active(&Id::Budget(BudgetId::AllocationForm)).ok();
                    }
                    self.form_mounted = true;
                }
                Err(e) => {
                    eprintln!("failed to mount allocation form: {e}");
                    self.pending_form = false;
                }
            }
        } else if !self.pending_form && self.form_mounted {
            #[expect(
                clippy::unused_result_ok,
                reason = "unmount errors are non-fatal; component may already be absent"
            )]
            {
                app.umount(&Id::Budget(BudgetId::AllocationForm)).ok();
            }
            self.form_mounted = false;
        }

        if self.loading {
            frame.render_widget(
                tuirealm::ratatui::widgets::Paragraph::new("Loading envelopes\u{2026}"),
                area,
            );
            return;
        }

        if self.detail_dirty {
            self.detail_dirty = false;
            #[expect(
                clippy::unused_result_ok,
                reason = "umount errors are non-fatal; component may already be absent"
            )]
            {
                app.umount(&Id::Budget(BudgetId::Detail)).ok();
            }
            if let Err(e) = app.mount(
                Id::Budget(BudgetId::Detail),
                Box::new(detail::EnvelopeDetail::new(self.selected_status.clone())),
                vec![],
            ) {
                eprintln!("failed to re-mount envelope detail: {e}");
            }
        }

        if self.focus_detail_after_dirty && !self.form_mounted {
            self.focus_detail_after_dirty = false;
            #[expect(
                clippy::unused_result_ok,
                reason = "focus restore is best-effort after re-mount"
            )]
            {
                app.active(&Id::Budget(BudgetId::Detail)).ok();
            }
        }

        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);
        app.view(&Id::Budget(BudgetId::Sidebar), frame, h_chunks[0]);
        app.view(&Id::Budget(BudgetId::Detail), frame, h_chunks[1]);

        // Render the form overlay on top if mounted.
        if self.form_mounted {
            app.view(&Id::Budget(BudgetId::AllocationForm), frame, area);
        }
    }

    /// Handle a message destined for the budget screen.
    ///
    /// Delegates [`Msg::Budget`] variants to [`Self::handle_budget_msg`].
    /// Returns `None` for any unrecognised message.
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Msg is non-exhaustive; non-Budget variants are intentionally ignored"
    )]
    fn handle(&mut self, msg: Msg) -> Option<Msg> {
        match msg {
            Msg::Budget(budget_msg) => self.handle_budget_msg(budget_msg),
            _ => None,
        }
    }

    /// Returns the sidebar as the initial focus target.
    #[inline]
    fn initial_focus(&self) -> Id {
        Id::Budget(BudgetId::Sidebar)
    }

    /// Returns the keybindings for the budget screen in the given mode.
    ///
    /// - Normal: 4 bindings (navigation, select envelope, allocate funds)
    /// - Insert: 4 bindings (form submit/cancel, next/previous field)
    /// - Visual: empty (visual mode is not used in this screen)
    #[inline]
    fn keybindings(&self, mode: &AppMode) -> Vec<KeyBinding> {
        match mode {
            AppMode::Normal => vec![
                KeyBinding {
                    key: "j / ↓".into(),
                    action: "Move down".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "k / ↑".into(),
                    action: "Move up".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "Enter".into(),
                    action: "Select envelope".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "a".into(),
                    action: "Allocate funds".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "[ / ]".into(),
                    action: "Previous / next period".into(),
                    mode: AppMode::Normal,
                },
            ],
            AppMode::Insert => vec![
                KeyBinding {
                    key: "Enter".into(),
                    action: "Submit form".into(),
                    mode: AppMode::Insert,
                },
                KeyBinding {
                    key: "Esc".into(),
                    action: "Cancel form".into(),
                    mode: AppMode::Insert,
                },
                KeyBinding {
                    key: "Tab".into(),
                    action: "Next field".into(),
                    mode: AppMode::Insert,
                },
                KeyBinding {
                    key: "Shift+Tab".into(),
                    action: "Previous field".into(),
                    mode: AppMode::Insert,
                },
            ],
            AppMode::Visual => vec![],
        }
    }
}

#[cfg(test)]
mod tests {

    use std::sync::Arc;

    use tuirealm::application::Application;
    use tuirealm::event::NoUserEvent;
    use tuirealm::listener::EventListenerCfg;

    use super::*;
    use crate::context::TuiContext;
    use crate::id::BudgetId;
    use crate::id::Id;
    use crate::msg::BudgetMsg;
    use crate::msg::Msg;

    fn make_ctx() -> Arc<TuiContext> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("build rt");
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        Arc::new(
            rt.block_on(TuiContext::open(&dir.path().join("test.db")))
                .expect("open ctx"),
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn mount_and_unmount_without_panic() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            TuiContext::open(&dir.path().join("test.db"))
                .await
                .expect("open test ctx"),
        );
        let mut app: Application<Id, Msg, NoUserEvent> =
            Application::init(EventListenerCfg::default());
        let mut screen = BudgetScreen::new(ctx);
        // block_in_place allows blocking calls (block_on) within a multi-threaded tokio runtime.
        tokio::task::block_in_place(|| {
            screen.mount(&mut app).expect("mount");
        });
        assert!(app.mounted(&Id::Budget(BudgetId::Sidebar)));
        assert!(app.mounted(&Id::Budget(BudgetId::Detail)));
        screen.unmount(&mut app);
        assert!(!app.mounted(&Id::Budget(BudgetId::Sidebar)));
    }

    #[test]
    fn open_allocate_without_selection_is_noop() {
        let mut screen = BudgetScreen::new(make_ctx());
        // No envelope selected — OpenAllocate should be a no-op.
        let result = screen.handle(Msg::Budget(BudgetMsg::OpenAllocate));
        pretty_assertions::assert_eq!(result, None);
        pretty_assertions::assert_eq!(screen.pending_form, false);
    }

    #[test]
    fn form_cancelled_returns_mode_change_normal() {
        let mut screen = BudgetScreen::new(make_ctx());
        screen.pending_form = true;
        screen.form_mounted = true;
        let result = screen.handle(Msg::Budget(BudgetMsg::FormCancelled));
        pretty_assertions::assert_eq!(result, Some(Msg::ModeChange(AppMode::Normal)));
        pretty_assertions::assert_eq!(screen.pending_form, false);
        // form_mounted is NOT cleared by handle(); view() does that on the next render.
        pretty_assertions::assert_eq!(screen.form_mounted, true);
    }

    #[test]
    fn form_submitted_returns_mode_change_normal() {
        let mut screen = BudgetScreen::new(make_ctx());
        screen.pending_form = true;
        screen.form_mounted = true;
        // Invalid amount — allocate_to_envelope logs to stderr and returns early.
        let result = screen.handle(Msg::Budget(BudgetMsg::FormSubmitted {
            amount: String::new(),
        }));
        pretty_assertions::assert_eq!(result, Some(Msg::ModeChange(AppMode::Normal)));
        pretty_assertions::assert_eq!(screen.pending_form, false);
        // form_mounted is NOT cleared by handle(); view() does that on the next render.
        pretty_assertions::assert_eq!(screen.form_mounted, true);
    }

    #[test]
    fn non_budget_msg_returns_none() {
        let mut screen = BudgetScreen::new(make_ctx());
        pretty_assertions::assert_eq!(screen.handle(Msg::AppQuit), None);
    }

    #[test]
    fn period_prev_wraps_to_last() {
        let mut screen = BudgetScreen::new(make_ctx());
        let date = jiff::civil::date(2026, 5, 1);
        screen.window_presets = vec![
            bc_models::BudgetWindow::last_month(date),
            bc_models::BudgetWindow::this_month(date),
        ];
        screen.selected_window_idx = 0;
        screen.handle(Msg::Budget(BudgetMsg::PeriodPrev));
        pretty_assertions::assert_eq!(screen.selected_window_idx, 1);
        pretty_assertions::assert_eq!(screen.detail_dirty, true);
    }

    #[test]
    fn period_next_wraps_to_first() {
        let mut screen = BudgetScreen::new(make_ctx());
        let date = jiff::civil::date(2026, 5, 1);
        screen.window_presets = vec![
            bc_models::BudgetWindow::last_month(date),
            bc_models::BudgetWindow::this_month(date),
        ];
        screen.selected_window_idx = 1;
        screen.handle(Msg::Budget(BudgetMsg::PeriodNext));
        pretty_assertions::assert_eq!(screen.selected_window_idx, 0);
        pretty_assertions::assert_eq!(screen.detail_dirty, true);
    }

    #[test]
    fn period_prev_next_noop_when_no_presets() {
        let mut screen = BudgetScreen::new(make_ctx());
        // window_presets is empty by default
        screen.handle(Msg::Budget(BudgetMsg::PeriodPrev));
        screen.handle(Msg::Budget(BudgetMsg::PeriodNext));
        pretty_assertions::assert_eq!(screen.selected_window_idx, 0);
        pretty_assertions::assert_eq!(screen.detail_dirty, false);
    }
}
