//! WASM-only typed wrappers for Tauri IPC commands.
//!
//! Each function corresponds to a command `bc_service::dispatch` routes. The command
//! name strings come from [`crate::commands`] so a rename on either side is a
//! compile error, not a silent runtime mismatch. Every wrapper builds an
//! owned argument struct and routes through [`call`], which will later swap
//! its transport without touching a single wrapper.
//!
//! # Usage
//!
//! ```ignore
//! let accounts = bc_ipc::client::list_accounts().await?;
//! ```

use serde::Serialize;
use serde::de::DeserializeOwned;
#[cfg(feature = "http")]
use wasm_bindgen::JsCast as _;
#[cfg(feature = "http")]
use wasm_bindgen_futures::JsFuture;

use crate::AccountNode;
use crate::AuditEntry;
use crate::BackupInfo;
use crate::BackupSettings;
use crate::BcError;
use crate::BudgetIntent;
use crate::BudgetOverview;
use crate::BudgetRevisionView;
use crate::BudgetRowTransaction;
use crate::CommodityInfo;
use crate::EditTransaction;
use crate::Filter;
use crate::FilteredTransaction;
use crate::MetaKeyDefDto;
use crate::MetaTypeDto;
use crate::MetaValueCount;
use crate::NativePeriodRow;
use crate::NewTransaction;
use crate::PluginInfo;
use crate::QueryCatalog;
use crate::Reconciliation;
use crate::RegisterPage;
use crate::RegisterRequest;
use crate::RolloverPolicy;
use crate::SettingsInfo;
use crate::TagInfo;
use crate::Transaction;
use crate::TransferSuggestion;
use crate::commands;
use crate::commands::ArchiveBudgetArgs;
use crate::commands::ClearPostingSpreadArgs;
use crate::commands::CreateBudgetArgs;
use crate::commands::CreateCurrencyArgs;
use crate::commands::CreateTagArgs;
use crate::commands::CreateTransactionArgs;
use crate::commands::DeleteBackupArgs;
use crate::commands::DeleteCurrencyArgs;
use crate::commands::DeleteTransactionArgs;
use crate::commands::EditTransactionArgs;
use crate::commands::GetAccountSparklineArgs;
use crate::commands::GetAccountStatsArgs;
use crate::commands::GetBudgetOverviewArgs;
use crate::commands::GetBudgetRowTransactionsArgs;
use crate::commands::GetNativePeriodsArgs;
use crate::commands::GetTransactionArgs;
use crate::commands::GetTransactionAuditArgs;
use crate::commands::ListBudgetRevisionsArgs;
use crate::commands::ListRejectedSourcesArgs;
use crate::commands::ListTransactionsArgs;
use crate::commands::MergeTransactionsArgs;
use crate::commands::RegisterPageArgs;
use crate::commands::ReleaseRejectedSourcesArgs;
use crate::commands::RemoveBudgetRevisionArgs;
use crate::commands::ResolveEffectiveDateArgs;
use crate::commands::RestoreDatabaseArgs;
use crate::commands::ReverseTransactionArgs;
use crate::commands::ReviseBudgetArgs;
use crate::commands::SearchTransactionsArgs;
use crate::commands::SetPostingSpreadArgs;
use crate::commands::SetReconciliationArgs;
use crate::commands::TransactionProvenanceArgs;
use crate::commands::UnmergeTransactionArgs;
use crate::commands::UpdateBackupSettingsArgs;
use crate::commands::UpdateCurrencyArgs;

/// Empty args struct for commands that take no parameters.
///
/// Tauri 2 IPC requires args to serialise as a JSON Object (`{}`), not `null`.
/// A unit struct (`struct NoArgs;`) serialises to `null`; an empty record
/// struct (`struct NoArgs {}`) serialises to `{}` as required.
#[derive(Serialize)]
#[expect(
    clippy::empty_structs_with_brackets,
    reason = "empty braces are intentional: serde serialises `struct S {}` as `{}` but `struct S;` as `null`"
)]
struct NoArgs {}

/// Envelope for the desktop host's single `rpc` command.
#[cfg(not(feature = "http"))]
#[derive(Serialize)]
struct RpcArgs<'a, A> {
    /// Command name.
    cmd: &'a str,
    /// The command's argument object.
    args: &'a A,
}

