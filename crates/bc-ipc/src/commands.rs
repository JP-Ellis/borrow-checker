//! Command name constants shared between the Tauri backend and the WASM client.
//!
//! Both sides reference the same constant strings so a rename on either end
//! produces a compile error rather than a silent runtime mismatch.

/// Command: list all active accounts.
pub const LIST_ACCOUNTS: &str = "list_accounts";

/// Command: list transactions for a specific account.
pub const LIST_TRANSACTIONS: &str = "list_transactions";

/// Command: create a new transaction.
pub const CREATE_TRANSACTION: &str = "create_transaction";

/// Command: get income and expense totals for an account over the last 30 days.
pub const GET_ACCOUNT_STATS: &str = "get_account_stats";

/// Command: get period-bucketed cash-flow data for a sparkline chart.
pub const GET_ACCOUNT_SPARKLINE: &str = "get_account_sparkline";

/// Command: get the count of non-voided postings for an account.
pub const GET_POSTING_COUNT: &str = "get_posting_count";

/// Command: list installed plugins.
pub const LIST_PLUGINS: &str = "list_plugins";

/// Command: get the current application settings.
pub const GET_SETTINGS: &str = "get_settings";

/// Command: get budget tree and summary for a display window.
pub const GET_BUDGET_OVERVIEW: &str = "get_budget_overview";

/// Command: get native period breakdown for one budget in a display window.
pub const GET_NATIVE_PERIODS: &str = "get_native_periods";

/// Command: get transactions matched by a budget in a date range.
pub const GET_BUDGET_TRANSACTIONS: &str = "get_budget_transactions";

/// Command: archive a budget.
pub const ARCHIVE_BUDGET: &str = "archive_budget";

/// Command: create a new budget on an account.
pub const CREATE_BUDGET: &str = "create_budget";

/// Lists a budget's revisions for a display window.
pub const LIST_BUDGET_REVISIONS: &str = "list_budget_revisions";

/// Resolves a snap effective date to the next grid boundary.
pub const RESOLVE_EFFECTIVE_DATE: &str = "resolve_effective_date";

/// Adds or amends a budget revision.
pub const REVISE_BUDGET: &str = "revise_budget";

/// Removes a budget revision.
pub const REMOVE_BUDGET_REVISION: &str = "remove_budget_revision";

/// Command: set an accrual spread on a posting.
pub const SET_POSTING_SPREAD: &str = "set_posting_spread";

/// Command: clear the accrual spread from a posting.
pub const CLEAR_POSTING_SPREAD: &str = "clear_posting_spread";

/// Reverses a transaction, creating a linked negated reversal transaction.
pub const REVERSE_TRANSACTION: &str = "reverse_transaction";
