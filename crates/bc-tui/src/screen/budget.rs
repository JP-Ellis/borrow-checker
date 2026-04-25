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
use tuirealm::Application;
use tuirealm::Frame;
use tuirealm::NoUserEvent;
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
/// Owns the envelope sidebar and envelope status detail panel.
/// Handles [`BudgetMsg`] variants delegated from `Model::update()`.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as budget::BudgetScreen; repetition is intentional"
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
        }
    }

    /// Load all active envelopes from the database into `self.envelopes`.
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

    /// Load the budget status for the currently selected envelope.
    ///
    /// Returns early if no envelope is selected. On success, stores the
    /// [`EnvelopeStatus`] in `self.selected_status`.
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
        let today = jiff::Zoned::now().date();
        match self
            .ctx
            .block_on(self.ctx.budget.status_for(&envelope, today))
        {
            Ok(status) => self.selected_status = Some(status),
            Err(e) => eprintln!("failed to load envelope status: {e}"),
        }
    }

    /// Handle a [`BudgetMsg`], updating internal state and returning a follow-up [`Msg`] if needed.
    #[inline]
    fn handle_budget_msg(&mut self, msg: BudgetMsg) -> Option<Msg> {
        match msg {
            BudgetMsg::EnvelopeSelected(id) => {
                self.selected_envelope = Some(id);
                self.load_status();
                None
            }
            BudgetMsg::OpenAllocate => Some(Msg::ModeChange(AppMode::Insert)),
            BudgetMsg::FormCancelled | BudgetMsg::FormSubmitted => {
                Some(Msg::ModeChange(AppMode::Normal))
            }
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
        self.load_envelopes();
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
    }

    /// Render the budget screen: sidebar on the left (30%), detail panel on the right (70%).
    #[inline]
    #[expect(
        clippy::indexing_slicing,
        clippy::missing_asserts_for_indexing,
        reason = "layout always returns exactly 2 chunks to match the 2 constraints"
    )]
    fn view(&mut self, app: &mut Application<Id, Msg, NoUserEvent>, frame: &mut Frame, area: Rect) {
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
            .split(area);
        app.view(&Id::Budget(BudgetId::Sidebar), frame, h_chunks[0]);
        app.view(&Id::Budget(BudgetId::Detail), frame, h_chunks[1]);
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
    use core::time::Duration;
    use std::sync::Arc;

    use tuirealm::Application;
    use tuirealm::EventListenerCfg;
    use tuirealm::NoUserEvent;

    use super::*;
    use crate::context::TuiContext;
    use crate::id::BudgetId;
    use crate::id::Id;
    use crate::msg::Msg;

    #[tokio::test(flavor = "multi_thread")]
    async fn mount_and_unmount_without_panic() {
        let dir = assert_fs::TempDir::new().expect("create temp dir");
        let ctx = Arc::new(
            TuiContext::open(&dir.path().join("test.db"))
                .await
                .expect("open test ctx"),
        );
        let mut app: Application<Id, Msg, NoUserEvent> =
            Application::init(EventListenerCfg::default().poll_timeout(Duration::from_millis(10)));
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
}