/// Sends `cmd` with `args` to the backend and decodes the reply.
///
/// The desktop host exposes one Tauri command, `rpc`, which routes `cmd`
/// to its implementation.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the underlying transport call fails.
#[cfg(not(feature = "http"))]
async fn call<T, A>(cmd: &str, args: &A) -> Result<T, BcError>
where
    T: DeserializeOwned + 'static,
    A: Serialize,
{
    tauri_sys::core::invoke_result::<T, BcError>("rpc", RpcArgs { cmd, args }).await
}

/// Sends `cmd` with `args` to `bc-server` over HTTP and decodes the reply.
///
/// Posts `args` as the JSON body to `/rpc/{cmd}`, matching the desktop
/// transport's single-endpoint shape.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the request cannot be built, the fetch
/// fails (network down, server restarting) or the response body does not
/// decode; otherwise returns the server's [`BcError`].
#[cfg(feature = "http")]
async fn call<T, A>(cmd: &str, args: &A) -> Result<T, BcError>
where
    T: DeserializeOwned,
    A: Serialize,
{
    let internal = |what: &str, e: wasm_bindgen::JsValue| {
        BcError::Internal(format!(
            "{what}: {}",
            e.as_string().unwrap_or_else(|| format!("{e:?}"))
        ))
    };
    let body = serde_json::to_string(args).map_err(|e| BcError::Internal(e.to_string()))?;
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    init.set_body(&wasm_bindgen::JsValue::from_str(&body));
    let request = web_sys::Request::new_with_str_and_init(&format!("/rpc/{cmd}"), &init)
        .map_err(|e| internal("request", e))?;
    request
        .headers()
        .set("Content-Type", "application/json")
        .map_err(|e| internal("header", e))?;
    let window = web_sys::window().ok_or_else(|| BcError::Internal("no window".to_owned()))?;
    let response: web_sys::Response = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| internal("network error", e))?
        .unchecked_into();
    let text = JsFuture::from(response.text().map_err(|e| internal("body", e))?)
        .await
        .map_err(|e| internal("body", e))?
        .as_string()
        .unwrap_or_default();
    crate::transport::decode(response.status(), &text)
}

/// Lists all active accounts from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_accounts() -> Result<Vec<AccountNode>, BcError> {
    call(commands::LIST_ACCOUNTS, &NoArgs {}).await
}

/// Lists transactions for `account_id` within `[date_from, date_until)` from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_transactions(
    account_id: &str,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
) -> Result<Vec<Transaction>, BcError> {
    call(
        commands::LIST_TRANSACTIONS,
        &ListTransactionsArgs {
            account_id: account_id.to_owned(),
            date_from,
            date_until,
        },
    )
    .await
}

/// Creates a new transaction and returns its assigned ID string.
///
/// # Errors
///
/// Returns a [`BcError`] from the backend if validation fails, or
/// [`BcError::Internal`] if the Tauri invoke itself fails.
#[inline]
pub async fn create_transaction(tx: &NewTransaction) -> Result<String, BcError> {
    call(
        commands::CREATE_TRANSACTION,
        &CreateTransactionArgs { tx: tx.clone() },
    )
    .await
}

/// Lists all tags from the backend as id/path pairs.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_tags() -> Result<Vec<TagInfo>, BcError> {
    call(commands::LIST_TAGS, &NoArgs {}).await
}

/// Lists every registered metadata key with its type.
///
/// The response is a whole-registry snapshot. A key enters the registry on the
/// first write of a value under it, so an ordinary transaction save adds keys
/// too — a caller holding this snapshot for the session goes stale as soon as
/// the user names a key the registry has not seen.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_metadata_keys() -> Result<Vec<MetaKeyDefDto>, BcError> {
    call(commands::LIST_METADATA_KEYS, &NoArgs {}).await
}

