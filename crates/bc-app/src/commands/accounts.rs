//! Tauri command handlers for account and transaction operations.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use bc_core::ipc::AccountNodeExt as _;
use bc_core::ipc::AuditEntryExt as _;
use bc_core::ipc::TransactionExt as _;
use tauri::State;

use crate::AppState;

// MARK: Scope helper

/// Resolves the account set a command operates on.
///
/// # Arguments
///
/// * `state` - Tauri managed application state.
/// * `id` - The selected account.
/// * `include_descendants` - When set, the account plus its active subtree.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the subtree lookup fails.
async fn scope_ids(
    state: &AppState,
    id: &bc_models::AccountId,
    include_descendants: bool,
) -> Result<Vec<bc_models::AccountId>, bc_ipc::BcError> {
    if !include_descendants {
        return Ok(vec![id.clone()]);
    }
    state
        .accounts
        .subtree_ids(id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

// MARK: Command handlers

/// List all accounts as a tree of nodes.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::AccountNode>, bc_ipc::BcError> {
    let accounts = state
        .accounts
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let balances = state
        .balance_engine
        .default_balances()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let rollups = state
        .balance_engine
        .rollup_balances()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let nodes = accounts
        .iter()
        .map(|account| {
            let balance = balances.get(account.id()).map(bc_ipc::Amount::from);
            let default_code = balance.as_ref().map(|b| b.currency_code.clone());
            let rollup = rollups
                .get(account.id())
                .map(|b| ordered_amounts(b, default_code.as_deref()))
                .unwrap_or_default();
            bc_ipc::AccountNode::from_model(account, &forest, balance).with_rollup(rollup)
        })
        .collect::<Vec<_>>();

    Ok(nodes)
}

/// Flattens `balances` into IPC amounts, moving `default_code` to the front.
///
/// Without a default commodity (e.g. a root with no own postings, only a
/// roll-up), the remainder is sorted by commodity code rather than left in
/// first-seen order: a root's own `balance` is `None` in that case, so there
/// is no default to anchor on, and first-seen order over a `HashMap`-derived
/// [`bc_models::Balances`] is not guaranteed stable across fetches — a
/// sorted tiebreak keeps the sidebar figure and sparkline currency from
/// flipping between calls.
///
/// # Arguments
///
/// * `balances` - Per-commodity totals in first-seen order.
/// * `default_code` - The account's own default commodity, if any.
fn ordered_amounts(
    balances: &bc_models::Balances,
    default_code: Option<&str>,
) -> Vec<bc_ipc::Amount> {
    let mut out: Vec<bc_ipc::Amount> = balances
        .iter()
        .map(|(code, value)| bc_ipc::Amount::new(value, code))
        .collect();
    if let Some(code) = default_code {
        if let Some(pos) = out.iter().position(|a| a.currency_code == code)
            && pos != 0
        {
            let first = out.remove(pos);
            out.insert(0, first);
        }
    } else {
        out.sort_unstable_by(|a, b| a.currency_code.cmp(&b.currency_code));
    }
    out
}

/// List transactions for the given account within a date window.
///
/// # Arguments
///
/// * `account_id` - The account ID to filter by.
/// * `date_from` - The inclusive start of the date window.
/// * `date_until` - The exclusive end of the date window.
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails or the ID is invalid.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_transactions(
    account_id: String,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::Transaction>, bc_ipc::BcError> {
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let accounts = state
        .accounts
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let account_map = accounts
        .iter()
        .map(|a| (a.id().to_string(), a))
        .collect::<std::collections::HashMap<_, _>>();

    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let txs = state
        .transactions
        .list_for_account_in_range(&id, date_from, date_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(txs
        .map(|tx| bc_ipc::Transaction::from_model_with_accounts(&tx, &account_map, &forest))
        .collect())
}

/// Create a new transaction.
///
/// # Arguments
///
/// * `tx` - The new transaction data.
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the service call fails, a field fails
/// validation, or an account ID cannot be parsed.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn create_transaction(
    tx: bc_ipc::NewTransaction,
    state: State<'_, AppState>,
) -> Result<String, bc_ipc::BcError> {
    let reconciliation = bc_models::Reconciliation::from(tx.reconciliation);

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p.account_id.parse::<bc_models::AccountId>().map_err(|e| {
            bc_ipc::BcError::Validation(format!("invalid account_id '{}': {e}", p.account_id))
        })?;
        let tag_ids = resolve_tag_inputs(&state.tags, &p.tags).await?;
        let posting = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id)
            .maybe_amount(p.amount.as_ref().map(bc_models::Amount::from))
            .maybe_price(p.price.as_ref().map(bc_models::Quote::from))
            .maybe_cost(p.cost.as_ref().map(bc_models::Cost::from))
            .metadata(metadata_from(&p.metadata)?)
            .tag_ids(tag_ids)
            .maybe_spread_from(p.spread_from)
            .maybe_spread_until(p.spread_until)
            .build();
        postings.push(posting);
    }

    let tx_tag_ids = resolve_tag_inputs(&state.tags, &tx.tags).await?;

    let model_tx = bc_models::Transaction::builder()
        .id(bc_models::TransactionId::new())
        .date(tx.date)
        .description(tx.description)
        .metadata(metadata_from(&tx.metadata)?)
        .postings(postings)
        .tag_ids(tx_tag_ids)
        .reconciliation(reconciliation)
        .created_at(jiff::Timestamp::now())
        .build();

    // Warnings are not yet surfaced to the UI; see the follow-up issue filed
    // from this work's out-of-scope list.
    let tx_id = state
        .transactions
        .create(model_tx)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
        .into_inner();

    Ok(tx_id.to_string())
}

