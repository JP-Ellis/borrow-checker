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

use tauri::State;

use crate::AppState;
use crate::ipc::IntoIpc;
use crate::ipc::IntoModel;

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
            let balance = balances.get(account.id()).map(IntoIpc::into_ipc);
            crate::ipc::into_ipc_with_balance(account, balance, &forest)
        })
        .collect::<Vec<_>>();

    Ok(nodes)
}

/// List transactions for the given account.
///
/// # Arguments
///
/// * `account_id` - The account ID to filter by.
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
        .list_for_account(&id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(txs
        .map(|tx| crate::ipc::transaction_into_ipc_with_accounts(&tx, &account_map, &forest))
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
    let reconciliation = tx.reconciliation.into_model();

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p.account_id.parse::<bc_models::AccountId>().map_err(|e| {
            bc_ipc::BcError::Validation(format!("invalid account_id '{}': {e}", p.account_id))
        })?;
        let tag_ids = resolve_tag_inputs(&state.tags, &p.tags).await?;
        let posting = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id)
            .maybe_amount(p.amount.as_ref().map(IntoModel::into_model))
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
    let reconciliation = tx.reconciliation.into_model();

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
            .maybe_amount(p.amount.as_ref().map(IntoModel::into_model))
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

    state
        .transactions
        .edit(model_tx)
        .await
        .map_err(|e| e.into_ipc())
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
    state
        .transactions
        .reconcile(&tx_id, reconciliation.into_model())
        .await
        .map_err(|e| e.into_ipc())
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

/// Returns income and expense totals for `account_id` over the last 30 days.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `commodity`  - Optional commodity code override. Defaults to the account's first commodity.
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

    let today = jiff::Zoned::now().date();
    let from = today.saturating_sub(jiff::Span::new().days(29_i32));
    let tomorrow = today.saturating_add(jiff::Span::new().days(1_i32));

    let (inflow, outflow) = state
        .balance_engine
        .posting_flows(&id, &commodity_code, from, tomorrow)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::AccountStats::new(
        (&inflow).into_ipc(),
        (&outflow).into_ipc(),
    ))
}

/// Returns the count of non-voided postings for `account_id`.
///
/// Voided transactions are excluded.
///
/// # Arguments
///
/// * `account_id` - The account to query.
/// * `state`      - Tauri managed application state.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError`] if the account ID is invalid or the query fails.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn get_posting_count(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<u32, bc_ipc::BcError> {
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;
    state
        .balance_engine
        .posting_count(&id)
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

/// Maps a core [`bc_core::Event`] to a UI audit entry with a short kind tag.
///
/// # Arguments
///
/// * `ts` - When the event was recorded.
/// * `event` - The core event to describe.
///
/// # Returns
///
/// A [`bc_ipc::AuditEntry`] with a short kind and a human-readable message.
fn audit_entry_from(ts: jiff::Timestamp, event: &bc_core::Event) -> bc_ipc::AuditEntry {
    use bc_core::Event;
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is #[non_exhaustive]; catch-all arm required for exhaustiveness against future variants"
    )]
    let (kind, message): (&str, String) = match event {
        Event::TransactionCreated { .. } => ("create", "transaction created".to_owned()),
        Event::TransactionAmended { .. } => ("amend", "transaction amended".to_owned()),
        Event::TransactionVoided { .. } => ("void", "transaction voided".to_owned()),
        Event::TransactionReversed { .. } => ("reverse", "transaction reversed".to_owned()),
        Event::TransactionPayeeChanged { to, .. } => (
            "payee",
            format!("payee → {}", to.as_deref().unwrap_or("(none)")),
        ),
        Event::TransactionDateChanged { to, .. } => ("date", format!("date → {to}")),
        Event::TransactionExtraDatesChanged { .. } => ("dates", "extra dates changed".to_owned()),
        Event::TransactionDescriptionChanged { .. } => ("desc", "description changed".to_owned()),
        Event::TransactionNoteChanged { to, .. } => (
            "note",
            match to {
                Some(_) => "note changed".to_owned(),
                None => "note removed".to_owned(),
            },
        ),
        Event::TransactionTagsChanged { added, removed, .. } => {
            ("tags", format!("tags +{} -{}", added.len(), removed.len()))
        }
        Event::TransactionReconciled { from, to, .. } => {
            ("reconcile", format!("reconciliation {from:?} → {to:?}"))
        }
        Event::PostingRecategorised { to_account, .. } => {
            ("recat", format!("recategorised → {to_account}"))
        }
        Event::PostingAmountChanged { .. } => ("amount", "amount changed".to_owned()),
        Event::PostingNoteChanged { .. } => ("note", "posting note changed".to_owned()),
        Event::PostingSpreadChanged { to, .. } => (
            "spread",
            match to {
                Some((from, until)) => format!("spread {from}..{until}"),
                None => "spread cleared".to_owned(),
            },
        ),
        Event::PostingAdded { account, .. } => ("split", format!("+leg {account}")),
        Event::PostingRemoved { .. } => ("split", "removed leg".to_owned()),
        other => {
            let k = other.kind();
            (k, k.to_owned())
        }
    };
    bc_ipc::AuditEntry::new(ts, kind.to_owned(), message)
}

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
        .map(|(ts, event)| audit_entry_from(*ts, event))
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

    let model_period = period.map_or(bc_models::Period::Monthly, IntoModel::into_model);

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
                (&b.inflow).into_ipc(),
                (&b.outflow).into_ipc(),
            )
        })
        .collect::<Vec<_>>();

    Ok(points)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

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

    #[test]
    fn audit_entry_from_recategorise_uses_recat_kind() {
        let event = bc_core::Event::PostingRecategorised {
            id: bc_models::TransactionId::new(),
            posting_id: bc_models::PostingId::new(),
            from_account: bc_models::AccountId::new(),
            to_account: bc_models::AccountId::new(),
        };
        let entry = super::audit_entry_from(jiff::Timestamp::now(), &event);
        assert_eq!(entry.kind, "recat");
        assert!(!entry.message.is_empty());
    }
}