/// Changes a metadata key's registered type, returning the type it had before.
///
/// Every stored value under the key is re-asserted against the new type:
/// widening to `text` is a relabel, and narrowing flags whatever will not
/// parse.
///
/// # Errors
///
/// Returns [`BcError::Validation`] for a key that breaks the charset, leading
/// character or length rules, [`BcError::NotFound`] for a key that is not
/// registered, or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn retype_metadata_key(key: &str, ty: MetaTypeDto) -> Result<MetaTypeDto, BcError> {
    call(
        commands::RETYPE_METADATA_KEY,
        &commands::RetypeMetadataKeyArgs {
            key: key.to_owned(),
            ty,
        },
    )
    .await
}

/// Renames a metadata key, carrying its entries with it.
///
/// # Errors
///
/// Returns [`BcError::Validation`] for a malformed name or a target that is
/// already registered, [`BcError::NotFound`] when the source key is not
/// registered, or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn rename_metadata_key(from: &str, to: &str) -> Result<(), BcError> {
    call(
        commands::RENAME_METADATA_KEY,
        &commands::RenameMetadataKeyArgs {
            from: from.to_owned(),
            to: to.to_owned(),
        },
    )
    .await
}

/// Fetches every fact query text resolves against, so the palette resolves
/// exactly as the server does.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails or the catalog cannot be
/// read.
#[inline]
pub async fn query_catalog() -> Result<QueryCatalog, BcError> {
    call(commands::QUERY_CATALOG, &NoArgs {}).await
}

/// Fetches the stored values of the text key `key` that contain `needle`,
/// ignoring ASCII case: prefix matches first, then the most used.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails or the values cannot be
/// read.
#[inline]
pub async fn metadata_values(
    key: &str,
    needle: &str,
    limit: u32,
) -> Result<Vec<MetaValueCount>, BcError> {
    call(
        commands::METADATA_VALUES,
        &commands::MetadataValuesArgs {
            key: key.to_owned(),
            needle: needle.to_owned(),
            limit,
        },
    )
    .await
}

/// Lists registered commodities/currencies.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn list_currencies() -> Result<Vec<CommodityInfo>, BcError> {
    call(commands::LIST_CURRENCIES, &NoArgs {}).await
}

/// Creates a new commodity, returning the stored value.
///
/// # Errors
///
/// Returns [`BcError::Validation`] on a marker conflict, or [`BcError::Internal`]
/// if the invoke fails.
#[inline]
pub async fn create_currency(info: &CommodityInfo) -> Result<CommodityInfo, BcError> {
    call(
        commands::CREATE_CURRENCY,
        &CreateCurrencyArgs { info: info.clone() },
    )
    .await
}

/// Updates an existing commodity (its code is immutable).
///
/// # Errors
///
/// Returns [`BcError::Validation`] on a marker conflict or code change, or
/// [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn update_currency(info: &CommodityInfo) -> Result<(), BcError> {
    call(
        commands::UPDATE_CURRENCY,
        &UpdateCurrencyArgs { info: info.clone() },
    )
    .await
}

/// Deletes a commodity, refusing if it is still referenced.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if the commodity is referenced, or
/// [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn delete_currency(id: &str) -> Result<(), BcError> {
    call(
        commands::DELETE_CURRENCY,
        &DeleteCurrencyArgs { id: id.to_owned() },
    )
    .await
}

/// Creates the full colon-path tag hierarchy, returning the leaf tag ID string.
///
/// Existing ancestors are reused; only missing segments are created.
///
/// # Errors
///
/// Returns a [`BcError`] from the backend if the path is invalid, or
/// [`BcError::Internal`] if the Tauri invoke itself fails.
#[inline]
pub async fn create_tag(path: &str) -> Result<String, BcError> {
    call(
        commands::CREATE_TAG,
        &CreateTagArgs {
            path: path.to_owned(),
        },
    )
    .await
}