/// Applies a desired transaction state (decomposed-event edit).
///
/// # Arguments
///
/// * `tx` - The desired transaction state.
/// * `state` - The shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for unparsable IDs or domain rule
/// violations, [`bc_ipc::BcError::NotFound`] if the transaction does not exist,
/// or [`bc_ipc::BcError::Internal`] for unexpected failures.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn edit_transaction(
    tx: bc_ipc::EditTransaction,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let tx_id = tx
        .id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    let reconciliation = bc_models::Reconciliation::from(tx.reconciliation);

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p
            .account_id
            .parse::<bc_models::AccountId>()
            .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account id: {e}")))?;
        let posting_id = match &p.id {
            Some(s) => s
                .parse::<bc_models::PostingId>()
                .map_err(|e| bc_ipc::BcError::Validation(format!("invalid posting id: {e}")))?,
            None => bc_models::PostingId::new(),
        };
        let tag_ids = resolve_tag_inputs(&state.tags, &p.tags).await?;
        let posting = bc_models::Posting::builder()
            .id(posting_id)
            .account_id(account_id)
            .maybe_amount(p.amount.as_ref().map(bc_models::Amount::from))
            .maybe_price(p.price.as_ref().map(bc_models::Quote::from))
            .maybe_cost(p.cost.as_ref().map(bc_models::Cost::from))
            .metadata(metadata_from(&p.metadata)?)
            .tag_ids(tag_ids)
            .maybe_spread_from(p.spread_from)
            .maybe_spread_until(p.spread_until)
            .build();
        postings.push(posting);
    }

    let tag_ids = resolve_tag_inputs(&state.tags, &tx.tags).await?;

    let model_tx = bc_models::Transaction::builder()
        .id(tx_id)
        .date(tx.date)
        .description(tx.description)
        .metadata(metadata_from(&tx.metadata)?)
        .postings(postings)
        .reconciliation(reconciliation)
        .tag_ids(tag_ids)
        .created_at(jiff::Timestamp::now())
        .build();

    // Warnings are not yet surfaced to the UI; see the follow-up issue filed
    // from this work's out-of-scope list.
    state.transactions.edit(model_tx).await?;
    Ok(())
}

/// Sets a transaction's reconciliation state.
///
/// # Arguments
///
/// * `id`             - The transaction ID to update.
/// * `reconciliation` - The desired reconciliation state.
/// * `state`          - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if `id` is malformed or the
/// transaction does not balance, [`bc_ipc::BcError::NotFound`] if no such
/// transaction exists, or [`bc_ipc::BcError::Internal`] if the update fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn set_reconciliation(
    id: String,
    reconciliation: bc_ipc::Reconciliation,
    state: State<'_, AppState>,
) -> Result<(), bc_ipc::BcError> {
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    Ok(state
        .transactions
        .reconcile(&tx_id, bc_models::Reconciliation::from(reconciliation))
        .await?)
}

