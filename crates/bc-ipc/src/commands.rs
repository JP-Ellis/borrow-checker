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

/// Command: get inflow, outflow, internal and balance stats for an account over
/// an explicit `date_from`–`date_until` window.
pub const GET_ACCOUNT_STATS: &str = "get_account_stats";

/// Command: get period-bucketed cash-flow data for a sparkline chart.
pub const GET_ACCOUNT_SPARKLINE: &str = "get_account_sparkline";

/// Command: list installed plugins.
pub const LIST_PLUGINS: &str = "list_plugins";

/// Command: get the current application settings.
pub const GET_SETTINGS: &str = "get_settings";

/// Command: get budget tree and summary for a display window.
pub const GET_BUDGET_OVERVIEW: &str = "get_budget_overview";

/// Command: get native period breakdown for one budget in a display window.
pub const GET_NATIVE_PERIODS: &str = "get_native_periods";

/// Command: get the transactions behind one budget tree row.
pub const GET_BUDGET_ROW_TRANSACTIONS: &str = "get_budget_row_transactions";

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

/// Command: list every registered metadata key with its type.
pub const LIST_METADATA_KEYS: &str = "list_metadata_keys";

/// Command: change a metadata key's registered type, re-asserting every stored
/// value against it.
pub const RETYPE_METADATA_KEY: &str = "retype_metadata_key";

/// Command: rename a metadata key, carrying its entries with it.
pub const RENAME_METADATA_KEY: &str = "rename_metadata_key";

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

/// Command: delete one backup from the open ledger's pool.
pub const DELETE_BACKUP: &str = "delete_backup";

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

/// Command: fetch one page of the account register with running balances.
pub const REGISTER_PAGE: &str = "register_page";

/// Command: load one transaction by ID.
pub const GET_TRANSACTION: &str = "get_transaction";

/// Command: rename a tag, keeping its subtree.
pub const RENAME_TAG: &str = "rename_tag";

/// Command: delete a tag.
pub const DELETE_TAG: &str = "delete_tag";

/// Every command name, for dispatch-coverage tests.
pub const ALL: &[&str] = &[
    LIST_ACCOUNTS,
    LIST_TRANSACTIONS,
    CREATE_TRANSACTION,
    EDIT_TRANSACTION,
    SET_RECONCILIATION,
    REVERSE_TRANSACTION,
    GET_ACCOUNT_STATS,
    SEARCH_TRANSACTIONS,
    REGISTER_PAGE,
    GET_TRANSACTION_AUDIT,
    GET_ACCOUNT_SPARKLINE,
    GET_TRANSACTION,
    BACKUP_DATABASE,
    LIST_BACKUPS,
    DELETE_BACKUP,
    RESTORE_DATABASE,
    GET_BACKUP_SETTINGS,
    UPDATE_BACKUP_SETTINGS,
    GET_BUDGET_OVERVIEW,
    GET_NATIVE_PERIODS,
    GET_BUDGET_ROW_TRANSACTIONS,
    LIST_BUDGET_REVISIONS,
    RESOLVE_EFFECTIVE_DATE,
    REVISE_BUDGET,
    REMOVE_BUDGET_REVISION,
    ARCHIVE_BUDGET,
    CREATE_BUDGET,
    SET_POSTING_SPREAD,
    CLEAR_POSTING_SPREAD,
    LIST_CURRENCIES,
    CREATE_CURRENCY,
    UPDATE_CURRENCY,
    DELETE_CURRENCY,
    LIST_METADATA_KEYS,
    RETYPE_METADATA_KEY,
    RENAME_METADATA_KEY,
    LIST_PLUGINS,
    GET_SETTINGS,
    CREATE_TAG,
    RENAME_TAG,
    DELETE_TAG,
    LIST_TAGS,
    MERGE_TRANSACTIONS,
    UNMERGE_TRANSACTION,
    SUGGEST_TRANSFERS,
];

// MARK: Argument structs

/// Arguments for the `list_transactions` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ListTransactionsArgs {
    /// Account whose transactions to list.
    pub account_id: String,
    /// Inclusive start.
    pub date_from: jiff::civil::Date,
    /// Exclusive end.
    pub date_until: jiff::civil::Date,
}

/// Arguments for the `create_transaction` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct CreateTransactionArgs {
    /// The transaction to create.
    pub tx: crate::NewTransaction,
}

/// Arguments for the `edit_transaction` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct EditTransactionArgs {
    /// The desired state.
    pub tx: crate::EditTransaction,
    /// The transaction as the editor loaded it.
    pub base: crate::EditTransaction,
}

/// Arguments for the `set_reconciliation` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SetReconciliationArgs {
    /// Transaction ID.
    pub id: String,
    /// The new state.
    pub reconciliation: crate::Reconciliation,
}

/// Arguments for the `reverse_transaction` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ReverseTransactionArgs {
    /// Transaction ID.
    pub id: String,
}

