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

/// Command: get income and expense totals for an account over an explicit
/// `date_from`–`date_until` window.
pub const GET_ACCOUNT_STATS: &str = "get_account_stats";

/// Command: get period-bucketed cash-flow data for a sparkline chart.
pub const GET_ACCOUNT_SPARKLINE: &str = "get_account_sparkline";

/// Command: get the most recent transaction date for an account.
pub const ACCOUNT_LATEST_ACTIVITY: &str = "account_latest_activity";

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

/// Command: apply a desired transaction state (edit in place).
pub const EDIT_TRANSACTION: &str = "edit_transaction";

/// Command: set a transaction's reconciliation state.
pub const SET_RECONCILIATION: &str = "set_reconciliation";

/// Command: load the audit trail for a transaction.
pub const GET_TRANSACTION_AUDIT: &str = "get_transaction_audit";

/// Command: list all tags as id/path pairs.
pub const LIST_TAGS: &str = "list_tags";

/// Lists registered commodities/currencies.
pub const LIST_CURRENCIES: &str = "list_currencies";

/// Command: create a new commodity/currency.
pub const CREATE_CURRENCY: &str = "create_currency";
/// Command: update an existing commodity/currency.
pub const UPDATE_CURRENCY: &str = "update_currency";
/// Command: delete a commodity/currency.
pub const DELETE_CURRENCY: &str = "delete_currency";

/// Command: create the full colon-path tag hierarchy, returning the leaf ID.
pub const CREATE_TAG: &str = "create_tag";

/// Command: snapshot the database to the managed backup directory.
pub const BACKUP_DATABASE: &str = "backup_database";

/// Command: restore the database from a backup file (relaunches the app).
pub const RESTORE_DATABASE: &str = "restore_database";

/// Command: list existing backups.
pub const LIST_BACKUPS: &str = "list_backups";

/// Command: read the current backup settings.
pub const GET_BACKUP_SETTINGS: &str = "get_backup_settings";

/// Command: persist updated backup settings.
pub const UPDATE_BACKUP_SETTINGS: &str = "update_backup_settings";

/// Merge two single-posting transactions into one.
pub const MERGE_TRANSACTIONS: &str = "merge_transactions";

/// Reverse the most recent merge on a transaction.
pub const UNMERGE_TRANSACTION: &str = "unmerge_transaction";

/// Propose candidate transfer pairs for review.
pub const SUGGEST_TRANSFERS: &str = "suggest_transfers";

/// Command: run a structured transaction search.
pub const SEARCH_TRANSACTIONS: &str = "search_transactions";

/// Argument struct for the `reverse_transaction` command.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(serde::Serialize)]
pub(crate) struct ReverseTransactionArgs<'a> {
    /// The transaction ID to reverse.
    pub id: &'a str,
}

/// Argument struct for the `search_transactions` command.
#[cfg(any(target_arch = "wasm32", test))]
#[derive(serde::Serialize)]
pub(crate) struct SearchTransactionsArgs<'a> {
    /// The structured filter to apply.
    pub filter: &'a crate::Filter,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::ReverseTransactionArgs;
    use super::SearchTransactionsArgs;

    #[test]
    fn reverse_transaction_args_serialize() {
        let args = ReverseTransactionArgs { id: "tx-123" };
        let json = serde_json::to_value(&args).expect("serialize");
        assert_eq!(json, serde_json::json!({ "id": "tx-123" }));
    }

    #[test]
    fn search_transactions_args_serialize() {
        let filter = crate::Filter::default();
        let args = SearchTransactionsArgs { filter: &filter };
        let json = serde_json::to_value(&args).expect("serialize");
        assert!(json.get("filter").is_some());
    }
}