/// Reverses a transaction, returning the new reversal transaction's id.
///
/// # Arguments
///
/// * `id`    - The transaction ID to reverse.
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if `id` is malformed or no such transaction exists.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn reverse_transaction(
    id: String,
    state: State<'_, AppState>,
) -> Result<String, bc_ipc::BcError> {
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid id: {e}")))?;

    let reversal_id = state
        .transactions
        .reverse(&tx_id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(reversal_id.to_string())
}

/// Returns income, expense, and balance totals for `account_id` over an
/// explicit date window.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `commodity`  - Optional commodity code override. Defaults to the account's first commodity.
/// * `include_descendants` - Fold the account's active subtree into the stats.
/// * `date_from`  - The inclusive start of the date window.
/// * `date_until` - The exclusive end of the date window.
/// * `filter`     - Active global filter, or `None` for the unfiltered fast path. When
///   `Some`, the stats are recomputed against it and the real (unfiltered)
///   opening/closing are attached for reference.
/// * `state`      - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the account ID or filter is malformed, or
/// [`bc_ipc::BcError::Internal`] if a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_account_stats(
    account_id: String,
    commodity: Option<String>,
    include_descendants: bool,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
    filter: Option<bc_ipc::Filter>,
    state: State<'_, AppState>,
) -> Result<bc_ipc::AccountStats, bc_ipc::BcError> {
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let commodity_code = match commodity {
        Some(c) => c,
        None => state
            .balance_engine
            .default_commodity_for(&id)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
            .unwrap_or_default(),
    };

    let ids = scope_ids(&state, &id, include_descendants).await?;

    // Real (unfiltered) window stats — cheap SQL, always computed so the
    // filtered branch can attach them for reference.
    let real = state
        .balance_engine
        .account_period_stats_for_set(&ids, &commodity_code, date_from, date_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let Some(active_filter) = filter else {
        return Ok(bc_ipc::AccountStats::new(
            bc_ipc::Amount::from(&real.income),
            bc_ipc::Amount::from(&real.expenses),
            bc_ipc::Amount::from(&real.net),
            bc_ipc::Amount::from(&real.opening),
            bc_ipc::Amount::from(&real.closing),
            real.tx_count,
        ));
    };

    let query = bc_core::search::TransactionQuery::try_from(active_filter)?;
    let filtered = state
        .transactions
        .filtered_period_stats(&ids, &commodity_code, &query, date_from, date_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::AccountStats::new(
        bc_ipc::Amount::from(&filtered.income),
        bc_ipc::Amount::from(&filtered.expenses),
        bc_ipc::Amount::from(&filtered.net),
        bc_ipc::Amount::from(&filtered.opening),
        bc_ipc::Amount::from(&filtered.closing),
        filtered.tx_count,
    )
    .with_real_balances(
        bc_ipc::Amount::from(&real.opening),
        bc_ipc::Amount::from(&real.closing),
    ))
}

/// Returns the most recent transaction date across the whole ledger, or `None`.
///
/// # Arguments
///
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the query fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn latest_activity(
    state: State<'_, AppState>,
) -> Result<Option<jiff::civil::Date>, bc_ipc::BcError> {
    state
        .transactions
        .latest_activity_date_all()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
}

/// Runs a structured transaction search.
///
/// # Arguments
///
/// * `filter` - The structured filter to apply.
/// * `state` - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if the filter contains malformed
/// ids, or [`bc_ipc::BcError::Internal`] if a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn search_transactions(
    filter: bc_ipc::Filter,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::FilteredTransaction>, bc_ipc::BcError> {
    let query = bc_core::search::TransactionQuery::try_from(filter)?;

    let accounts = state
        .accounts
        .list_active()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let account_map = accounts
        .iter()
        .map(|a| (a.id().to_string(), a))
        .collect::<std::collections::HashMap<_, _>>();

    let forest = state
        .tags
        .forest()
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    let matched = state.transactions.search(&query).await?;

    Ok(matched
        .into_iter()
        .map(|m| {
            bc_ipc::FilteredTransaction::new(
                bc_ipc::Transaction::from_model_with_accounts(
                    &m.transaction,
                    &account_map,
                    &forest,
                ),
                m.matched_postings.iter().map(ToString::to_string).collect(),
            )
        })
        .collect())
}

// MARK: Metadata helpers

/// Converts a list of metadata entry DTOs into a domain metadata list.
///
/// The order is the display order and is preserved. Each entry's `mismatched`
/// flag is discarded: the write path derives it against the key's registered
/// type, so an incoming entry does not get to assert it.
///
/// # Arguments
///
/// * `entries` - The entries as they arrived over IPC.
///
/// # Returns
///
/// The domain list, in input order.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if a key fails validation or an
/// account-valued entry carries an unparsable account id.
fn metadata_from(entries: &[bc_ipc::MetaEntryDto]) -> Result<bc_models::Metadata, bc_ipc::BcError> {
    entries.iter().map(bc_models::MetaEntry::try_from).collect()
}

// MARK: Tag helpers

/// Resolves a list of tag path strings to existing tag IDs.
///
/// # Arguments
///
/// * `tags` - The tag service used for resolution.
/// * `paths` - The colon-joined tag paths to resolve.
///
/// # Returns
///
/// The resolved tag IDs, in input order.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] if a path is malformed or unknown.
async fn resolve_tag_inputs(
    tags: &bc_core::TagService,
    paths: &[String],
) -> Result<Vec<bc_models::TagId>, bc_ipc::BcError> {
    let mut ids = Vec::with_capacity(paths.len());
    for raw in paths {
        let path = raw
            .parse::<bc_models::TagPath>()
            .map_err(|e| bc_ipc::BcError::Validation(format!("invalid tag '{raw}': {e}")))?;
        let id = tags
            .resolve_existing(&path)
            .await
            .map_err(|e| bc_ipc::BcError::Validation(e.to_string()))?;
        ids.push(id);
    }
    Ok(ids)
}

// MARK: Sparkline helpers

/// Formats a bucket start date as a sparkline X-axis label.
fn spark_label(start: jiff::civil::Date, period: &bc_models::Period) -> String {
    match period {
        bc_models::Period::Monthly => {
            let months = [
                "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
            ];
            #[expect(
                clippy::as_conversions,
                clippy::cast_sign_loss,
                reason = "month() returns i8 in 1–12; casting to usize is safe"
            )]
            let idx = start.month() as usize;
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "idx is 1–12; subtracting 1 gives 0–11, always in bounds"
            )]
            #[expect(
                clippy::indexing_slicing,
                reason = "idx - 1 is 0–11; array has exactly 12 elements"
            )]
            months[idx - 1].to_owned()
        }
        bc_models::Period::Weekly => {
            #[expect(
                clippy::expect_used,
                reason = "Jan 1 of any year is always a valid date"
            )]
            let jan1 = jiff::civil::Date::new(start.year(), 1, 1).expect("Jan 1 is always valid");
            #[expect(
                clippy::arithmetic_side_effects,
                clippy::integer_division,
                clippy::integer_division_remainder_used,
                reason = "approximate week from day-of-year; day count is bounded [0, 365]"
            )]
            let week = i64::from((start - jan1).get_days()) / 7 + 1;
            format!("w{week:02}")
        }
        bc_models::Period::Quarterly | bc_models::Period::FinancialQuarter { .. } => {
            // Map month to Q1–Q4 (calendar quarters)
            #[expect(
                clippy::as_conversions,
                clippy::cast_sign_loss,
                reason = "month() returns i8 in 1–12; casting to u8 is safe"
            )]
            let month_u8 = start.month() as u8;
            #[expect(
                clippy::arithmetic_side_effects,
                clippy::integer_division,
                clippy::integer_division_remainder_used,
                reason = "month_u8 is 1–12; arithmetic maps to quarter 1–4 without overflow"
            )]
            let q = (month_u8 - 1) / 3 + 1;
            format!("Q{q}")
        }
        bc_models::Period::CalendarYear => format!("{}", start.year()),
        bc_models::Period::FinancialYear { .. } => {
            #[expect(
                clippy::integer_division_remainder_used,
                clippy::modulo_arithmetic,
                reason = "year % 100 gives 2-digit year; i16 year is always positive for realistic dates"
            )]
            let two_digit = start.year() % 100;
            format!("FY{two_digit:02}")
        }
        bc_models::Period::Daily
        | bc_models::Period::Fortnightly { .. }
        | bc_models::Period::Custom { .. } => start.to_string(),
        _ => {
            tracing::warn!(period = ?period, "unknown Period variant in spark_label; using start date as label");
            start.to_string()
        }
    }
}