/// Arguments for the `get_account_stats` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetAccountStatsArgs {
    /// Account ID.
    pub account_id: String,
    /// Restrict to one commodity.
    pub commodity: Option<String>,
    /// Include descendant accounts.
    pub include_descendants: bool,
    /// Inclusive start.
    pub date_from: jiff::civil::Date,
    /// Exclusive end.
    pub date_until: jiff::civil::Date,
    /// Global transaction filter.
    pub filter: Option<crate::Filter>,
}

/// Arguments for the `search_transactions` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SearchTransactionsArgs {
    /// The query text and window to search.
    pub filter: crate::Filter,
}

/// Arguments for the `register_page` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RegisterPageArgs {
    /// The page request.
    pub request: crate::RegisterRequest,
}

/// Arguments for the `get_transaction_audit` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetTransactionAuditArgs {
    /// Transaction ID.
    pub id: String,
}

/// Arguments for the `get_account_sparkline` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetAccountSparklineArgs {
    /// Account ID.
    pub account_id: String,
    /// Restrict to one commodity.
    pub commodity: Option<String>,
    /// Include descendant accounts.
    pub include_descendants: bool,
    /// Number of buckets.
    pub count: Option<u32>,
    /// Bucket period.
    pub period: Option<crate::Period>,
    /// Last bucket's reference date.
    pub as_of: Option<jiff::civil::Date>,
    /// Global transaction filter.
    pub filter: Option<crate::Filter>,
}

/// Arguments for the `get_transaction` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetTransactionArgs {
    /// Transaction ID.
    pub id: String,
}

/// Arguments for the `restore_database` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RestoreDatabaseArgs {
    /// Path of the backup to restore.
    pub path: String,
}

/// Arguments for the `delete_backup` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct DeleteBackupArgs {
    /// The backup's file name, as `list_backups` reports it.
    pub file_name: String,
}

/// Arguments for the `update_backup_settings` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct UpdateBackupSettingsArgs {
    /// The settings to persist.
    pub settings: crate::BackupSettings,
}

/// Arguments for the `get_budget_overview` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetBudgetOverviewArgs {
    /// Display period.
    pub period_type: crate::Period,
    /// Display period start.
    pub period_start: jiff::civil::Date,
    /// Global transaction filter.
    pub filter: Option<crate::Filter>,
}

/// Arguments for the `get_native_periods` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetNativePeriodsArgs {
    /// Budget ID.
    pub budget_id: String,
    /// Inclusive display start.
    pub display_start: jiff::civil::Date,
    /// Exclusive display end.
    pub display_end: jiff::civil::Date,
    /// Global transaction filter.
    pub filter: Option<crate::Filter>,
}

/// Arguments for the `get_budget_row_transactions` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct GetBudgetRowTransactionsArgs {
    /// Budget tree row ID.
    pub row_id: String,
    /// Display period.
    pub period_type: crate::Period,
    /// Display period start.
    pub period_start: jiff::civil::Date,
    /// Global transaction filter.
    pub filter: Option<crate::Filter>,
}

/// Arguments for the `list_budget_revisions` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ListBudgetRevisionsArgs {
    /// Budget ID.
    pub budget_id: String,
    /// Inclusive display start.
    pub display_start: jiff::civil::Date,
    /// Exclusive display end.
    pub display_end: jiff::civil::Date,
}

/// Arguments for the `resolve_effective_date` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ResolveEffectiveDateArgs {
    /// Budget ID.
    pub budget_id: String,
    /// Requested date.
    pub date: jiff::civil::Date,
    /// Revision to ignore while resolving.
    pub exclude_revision_id: Option<String>,
}

/// Arguments for the `revise_budget` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ReviseBudgetArgs {
    /// Budget ID.
    pub budget_id: String,
    /// Revision to amend, or `None` to add one.
    pub revision_id: Option<String>,
    /// Revision start.
    pub effective_from: jiff::civil::Date,
    /// Display name.
    pub name: Option<String>,
    /// Target expression.
    pub target: Option<String>,
    /// Target currency.
    pub target_currency: Option<String>,
    /// Spend or save intent.
    pub intent: Option<crate::BudgetIntent>,
    /// Rollover policy.
    pub rollover: crate::RolloverPolicy,
    /// Budget period.
    pub period: crate::Period,
    /// Tag filter, or `None` if unfiltered; only the ID is read.
    pub tag_filter: Option<crate::TagInfo>,
}

/// Arguments for the `remove_budget_revision` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RemoveBudgetRevisionArgs {
    /// Budget ID.
    pub budget_id: String,
    /// Revision ID.
    pub revision_id: String,
}

/// Arguments for the `archive_budget` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ArchiveBudgetArgs {
    /// Budget ID.
    pub budget_id: String,
}

/// Arguments for the `create_budget` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct CreateBudgetArgs {
    /// Account the budget covers.
    pub account_id: String,
    /// First revision's start.
    pub effective_from: jiff::civil::Date,
    /// Display name.
    pub name: Option<String>,
    /// Target expression.
    pub target: Option<String>,
    /// Target currency.
    pub target_currency: Option<String>,
    /// Spend or save intent.
    pub intent: Option<crate::BudgetIntent>,
    /// Budget period.
    pub period: crate::Period,
    /// Rollover policy.
    pub rollover: crate::RolloverPolicy,
    /// Tag filter, or `None` if unfiltered; only the ID is read.
    pub tag_filter: Option<crate::TagInfo>,
}

