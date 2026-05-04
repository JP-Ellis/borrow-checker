//! Reports screen — report selector and output view.
//!
//! This module owns the single component that makes up the Reports tab:
//! - [`view::ReportView`] — left/full-width panel for selecting and viewing reports

pub mod view;

use std::sync::Arc;

use tuirealm::application::Application;
use tuirealm::event::NoUserEvent;
use tuirealm::props::AttrValue;
use tuirealm::props::Attribute;
use tuirealm::ratatui::Frame;
use tuirealm::ratatui::layout::Rect;

use crate::context::TuiContext;
use crate::id::Id;
use crate::id::ReportsId;
use crate::mode::AppMode;
use crate::msg::Msg;
use crate::msg::ReportKind;
use crate::msg::ReportsMsg;
use crate::screen::KeyBinding;
use crate::screen::Screen;

/// The reports tab screen.
///
/// Owns the combined report selector and output view component.
/// Handles [`ReportsMsg`] variants delegated from `Model::update()`.
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as reports::ReportsScreen; repetition is intentional"
)]
#[non_exhaustive]
pub struct ReportsScreen {
    /// Shared bc-core services.
    ctx: Arc<TuiContext>,
    /// Buffered report output to flush into the view component on the next `view()` call.
    pending_output: Option<String>,
    /// The last report kind that was run, used for refresh.
    last_kind: Option<ReportKind>,
}

impl ReportsScreen {
    /// Create a new `ReportsScreen` bound to the given context.
    ///
    /// No data is loaded until a report is selected.
    ///
    /// # Arguments
    ///
    /// * `ctx` - Shared bc-core services and tokio handle.
    ///
    /// # Returns
    ///
    /// A new `ReportsScreen` ready to be mounted.
    #[inline]
    #[must_use]
    pub fn new(ctx: Arc<TuiContext>) -> Self {
        Self {
            ctx,
            pending_output: None,
            last_kind: None,
        }
    }

    /// Run the Net Worth report and return the formatted output.
    ///
    /// # Returns
    ///
    /// A multi-line string with net worth in AUD, or an error message.
    #[inline]
    fn run_net_worth(&self) -> String {
        match self.ctx.block_on(self.ctx.balances.net_worth("AUD")) {
            Ok(value) => format!(
                "Net Worth\n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\nAUD {value}"
            ),
            Err(e) => format!("Error running Net Worth report: {e}"),
        }
    }

    /// Run the Monthly Summary report and return the formatted output.
    ///
    /// Shows the count of transactions recorded this calendar month.
    ///
    /// # Returns
    ///
    /// A multi-line string summarising this month's transaction count.
    #[inline]
    fn run_monthly_summary(&self) -> String {
        match self.ctx.block_on(self.ctx.transactions.list()) {
            Err(e) => format!("Error running Monthly Summary report: {e}"),
            Ok(txns) => {
                let today = jiff::Zoned::now().date();
                #[expect(
                    clippy::expect_used,
                    reason = "with().day(1).build() only fails if day is out of range for the current month; day 1 is always valid"
                )]
                let month_start = today.with().day(1).build().expect("day 1 is always valid");

                let this_month: Vec<_> = txns
                    .iter()
                    .filter(|t| t.date() >= month_start && t.date() <= today)
                    .collect();

                format!(
                    "Monthly Summary — {year}-{month:02}\n\
                     \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\n\
                     Transactions this month: {count}\n\
                     Total transactions:      {total}",
                    year = today.year(),
                    month = today.month(),
                    count = this_month.len(),
                    total = txns.len(),
                )
            }
        }
    }

    /// Run the Budget Summary report and return the formatted output.
    ///
    /// Loads all envelopes and their budget status as of today, then formats
    /// a table of allocated / actual / available amounts.
    ///
    /// # Returns
    ///
    /// A multi-line string summarising each envelope's budget status.
    #[inline]
    fn run_budget_summary(&self) -> String {
        let envelopes = match self.ctx.block_on(self.ctx.envelopes.list()) {
            Ok(e) => e,
            Err(e) => return format!("Error loading envelopes: {e}"),
        };

        let today = jiff::Zoned::now().date();
        let statuses = match self
            .ctx
            .block_on(self.ctx.budget.status_all(&envelopes, today))
        {
            Ok(s) => s,
            Err(e) => return format!("Error running Budget Summary report: {e}"),
        };

        if statuses.is_empty() {
            return "Budget Summary\n\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\nNo envelopes configured.".to_owned();
        }

        let mut lines = vec![
            "Budget Summary".to_owned(),
            "\u{2500}".repeat(56),
            format!(
                "{:<20}  {:>10}  {:>10}  {:>10}",
                "Envelope", "Allocated", "Actual", "Available"
            ),
            "\u{2500}".repeat(56),
        ];

        for s in &statuses {
            lines.push(format!(
                "{:<20}  {:>10}  {:>10}  {:>10}",
                s.envelope.name(),
                s.allocated,
                s.actuals,
                s.available,
            ));
        }

        lines.join("\n")
    }

    /// Run the report of the given kind and store output in `self.pending_output`.
    ///
    /// # Arguments
    ///
    /// * `kind` - Which report to run.
    #[inline]
    fn run_report(&mut self, kind: ReportKind) {
        let output = match &kind {
            ReportKind::NetWorth => self.run_net_worth(),
            ReportKind::MonthlySummary => self.run_monthly_summary(),
            ReportKind::BudgetSummary => self.run_budget_summary(),
        };
        self.last_kind = Some(kind);
        self.pending_output = Some(output);
    }

    /// Handle a [`ReportsMsg`], running reports and storing pending output.
    ///
    /// Returns `None` — output is flushed asynchronously via `view()`.
    #[inline]
    fn handle_reports_msg(&mut self, msg: ReportsMsg) -> Option<Msg> {
        match msg {
            ReportsMsg::RunReport(kind) => {
                self.run_report(kind);
            }
            ReportsMsg::Refresh => {
                if let Some(kind) = self.last_kind.clone() {
                    self.run_report(kind);
                }
            }
            ReportsMsg::BackToSelector => {}
        }
        None
    }
}