/// Gets windowed inflow, outflow, internal and balance stats for `account_id`.
///
/// When `filter` is `Some`, the stats are recomputed against it and the real
/// (unfiltered) opening/closing are attached for reference.
///
/// # Arguments
///
/// * `account_id` - Account to query.
/// * `commodity` - Commodity code override; `None` lets the backend pick the account's default.
/// * `include_descendants` - Fold the account's subtree into the result.
/// * `date_from` - Inclusive window start.
/// * `date_until` - Exclusive window end.
/// * `filter` - Active global filter, or `None` for the unfiltered fast path.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_stats(
    account_id: &str,
    commodity: Option<&str>,
    include_descendants: bool,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
    filter: Option<&crate::Filter>,
) -> Result<crate::AccountStats, BcError> {
    call(
        commands::GET_ACCOUNT_STATS,
        &GetAccountStatsArgs {
            account_id: account_id.to_owned(),
            commodity: commodity.map(ToOwned::to_owned),
            include_descendants,
            date_from,
            date_until,
            filter: filter.cloned(),
        },
    )
    .await
}

/// Gets period-bucketed cash-flow data for a sparkline.
///
/// # Arguments
///
/// * `account_id` - Account ID to query.
/// * `commodity` - Commodity code override; `None` lets the backend pick the account's default.
/// * `include_descendants` - Fold the account's subtree into the result.
/// * `period` - Time-bucket granularity.
/// * `count` - Number of buckets to return.
/// * `as_of` - Reference date; the most recent bucket contains this date.
/// * `filter` - Active global filter, or `None` for the unfiltered fast path.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_account_sparkline(
    account_id: &str,
    commodity: Option<&str>,
    include_descendants: bool,
    period: crate::Period,
    count: u32,
    as_of: jiff::civil::Date,
    filter: Option<&crate::Filter>,
) -> Result<Vec<crate::SparkPoint>, BcError> {
    call(
        commands::GET_ACCOUNT_SPARKLINE,
        &GetAccountSparklineArgs {
            account_id: account_id.to_owned(),
            commodity: commodity.map(ToOwned::to_owned),
            include_descendants,
            count: Some(count),
            period: Some(period),
            as_of: Some(as_of),
            filter: filter.cloned(),
        },
    )
    .await
}

/// Lists all installed plugins from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn list_plugins() -> Result<Vec<PluginInfo>, BcError> {
    call(commands::LIST_PLUGINS, &NoArgs {}).await
}

/// Gets the current application settings from the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails or the config
/// cannot be loaded.
#[inline]
pub async fn get_settings() -> Result<SettingsInfo, BcError> {
    call(commands::GET_SETTINGS, &NoArgs {}).await
}

/// Gets the budget overview (summary, tree and pace) for a display window.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_budget_overview(
    period_type: crate::Period,
    period_start: jiff::civil::Date,
    filter: Option<&Filter>,
) -> Result<BudgetOverview, BcError> {
    call(
        commands::GET_BUDGET_OVERVIEW,
        &GetBudgetOverviewArgs {
            period_type,
            period_start,
            filter: filter.cloned(),
        },
    )
    .await
}

/// Gets native period sub-rows for one budget in a display window.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_native_periods(
    budget_id: &str,
    display_start: jiff::civil::Date,
    display_end: jiff::civil::Date,
    filter: Option<&Filter>,
) -> Result<Vec<NativePeriodRow>, BcError> {
    call(
        commands::GET_NATIVE_PERIODS,
        &GetNativePeriodsArgs {
            budget_id: budget_id.to_owned(),
            display_start,
            display_end,
            filter: filter.cloned(),
        },
    )
    .await
}

/// Gets the transactions behind one budget tree row, each tagged with the
/// bucket its posting landed in.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn get_budget_row_transactions(
    row_id: &str,
    period_type: crate::Period,
    period_start: jiff::civil::Date,
    filter: Option<&Filter>,
) -> Result<Vec<BudgetRowTransaction>, BcError> {
    call(
        commands::GET_BUDGET_ROW_TRANSACTIONS,
        &GetBudgetRowTransactionsArgs {
            row_id: row_id.to_owned(),
            period_type,
            period_start,
            filter: filter.cloned(),
        },
    )
    .await
}

/// Archives a budget.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn archive_budget(budget_id: &str) -> Result<(), BcError> {
    call(
        commands::ARCHIVE_BUDGET,
        &ArchiveBudgetArgs {
            budget_id: budget_id.to_owned(),
        },
    )
    .await
}

