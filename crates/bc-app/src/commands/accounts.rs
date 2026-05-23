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

use rust_decimal::prelude::ToPrimitive as _;
use tauri::State;

use crate::AppState;

// MARK: Mapping helpers

/// Maps a [`bc_models::AccountType`] to the IPC [`bc_ipc::AccountType`].
#[inline]
#[expect(
    clippy::match_same_arms,
    reason = "both bc_models::AccountType and bc_ipc::AccountType are #[non_exhaustive]; \
              the wildcard fallback to Asset is intentional for future unknown variants"
)]
fn map_account_type(t: bc_models::AccountType) -> bc_ipc::AccountType {
    match t {
        bc_models::AccountType::Asset => bc_ipc::AccountType::Asset,
        bc_models::AccountType::Liability => bc_ipc::AccountType::Liability,
        bc_models::AccountType::Equity => bc_ipc::AccountType::Equity,
        bc_models::AccountType::Income => bc_ipc::AccountType::Income,
        bc_models::AccountType::Expense => bc_ipc::AccountType::Expense,
        _ => bc_ipc::AccountType::Asset,
    }
}

/// Maps a [`bc_models::TransactionStatus`] to the IPC [`bc_ipc::TxStatus`].
#[inline]
#[expect(
    clippy::match_same_arms,
    reason = "both bc_models::TransactionStatus and bc_ipc::TxStatus are #[non_exhaustive]; \
              Voided is kept explicit even though the wildcard fallback also maps to Unreconciled"
)]
fn map_tx_status(s: bc_models::TransactionStatus) -> bc_ipc::TxStatus {
    match s {
        bc_models::TransactionStatus::Cleared => bc_ipc::TxStatus::Cleared,
        bc_models::TransactionStatus::Pending => bc_ipc::TxStatus::Pending,
        bc_models::TransactionStatus::Voided => bc_ipc::TxStatus::Unreconciled,
        _ => bc_ipc::TxStatus::Unreconciled,
    }
}

/// Maps an IPC [`bc_ipc::TxStatus`] to a [`bc_models::TransactionStatus`].
#[inline]
fn map_ipc_status(s: bc_ipc::TxStatus) -> bc_models::TransactionStatus {
    match s {
        bc_ipc::TxStatus::Cleared => bc_models::TransactionStatus::Cleared,
        bc_ipc::TxStatus::Pending | bc_ipc::TxStatus::Unreconciled => {
            bc_models::TransactionStatus::Pending
        }
        _ => bc_models::TransactionStatus::Pending,
    }
}

/// Returns the number of minor-unit decimal places for `currency_code`.
///
/// Falls back to `2` (cents) for codes not found in the IPC registry.
#[inline]
fn currency_decimals(currency_code: &str) -> u32 {
    bc_ipc::currency_from_code(currency_code).map_or(2, |c| u32::from(c.decimals))
}

/// Converts a [`bc_models::Amount`] to an IPC [`bc_ipc::Money`].
///
/// Multiplies the decimal value by `10 ^ decimals` to produce minor units.
/// If the value cannot be represented as `i64`, it is clamped to `0`.
#[inline]
fn amount_to_money(amount: &bc_models::Amount) -> bc_ipc::Money {
    let code = amount.commodity().as_str();
    let decimals = currency_decimals(code);
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "Decimal multiplication by a power-of-ten scale factor is bounded by the IPC contract and safe in practice"
    )]
    let minor = (amount.value() * rust_decimal::Decimal::from(10_u64.pow(decimals)))
        .round() // half-up rounding before truncation
        .to_i64()
        .unwrap_or(0);
    bc_ipc::Money::new(minor, code)
}

/// Converts an IPC [`bc_ipc::Money`] to a [`bc_models::Amount`].
///
/// Divides `minor_units` by `10 ^ decimals` to recover the decimal value.
#[inline]
fn money_to_amount(money: &bc_ipc::Money) -> bc_models::Amount {
    let decimals = currency_decimals(&money.currency_code);
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "Decimal division by a power-of-ten scale factor is bounded by the IPC contract and safe in practice"
    )]
    let value = rust_decimal::Decimal::from(money.minor_units)
        / rust_decimal::Decimal::from(10_u64.pow(decimals));
    bc_models::Amount::new(
        value,
        bc_models::CommodityCode::new(money.currency_code.clone()),
    )
}

/// Converts a [`bc_models::Account`] to an IPC [`bc_ipc::AccountNode`].
#[inline]
fn account_to_node(account: &bc_models::Account) -> bc_ipc::AccountNode {
    bc_ipc::AccountNode::new(
        account.id().to_string(),
        account.name(),
        None::<&str>,
        bc_ipc::Money::new(0, "AUD"), // TODO(ipc): compute via BalanceEngine
        account.parent_id().map(ToString::to_string),
        map_account_type(account.account_type()),
        vec![],
    )
}

/// Converts a [`bc_models::Posting`] to an IPC [`bc_ipc::Posting`].
#[inline]
fn posting_to_ipc(posting: &bc_models::Posting) -> bc_ipc::Posting {
    let account_id = posting.account_id().to_string();
    bc_ipc::Posting::new(
        account_id.clone(),
        account_id, // TODO(ipc): resolve display path via AccountService
        amount_to_money(posting.amount()),
        posting.memo(),
    )
}

/// Converts a [`bc_models::Transaction`] to an IPC [`bc_ipc::Transaction`].
#[inline]
fn transaction_to_ipc(tx: &bc_models::Transaction) -> bc_ipc::Transaction {
    let postings: Vec<bc_ipc::Posting> = tx.postings().iter().map(posting_to_ipc).collect();
    bc_ipc::Transaction::new(
        tx.id().to_string(),
        tx.date().to_string(),
        tx.payee().unwrap_or_default(),
        map_tx_status(tx.status()),
        vec![], // TODO(ipc): resolve tag paths via TagService
        postings,
        vec![],
    )
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
    Ok(accounts.iter().map(account_to_node).collect())
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
    let txs = state
        .transactions
        .list_for_account(&id)
        .await
        .map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;
    Ok(txs.map(|tx| transaction_to_ipc(&tx)).collect())
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

    let status = map_ipc_status(tx.status);

    let mut postings = Vec::with_capacity(tx.postings.len());
    for p in &tx.postings {
        let account_id = p.account_id.parse::<bc_models::AccountId>().map_err(|e| {
            bc_ipc::BcError::Validation(format!("invalid account_id '{}': {e}", p.account_id))
        })?;
        let posting = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account_id)
            .amount(money_to_amount(&p.amount))
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