impl Screen for ReportsScreen {
    /// Mount the reports screen components into the application.
    ///
    /// Mounts the combined report selector and output view.
    ///
    /// # Errors
    ///
    /// Returns an error if the component fails to mount (e.g., duplicate ID).
    #[inline]
    fn mount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) -> anyhow::Result<()> {
        app.mount(
            Id::Reports(ReportsId::View),
            Box::new(view::ReportView::new()),
            vec![],
        )?;
        Ok(())
    }

    /// Unmount the reports screen component from the application.
    #[inline]
    #[expect(
        clippy::unused_result_ok,
        reason = "unmount errors are non-fatal; component may already be absent"
    )]
    fn unmount(&mut self, app: &mut Application<Id, Msg, NoUserEvent>) {
        app.umount(&Id::Reports(ReportsId::View)).ok();
    }

    /// Render the reports screen.
    ///
    /// Flushes any pending report output into the view component via
    /// `app.attr()`, then renders the component into the full area.
    #[inline]
    #[expect(
        clippy::unused_result_ok,
        reason = "attr errors are non-fatal; best-effort output flush"
    )]
    fn view(&mut self, app: &mut Application<Id, Msg, NoUserEvent>, frame: &mut Frame, area: Rect) {
        if let Some(output) = self.pending_output.take() {
            app.attr(
                &Id::Reports(ReportsId::View),
                Attribute::Text,
                AttrValue::String(output),
            )
            .ok();
        }
        app.view(&Id::Reports(ReportsId::View), frame, area);
    }

    /// Handle a message destined for the reports screen.
    ///
    /// Delegates [`Msg::Reports`] variants to [`Self::handle_reports_msg`].
    /// Returns `None` for any unrecognised message.
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Msg is non-exhaustive; non-Reports variants are intentionally ignored"
    )]
    fn handle(&mut self, msg: Msg) -> Option<Msg> {
        match msg {
            Msg::Reports(reports_msg) => self.handle_reports_msg(reports_msg),
            _ => None,
        }
    }

    /// Returns the report view as the initial focus target.
    #[inline]
    fn initial_focus(&self) -> Id {
        Id::Reports(ReportsId::View)
    }

    /// Returns the keybindings for the reports screen in the given mode.
    ///
    /// - Normal: 5 bindings (navigation, run, refresh, back)
    /// - Insert/Visual: empty
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
                    action: "Run report".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "r".into(),
                    action: "Refresh report".into(),
                    mode: AppMode::Normal,
                },
                KeyBinding {
                    key: "Esc".into(),
                    action: "Back to selector".into(),
                    mode: AppMode::Normal,
                },
            ],
            AppMode::Insert | AppMode::Visual => vec![],
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
    use crate::id::Id;
    use crate::id::ReportsId;
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
            Application::init(EventListenerCfg::default());
        let mut screen = ReportsScreen::new(ctx);
        tokio::task::block_in_place(|| {
            screen.mount(&mut app).expect("mount");
        });
        assert!(app.mounted(&Id::Reports(ReportsId::View)));
        screen.unmount(&mut app);
        assert!(!app.mounted(&Id::Reports(ReportsId::View)));
    }

    #[test]
    fn handle_non_reports_msg_returns_none() {
        let ctx = {
            // We need a dummy context; use a not-yet-opened path — we never call block_on here.
            // Build a minimal context by running open in a temporary runtime.
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("build rt");
            let dir = assert_fs::TempDir::new().expect("create temp dir");
            let db_path = dir.path().join("test.db");
            Arc::new(rt.block_on(TuiContext::open(&db_path)).expect("open ctx"))
        };
        let mut screen = ReportsScreen::new(ctx);
        pretty_assertions::assert_eq!(screen.handle(Msg::AppQuit), None);
    }
}