// MARK: Audit helpers

/// Loads the audit trail for a transaction.
///
/// # Arguments
///
/// * `id` - The transaction's ID.
/// * `state` - The shared application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Validation`] for an unparsable ID, or
/// [`bc_ipc::BcError::Internal`] if the lookup fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_transaction_audit(
    id: String,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::AuditEntry>, bc_ipc::BcError> {
    let tx_id = id
        .parse::<bc_models::TransactionId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid transaction id: {e}")))?;
    let trail = state
        .transactions
        .audit_trail(&tx_id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "bc_core::Event is #[non_exhaustive]; catch-all arm required for exhaustiveness against future variants"
    )]
    let source_account_ids: std::collections::HashSet<_> = trail
        .iter()
        .filter_map(|(_, event)| match event {
            bc_core::Event::TransactionSourceAttached { account_id, .. } => {
                Some(account_id.clone())
            }
            _ => None,
        })
        .collect();

    let mut account_names = std::collections::HashMap::new();
    for account_id in source_account_ids.into_iter().collect::<Vec<_>>() {
        let account = state
            .accounts
            .find_by_id(&account_id)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
        account_names.insert(account_id, account.name().to_owned());
    }

    Ok(trail
        .iter()
        .map(|(ts, event)| bc_ipc::AuditEntry::from_event(*ts, event, &account_names))
        .collect())
}

