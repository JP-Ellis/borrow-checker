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

    let nodes = accounts
        .iter()
        .map(|account| {
            let balance = balances.get(account.id()).map(bc_ipc::Amount::from);
            bc_ipc::AccountNode::from_model(account, &forest, balance)
        })
        .collect::<Vec<_>>();

    Ok(nodes)
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
            .maybe_note(p.note.clone())
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
        .maybe_payee(Some(tx.payee))
        .description(tx.description)
        .maybe_note(tx.note)
        .postings(postings)
        .tag_ids(tx_tag_ids)
        .reconciliation(reconciliation)
        .created_at(jiff::Timestamp::now())
        .build();

    let tx_id = state
        .transactions
        .create(model_tx)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

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
            .maybe_note(p.note.clone())
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
        .maybe_payee(Some(tx.payee))
        .description(tx.description)
        .maybe_note(tx.note)
        .postings(postings)
        .reconciliation(reconciliation)
        .tag_ids(tag_ids)
        .extra_dates(tx.extra_dates)
        .created_at(jiff::Timestamp::now())
        .build();

    Ok(state.transactions.edit(model_tx).await?)
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
/// * `date_from`  - The inclusive start of the date window.
/// * `date_until` - The exclusive end of the date window.
/// * `state`      - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the account ID is invalid or a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_account_stats(
    account_id: String,
    commodity: Option<String>,
    date_from: jiff::civil::Date,
    date_until: jiff::civil::Date,
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

    let s = state
        .balance_engine
        .account_period_stats(&id, &commodity_code, date_from, date_until)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::AccountStats::new(
        bc_ipc::Amount::from(&s.income),
        bc_ipc::Amount::from(&s.expenses),
        bc_ipc::Amount::from(&s.net),
        bc_ipc::Amount::from(&s.opening),
        bc_ipc::Amount::from(&s.closing),
        s.tx_count,
    ))
}

/// Returns the most recent transaction date for `account_id`, or `None`.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `state`      - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the ID is invalid or the query fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn account_latest_activity(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<Option<jiff::civil::Date>, bc_ipc::BcError> {
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;
    state
        .transactions
        .latest_activity_date(&id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
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
        bc_models::Period::Fortnightly { .. } | bc_models::Period::Custom { .. } => {
            start.to_string()
        }
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
    Ok(trail
        .iter()
        .map(|(ts, event)| bc_ipc::AuditEntry::from_event(*ts, event))
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
/// * `count`      - Optional bucket count (default 6).
/// * `period`     - Optional bucket period (default Monthly).
/// * `state`      - Tauri managed application state.
///
/// # Panics
///
/// Never panics in practice — the internal `NonZeroUsize::new(6)` is a
/// compile-time constant and can never be zero.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the account ID is invalid or a service call fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_account_sparkline(
    account_id: String,
    commodity: Option<String>,
    count: Option<u32>,
    period: Option<bc_ipc::Period>,
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

    let as_of = jiff::Zoned::now().date();

    let buckets = state
        .balance_engine
        .posting_buckets(&id, &commodity_code, &model_period, bucket_count, as_of)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

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
mod tests {
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
