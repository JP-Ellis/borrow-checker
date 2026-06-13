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

    let nodes = accounts
        .iter()
        .map(|account| {
            let balance = balances
                .get(account.id())
                .map(|(c, d)| crate::ipc::decimal_to_amount(*d, c))
                .transpose()?;
            Ok(crate::ipc::into_ipc_with_balance(account, balance))
        })
        .collect::<Result<Vec<_>, _>>()?;

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

    let txs = state
        .transactions
        .list_for_account(&id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(txs
        .map(|tx| crate::ipc::transaction_into_ipc_with_accounts(&tx, &account_map))
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
    let date = tx
        .date
        .parse::<jiff::civil::Date>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid date '{}': {e}", tx.date)))?;

    let status = tx.status.into_model();

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p.account_id.parse::<bc_models::AccountId>().map_err(|e| {
            bc_ipc::BcError::Validation(format!("invalid account_id '{}': {e}", p.account_id))
        })?;
        let posting = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id)
            .amount(p.amount.into_model())
            .maybe_memo(p.note.clone())
            .build();
        postings.push(posting);
    }

    let model_tx = bc_models::Transaction::builder()
        .id(bc_models::TransactionId::new())
        .date(date)
        .maybe_payee(Some(tx.payee))
        .description(String::new())
        .postings(postings)
        .status(status)
        .created_at(jiff::Timestamp::now())
        .build();

    let tx_id = state
        .transactions
        .create(model_tx)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(tx_id.to_string())
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
        crate::ipc::decimal_to_amount(inflow, &commodity_code)?,
        crate::ipc::decimal_to_amount(outflow, &commodity_code)?,
    ))
}

/// Returns the count of uncategorised postings for `account_id`.
///
/// A posting is uncategorised when it has no `envelope_id`. Voided transactions
/// are excluded.
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
pub async fn get_uncategorised_count(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<u32, bc_ipc::BcError> {
    let id = account_id
        .parse::<bc_models::AccountId>()
        .map_err(|e| bc_ipc::BcError::Validation(format!("invalid account_id: {e}")))?;
    state
        .balance_engine
        .uncategorised_count(&id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))
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
            let income_amount = crate::ipc::decimal_to_amount(b.inflow, &commodity_code)?;
            let expenses_amount = crate::ipc::decimal_to_amount(b.outflow, &commodity_code)?;
            Ok(bc_ipc::SparkPoint::new(
                spark_label(b.start, &model_period),
                income_amount.minor_units,
                expenses_amount.minor_units,
            ))
        })
        .collect::<Result<Vec<_>, bc_ipc::BcError>>()?;

    Ok(points)
}