/// Creates a new budget.
///
/// `target` is the text as typed; the backend evaluates an expression and
/// stores its source. `intent: None` uses the account type's default.
///
/// # Returns
///
/// The write's warnings, rendered for display.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
#[expect(
    clippy::too_many_arguments,
    reason = "budget creation requires all revision fields; a builder struct would add more boilerplate than clarity here"
)]
pub async fn create_budget(
    account_id: &str,
    effective_from: jiff::civil::Date,
    name: Option<&str>,
    target: Option<&str>,
    target_currency: Option<&str>,
    intent: Option<BudgetIntent>,
    period: crate::Period,
    rollover: RolloverPolicy,
    tag_filter: Option<&TagInfo>,
) -> Result<Vec<String>, BcError> {
    call(
        commands::CREATE_BUDGET,
        &CreateBudgetArgs {
            account_id: account_id.to_owned(),
            effective_from,
            name: name.map(ToOwned::to_owned),
            target: target.map(ToOwned::to_owned),
            target_currency: target_currency.map(ToOwned::to_owned),
            intent,
            period,
            rollover,
            tag_filter: tag_filter.cloned(),
        },
    )
    .await
}

/// Sets the accrual spread on a posting.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn set_posting_spread(
    posting_id: &str,
    spread_from: jiff::civil::Date,
    spread_until: jiff::civil::Date,
) -> Result<(), BcError> {
    call(
        commands::SET_POSTING_SPREAD,
        &SetPostingSpreadArgs {
            posting_id: posting_id.to_owned(),
            spread_from,
            spread_until,
        },
    )
    .await
}

/// Clears the accrual spread from a posting.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn clear_posting_spread(posting_id: &str) -> Result<(), BcError> {
    call(
        commands::CLEAR_POSTING_SPREAD,
        &ClearPostingSpreadArgs {
            posting_id: posting_id.to_owned(),
        },
    )
    .await
}

/// Lists a budget's revisions for a display window.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn list_budget_revisions(
    budget_id: &str,
    display_start: jiff::civil::Date,
    display_end: jiff::civil::Date,
) -> Result<Vec<BudgetRevisionView>, BcError> {
    call(
        commands::LIST_BUDGET_REVISIONS,
        &ListBudgetRevisionsArgs {
            budget_id: budget_id.to_owned(),
            display_start,
            display_end,
        },
    )
    .await
}

/// Resolves a snap effective date to the next grid boundary.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn resolve_effective_date(
    budget_id: &str,
    date: jiff::civil::Date,
    exclude_revision_id: Option<&str>,
) -> Result<jiff::civil::Date, BcError> {
    call(
        commands::RESOLVE_EFFECTIVE_DATE,
        &ResolveEffectiveDateArgs {
            budget_id: budget_id.to_owned(),
            date,
            exclude_revision_id: exclude_revision_id.map(ToOwned::to_owned),
        },
    )
    .await
}

/// Adds or amends a budget revision.
///
/// `target` is the text as typed; the backend evaluates an expression and
/// stores its source. `intent: None` keeps the amended revision's intent, or
/// uses the account type's default for a new revision.
///
/// # Returns
///
/// The write's warnings, rendered for display.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[expect(
    clippy::too_many_arguments,
    reason = "IPC wrapper mirrors the Tauri command's flat argument list"
)]
#[inline]
pub async fn revise_budget(
    budget_id: &str,
    revision_id: Option<&str>,
    effective_from: jiff::civil::Date,
    name: Option<&str>,
    target: Option<&str>,
    target_currency: Option<&str>,
    intent: Option<BudgetIntent>,
    rollover: RolloverPolicy,
    period: crate::Period,
    tag_filter: Option<&TagInfo>,
) -> Result<Vec<String>, BcError> {
    call(
        commands::REVISE_BUDGET,
        &ReviseBudgetArgs {
            budget_id: budget_id.to_owned(),
            revision_id: revision_id.map(ToOwned::to_owned),
            effective_from,
            name: name.map(ToOwned::to_owned),
            target: target.map(ToOwned::to_owned),
            target_currency: target_currency.map(ToOwned::to_owned),
            intent,
            rollover,
            period,
            tag_filter: tag_filter.cloned(),
        },
    )
    .await
}

