//! Message types for the tui-realm event/update pipeline.
//!
//! [`Msg`] is the key type for `Application<Id, Msg, NoUserEvent>`.
//! `Model::update()` handles cross-cutting variants directly and delegates
//! screen-specific variants to `Screen::handle()`.

use crate::id::Id;
use crate::mode::AppMode;

/// Top-level message type.
///
/// Cross-cutting variants are handled directly in `Model::update()`.
/// Screen-specific variants are delegated to the active `Screen::handle()`.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum Msg {
    /// Quit the application.
    AppQuit,
    /// Switch to a different top-level tab.
    TabSwitch(Tab),
    /// Change the current input mode.
    ModeChange(AppMode),
    /// Toggle the help overlay visibility.
    HelpToggle,
    /// Move keyboard focus to the given component ID.
    FocusChange(Id),
    /// Chrome layer messages (reserved for future use).
    Chrome(ChromeMsg),
    /// Accounts screen messages.
    Accounts(AccountsMsg),
    /// Budget screen messages.
    Budget(BudgetMsg),
    /// Reports screen messages.
    Reports(ReportsMsg),
}

/// The top-level tabs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Tab {
    /// Account tree + transaction list/detail.
    Accounts,
    /// Envelope tree + budget status.
    Budget,
    /// Report selector + output.
    Reports,
}

impl Tab {
    /// Returns the display label for this tab.
    #[inline]
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Accounts => "Accounts",
            Self::Budget => "Budget",
            Self::Reports => "Reports",
        }
    }
}

/// Messages produced by the chrome layer.
///
/// Reserved for future chrome-specific messages. Chrome components currently
/// emit cross-cutting [`Msg`] variants directly.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum ChromeMsg {}

/// Messages produced by the accounts screen.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum AccountsMsg {
    /// User selected a different account in the sidebar.
    AccountSelected(bc_models::AccountId),
    /// User wants to add a new transaction.
    OpenAddTransaction,
    /// User wants to edit the selected transaction.
    OpenEditTransaction,
    /// User requested to void the selected transaction (`d` key).
    ///
    /// No confirmation dialog is shown yet; the void is applied immediately.
    VoidRequested,
    /// User cancelled an open form.
    FormCancelled,
    /// A transaction form was submitted successfully.
    FormSubmitted,
    /// User pressed `l`/`→`/`Enter` on the list — open the detail panel.
    OpenDetail(bc_models::TransactionId),
    /// User pressed `Esc`/`h` on the detail panel — return focus to the list.
    CloseDetail,
    /// User pressed `h`/`←` on the list — return focus to the sidebar.
    FocusSidebar,
}

/// Messages produced by the budget screen.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum BudgetMsg {
    /// User selected a different envelope in the sidebar.
    EnvelopeSelected(bc_models::EnvelopeId),
    /// User wants to allocate funds to the selected envelope.
    OpenAllocate,
    /// User cancelled an open form.
    FormCancelled,
    /// An allocation form was submitted successfully.
    FormSubmitted,
}

/// Messages produced by the reports screen.
#[derive(Debug, PartialEq)]
#[non_exhaustive]
pub enum ReportsMsg {
    /// User selected a report to run.
    RunReport(ReportKind),
    /// User pressed `r` to refresh the current report.
    Refresh,
    /// User pressed `Esc` to return to the report selector.
    BackToSelector,
}

/// The available report types.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReportKind {
    /// Overall net worth across all accounts.
    NetWorth,
    /// Monthly income/expense summary.
    MonthlySummary,
    /// Budget allocation vs actuals.
    BudgetSummary,
}