/// Returns period-bucketed cash-flow data for a sparkline chart.
///
/// Defaults to 6 monthly buckets ending with the current month.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `commodity`  - Optional commodity code. Defaults to the account's first commodity.
/// * `include_descendants` - Fold the account's active subtree into the buckets.
/// * `count`      - Optional bucket count (default 6).
/// * `period`     - Optional bucket period (default Monthly).
/// * `as_of`      - Optional reference date; the most recent bucket contains this
///   date. Defaults to today.
/// * `filter`     - Active global filter, or `None` for the unfiltered fast path.
/// * `state`      - Tauri managed application state.
///
/// # Panics
///
/// Never panics in practice — the internal `NonZeroUsize::new(6)` is a
/// compile-time constant and can never be zero.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the account ID or filter is invalid, or a
/// service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[expect(
    clippy::too_many_arguments,
    reason = "Tauri command parameters mirror the IPC contract's flat args struct one-for-one"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_account_sparkline(
    account_id: String,
    commodity: Option<String>,
    include_descendants: bool,
    count: Option<u32>,
    period: Option<bc_ipc::Period>,
    as_of: Option<jiff::civil::Date>,
    filter: Option<bc_ipc::Filter>,
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::SparkPoint>, bc_ipc::BcError> {
    use core::num::NonZeroUsize;

    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;

    let commodity_code = match commodity {
        Some(c) => c,
        None => state
            .balance_engine
            .default_commodity_for(&id)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
            .unwrap_or_default(),
    };

    let ids = scope_ids(&state, &id, include_descendants).await?;

    let bucket_count = count
        .and_then(|c| NonZeroUsize::new(usize::try_from(c).unwrap_or(0)))
        .unwrap_or_else(|| {
            #[expect(
                clippy::expect_used,
                reason = "6 is a non-zero constant; this can never panic"
            )]
            NonZeroUsize::new(6).expect("6 > 0")
        });

    let model_period = period.map_or(bc_models::Period::Monthly, bc_models::Period::from);

    let anchor = as_of.unwrap_or_else(|| jiff::Zoned::now().date());

    let buckets = match filter {
        None => state
            .balance_engine
            .posting_buckets_for_set(&ids, &commodity_code, &model_period, bucket_count, anchor)
            .await
            .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?,
        Some(active_filter) => {
            let query = bc_core::search::TransactionQuery::try_from(active_filter)?;
            state
                .transactions
                .filtered_posting_buckets(
                    &ids,
                    &commodity_code,
                    &query,
                    &model_period,
                    bucket_count,
                    anchor,
                )
                .await
                .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?
        }
    };

    let points = buckets
        .into_iter()
        .map(|b| {
            bc_ipc::SparkPoint::new(
                spark_label(b.start, &model_period),
                bc_ipc::Amount::from(&b.inflow),
                bc_ipc::Amount::from(&b.outflow),
            )
        })
        .collect::<Vec<_>>();

    Ok(points)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn spark_label_names_a_daily_bucket_by_its_date() {
        let start = jiff::civil::date(2026, 2, 28);
        assert_eq!(
            super::spark_label(start, &bc_models::Period::Daily),
            "2026-02-28"
        );
        assert_eq!(super::spark_label(start, &bc_models::Period::Weekly), "w09");
    }

    /// Builds a [`bc_models::Balances`] from `(commodity, amount)` pairs, added
    /// in order so the first pair is first-seen.
    fn balances_of(pairs: &[(&str, &str)]) -> bc_models::Balances {
        let mut balances = bc_models::Balances::new();
        for (code, value) in pairs {
            let amount = bc_models::Amount::new(
                value
                    .parse()
                    .expect("test amount literal is a valid decimal"),
                *code,
            );
            balances
                .try_add(&amount)
                .expect("test amounts do not overflow");
        }
        balances
    }

    #[test]
    fn ordered_amounts_moves_the_default_code_to_the_front() {
        let balances = balances_of(&[("AUD", "100.00"), ("USD", "50.00"), ("EUR", "25.00")]);
        let out = super::ordered_amounts(&balances, Some("USD"));
        assert_eq!(
            out.iter()
                .map(|a| a.currency_code.as_str())
                .collect::<Vec<_>>(),
            vec!["USD", "AUD", "EUR"],
            "the default commodity moves to the front; the rest keep their relative order"
        );
    }

    #[test]
    fn ordered_amounts_is_a_no_op_when_the_default_is_already_first() {
        let balances = balances_of(&[("AUD", "100.00"), ("USD", "50.00")]);
        let out = super::ordered_amounts(&balances, Some("AUD"));
        assert_eq!(
            out.iter()
                .map(|a| a.currency_code.as_str())
                .collect::<Vec<_>>(),
            vec!["AUD", "USD"],
            "already-first default code leaves first-seen order untouched"
        );
    }

    #[test]
    fn ordered_amounts_sorts_by_code_without_a_default() {
        let balances = balances_of(&[("USD", "50.00"), ("AUD", "100.00"), ("EUR", "25.00")]);
        let out = super::ordered_amounts(&balances, None);
        assert_eq!(
            out.iter()
                .map(|a| a.currency_code.as_str())
                .collect::<Vec<_>>(),
            vec!["AUD", "EUR", "USD"],
            "no default code means the remainder is sorted, not left in \
             first-seen order, so a root with no own balance does not flip \
             between fetches"
        );
    }

    #[test]
    fn ordered_amounts_of_empty_balances_is_empty() {
        let balances = bc_models::Balances::new();
        assert_eq!(super::ordered_amounts(&balances, Some("AUD")), vec![]);
        assert_eq!(super::ordered_amounts(&balances, None), vec![]);
    }

    #[test]
    fn metadata_from_preserves_order_and_drops_the_flag() {
        let entries = vec![
            bc_ipc::MetaEntryDto::flagged("payee", "Generic Grocer"),
            bc_ipc::MetaEntryDto::new("note", bc_ipc::MetaValueDto::Text("weekly shop".to_owned())),
        ];
        assert!(
            entries.first().is_some_and(|e| e.mismatched),
            "the first entry arrives claiming to be flagged"
        );

        let meta = super::metadata_from(&entries).expect("valid entries convert");
        assert_eq!(
            meta.iter().map(|e| e.key().as_str()).collect::<Vec<_>>(),
            vec!["payee", "note"],
            "input order is the display order"
        );
        assert!(
            meta.iter().all(|e| !e.mismatched()),
            "the write path derives the flag; an incoming entry does not assert it"
        );
    }

    #[test]
    fn metadata_from_rejects_an_invalid_key() {
        let entries = vec![bc_ipc::MetaEntryDto::new(
            "1nvoice",
            bc_ipc::MetaValueDto::Boolean(true),
        )];
        let err = super::metadata_from(&entries).expect_err("a key must start with a letter");
        assert!(
            matches!(err, bc_ipc::BcError::Validation(ref m) if m.contains("1nvoice")),
            "the failure names the offending key, got: {err:?}"
        );
    }

    #[test]
    fn resolve_tag_inputs_errors_on_unknown_tag() {
        tauri::async_runtime::block_on(async {
            let pool = bc_core::open_db("sqlite::memory:").await.expect("db");
            let tags = bc_core::TagService::new(pool);
            let err = super::resolve_tag_inputs(&tags, &["person:ghost".to_owned()])
                .await
                .expect_err("unknown tag must error");
            assert!(matches!(err, bc_ipc::BcError::Validation(_)));
        });
    }
}