/// Removes a budget revision.
///
/// # Errors
///
/// Returns [`BcError`] if the backend call fails.
#[inline]
pub async fn remove_budget_revision(budget_id: &str, revision_id: &str) -> Result<(), BcError> {
    call(
        commands::REMOVE_BUDGET_REVISION,
        &RemoveBudgetRevisionArgs {
            budget_id: budget_id.to_owned(),
            revision_id: revision_id.to_owned(),
        },
    )
    .await
}

/// Reverses a transaction by id, returning the new reversal transaction's id.
///
/// # Errors
///
/// Returns [`BcError`] if the id is invalid or the transaction does not exist.
#[inline]
pub async fn reverse_transaction(id: &str) -> Result<String, BcError> {
    call(
        commands::REVERSE_TRANSACTION,
        &ReverseTransactionArgs { id: id.to_owned() },
    )
    .await
}

/// Deletes a transaction, keeping or forgetting its import provenance.
///
/// # Errors
///
/// Returns [`BcError`] if the id is invalid or the transaction does not exist.
#[inline]
pub async fn delete_transaction(
    id: &str,
    forget_provenance: bool,
) -> Result<crate::DeleteOutcome, BcError> {
    call(
        commands::DELETE_TRANSACTION,
        &DeleteTransactionArgs {
            id: id.to_owned(),
            forget_provenance,
        },
    )
    .await
}

/// Summarises the import provenance a transaction carries.
///
/// # Errors
///
/// Returns [`BcError`] if the id is invalid or the transaction does not exist.
#[inline]
pub async fn transaction_provenance(id: &str) -> Result<crate::TransactionProvenance, BcError> {
    call(
        commands::TRANSACTION_PROVENANCE,
        &TransactionProvenanceArgs { id: id.to_owned() },
    )
    .await
}

/// Lists rejected statement rows, optionally for one account id.
///
/// # Errors
///
/// Returns [`BcError`] if the account id is invalid or the invoke fails.
#[inline]
pub async fn list_rejected_sources(
    account: Option<&str>,
) -> Result<Vec<crate::RejectedRow>, BcError> {
    call(
        commands::LIST_REJECTED_SOURCES,
        &ListRejectedSourcesArgs {
            account: account.map(str::to_owned),
        },
    )
    .await
}

/// Releases rejected statement rows, returning how many references were released.
///
/// # Errors
///
/// Returns [`BcError`] if a target is malformed, unknown or still live.
#[inline]
pub async fn release_rejected_sources(targets: &[String]) -> Result<u64, BcError> {
    call(
        commands::RELEASE_REJECTED_SOURCES,
        &ReleaseRejectedSourcesArgs {
            targets: targets.to_vec(),
        },
    )
    .await
}

/// Applies a desired transaction state via the backend edit command.
///
/// # Arguments
///
/// * `tx` - The desired transaction state.
/// * `base` - The transaction as the editor loaded it.
///
/// # Errors
///
/// Returns [`BcError`] if the backend rejects or fails the edit, including
/// [`BcError::Conflict`] if the transaction changed since `base` was loaded.
#[inline]
pub async fn edit_transaction(tx: &EditTransaction, base: &EditTransaction) -> Result<(), BcError> {
    call(
        commands::EDIT_TRANSACTION,
        &EditTransactionArgs {
            tx: tx.clone(),
            base: base.clone(),
        },
    )
    .await
}

/// Loads one transaction by ID.
///
/// # Errors
///
/// Returns [`BcError::Validation`] for a malformed ID, [`BcError::NotFound`]
/// if no transaction has that ID, or [`BcError::Internal`] if the invoke
/// fails.
#[inline]
pub async fn get_transaction(id: &str) -> Result<Transaction, BcError> {
    call(
        commands::GET_TRANSACTION,
        &GetTransactionArgs { id: id.to_owned() },
    )
    .await
}

/// Sets a transaction's reconciliation state.
///
/// # Arguments
///
/// * `id` - The transaction ID.
/// * `state` - The desired reconciliation state.
///
/// # Errors
///
/// Returns [`BcError`] if the backend rejects (e.g. reconciling an
/// unbalanced transaction) or fails the update.
#[inline]
pub async fn set_reconciliation(id: &str, state: Reconciliation) -> Result<(), BcError> {
    call(
        commands::SET_RECONCILIATION,
        &SetReconciliationArgs {
            id: id.to_owned(),
            reconciliation: state,
        },
    )
    .await
}