/// Arguments for the `set_posting_spread` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SetPostingSpreadArgs {
    /// Posting ID.
    pub posting_id: String,
    /// Inclusive spread start.
    pub spread_from: jiff::civil::Date,
    /// Exclusive spread end.
    pub spread_until: jiff::civil::Date,
}

/// Arguments for the `clear_posting_spread` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ClearPostingSpreadArgs {
    /// Posting ID.
    pub posting_id: String,
}

/// Arguments for the `create_currency` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct CreateCurrencyArgs {
    /// The commodity to create.
    pub info: crate::CommodityInfo,
}

/// Arguments for the `update_currency` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct UpdateCurrencyArgs {
    /// The commodity's new state.
    pub info: crate::CommodityInfo,
}

/// Arguments for the `delete_currency` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct DeleteCurrencyArgs {
    /// Commodity ID.
    pub id: String,
}

/// Arguments for the `retype_metadata_key` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RetypeMetadataKeyArgs {
    /// The key to retype.
    pub key: String,
    /// The type to give it.
    pub ty: crate::MetaTypeDto,
}

/// Arguments for the `rename_metadata_key` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RenameMetadataKeyArgs {
    /// The key's current name.
    pub from: String,
    /// The name to give it.
    pub to: String,
}

/// Arguments for the `create_tag` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct CreateTagArgs {
    /// Colon-separated tag path.
    pub path: String,
}

/// Arguments for the `rename_tag` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RenameTagArgs {
    /// Tag ID.
    pub id: String,
    /// New leaf name.
    pub new_name: String,
}

/// Arguments for the `delete_tag` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct DeleteTagArgs {
    /// Tag ID.
    pub id: String,
}

/// Arguments for the `merge_transactions` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct MergeTransactionsArgs {
    /// Transaction that keeps both legs.
    pub survivor: String,
    /// Transaction merged into the survivor.
    pub absorbed: String,
}

/// Arguments for the `unmerge_transaction` command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct UnmergeTransactionArgs {
    /// The merged transaction.
    pub transaction: String,
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::BTreeSet;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn sample_edit() -> crate::EditTransaction {
        crate::EditTransaction::new(
            "tx-1".to_owned(),
            jiff::civil::date(2026, 1, 3),
            "Supermarket".to_owned(),
            Vec::new(),
            crate::Reconciliation::Unreconciled,
            Vec::new(),
            vec![crate::EditPosting::new(
                Some("p-1".to_owned()),
                "acct-1".to_owned(),
                None,
                Vec::new(),
                Vec::new(),
                None,
                None,
            )],
        )
    }

    #[test]
    fn all_names_are_unique() {
        let unique: BTreeSet<&str> = ALL.iter().copied().collect();
        assert_eq!(unique.len(), ALL.len(), "duplicate command name");
    }

    /// Reads every `pub const NAME: &str = "value";` from this file's source,
    /// so a constant added without an `ALL` entry fails here.
    #[test]
    fn all_lists_every_command_constant() {
        let declared: BTreeSet<&str> = include_str!("commands.rs")
            .lines()
            .filter_map(|line| line.strip_prefix("pub const "))
            .filter_map(|rest| rest.split_once(": &str = \""))
            .filter_map(|(_, value)| value.strip_suffix("\";"))
            .collect();
        let listed: BTreeSet<&str> = ALL.iter().copied().collect();
        assert!(declared.len() > 40, "the scan found only {declared:?}");
        assert_eq!(declared, listed);
    }

    #[rstest]
    #[case::reverse(serde_json::json!({ "id": "tx-1" }))]
    fn reverse_args_round_trip(#[case] json: serde_json::Value) {
        let args: ReverseTransactionArgs = serde_json::from_value(json.clone()).expect("de");
        assert_eq!(serde_json::to_value(&args).expect("ser"), json);
    }

    #[test]
    fn edit_args_carry_tx_and_base() {
        let json = serde_json::json!({ "tx": sample_edit(), "base": sample_edit() });
        let args: EditTransactionArgs = serde_json::from_value(json).expect("de");
        assert_eq!(args.tx, args.base);
    }

    #[test]
    fn retype_args_use_the_wire_names() {
        let args: RetypeMetadataKeyArgs =
            serde_json::from_value(serde_json::json!({ "key": "invoice", "ty": "number" }))
                .expect("de");
        assert_eq!(args.key, "invoice");
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code")]
    fn register_page_args_nest_under_request() {
        let request = crate::RegisterRequest::new(
            crate::Filter::default(),
            "acct-1".to_owned(),
            false,
            None,
            100,
        );
        let json = serde_json::to_value(RegisterPageArgs { request }).expect("ser");
        assert_eq!(json["request"]["account_id"], "acct-1");
    }
}