/// Loads the audit trail for a transaction.
///
/// # Arguments
///
/// * `id` - The transaction's ID.
///
/// # Errors
///
/// Returns [`BcError`] if the backend lookup fails.
#[inline]
pub async fn get_transaction_audit(id: &str) -> Result<Vec<AuditEntry>, BcError> {
    call(
        commands::GET_TRANSACTION_AUDIT,
        &GetTransactionAuditArgs { id: id.to_owned() },
    )
    .await
}

/// Snapshots the database to the managed backup directory.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn backup_database() -> Result<BackupInfo, BcError> {
    call(commands::BACKUP_DATABASE, &NoArgs {}).await
}

/// Lists existing backups, newest-first.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn list_backups() -> Result<Vec<BackupInfo>, BcError> {
    call(commands::LIST_BACKUPS, &NoArgs {}).await
}

/// Restores the database from `path`; the backend relaunches the app on success.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if the file is not a valid backup, or
/// [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn restore_database(path: &str) -> Result<(), BcError> {
    call(
        commands::RESTORE_DATABASE,
        &RestoreDatabaseArgs {
            path: path.to_owned(),
        },
    )
    .await
}

/// Deletes one backup from the open ledger's pool.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if `file_name` is not a backup in the pool,
/// or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn delete_backup(file_name: &str) -> Result<(), BcError> {
    call(
        commands::DELETE_BACKUP,
        &DeleteBackupArgs {
            file_name: file_name.to_owned(),
        },
    )
    .await
}

/// Reads the current backup settings.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn get_backup_settings() -> Result<BackupSettings, BcError> {
    call(commands::GET_BACKUP_SETTINGS, &NoArgs {}).await
}

/// Persists updated backup settings to the config file.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn update_backup_settings(settings: &BackupSettings) -> Result<(), BcError> {
    call(
        commands::UPDATE_BACKUP_SETTINGS,
        &UpdateBackupSettingsArgs {
            settings: settings.clone(),
        },
    )
    .await
}

/// Proposes candidate transfer pairs for review.
///
/// # Errors
///
/// Returns [`BcError`] if the backend query fails.
#[inline]
pub async fn suggest_transfers() -> Result<Vec<TransferSuggestion>, BcError> {
    call(commands::SUGGEST_TRANSFERS, &NoArgs {}).await
}

/// Merges `absorbed` into `survivor` (survivor is the debit leg).
///
/// # Errors
///
/// Returns [`BcError::Validation`] if an id is invalid or the pair is not
/// mergeable, or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn merge_transactions(survivor: &str, absorbed: &str) -> Result<(), BcError> {
    call(
        commands::MERGE_TRANSACTIONS,
        &MergeTransactionsArgs {
            survivor: survivor.to_owned(),
            absorbed: absorbed.to_owned(),
        },
    )
    .await
}

/// Reverses the most recent merge on `transaction`, returning the restored id.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if the id is invalid or there is no merge to
/// reverse, or [`BcError::Internal`] if the invoke fails.
#[inline]
pub async fn unmerge_transaction(transaction: &str) -> Result<String, BcError> {
    call(
        commands::UNMERGE_TRANSACTION,
        &UnmergeTransactionArgs {
            transaction: transaction.to_owned(),
        },
    )
    .await
}

/// Runs a structured transaction search on the backend.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn search_transactions(filter: &Filter) -> Result<Vec<FilteredTransaction>, BcError> {
    call(
        commands::SEARCH_TRANSACTIONS,
        &SearchTransactionsArgs {
            filter: filter.clone(),
        },
    )
    .await
}

/// Fetches one page of the register for an account, with running balances.
///
/// # Arguments
///
/// * `request` - Filter, scope, cursor and limit.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the Tauri invoke fails.
#[inline]
pub async fn register_page(request: &RegisterRequest) -> Result<RegisterPage, BcError> {
    call(
        commands::REGISTER_PAGE,
        &RegisterPageArgs {
            request: request.clone(),
        },
    )
    .await
}
