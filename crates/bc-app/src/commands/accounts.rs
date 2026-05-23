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

// MARK: Model → IPC conversions

/// Returns the number of minor-unit decimal places for `currency_code`.
///
/// Falls back to `2` (cents) for codes not found in the IPC registry.
#[inline]
fn currency_decimals(currency_code: &str) -> u32 {
    bc_ipc::currency_from_code(currency_code).map_or(2, |c| u32::from(c.decimals))
}

/// Conversions from `bc_models` types to `bc_ipc` types and back.
///
/// These cannot use the standard [`From`] trait because both sides are
/// defined in external crates (orphan rule). The extension-trait pattern is
/// the standard Rust alternative for exactly this situation.
trait IntoIpc {
    /// The IPC counterpart type.
    type Output;
    /// Convert `self` into its IPC representation.
    fn into_ipc(self) -> Self::Output;
}

/// Conversions from `bc_ipc` types back to `bc_models` domain types.
///
/// The inverse of [`IntoIpc`]. Same orphan-rule rationale applies.
trait IntoModel {
    /// The domain model counterpart type.
    type Output;
    /// Convert `self` into its domain model representation.
    fn into_model(self) -> Self::Output;
}

impl IntoIpc for &bc_models::Amount {
    type Output = bc_ipc::Amount;

    /// Converts a [`bc_models::Amount`] to an IPC [`bc_ipc::Amount`].
    ///
    /// Multiplies the decimal value by `10 ^ decimals` to produce minor units,
    /// using midpoint-nearest-even rounding. If the value cannot be represented
    /// as `i64`, it is clamped to `0`.
    #[inline]
    fn into_ipc(self) -> bc_ipc::Amount {
        let code = self.commodity().as_str();
        let decimals = currency_decimals(code);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Decimal multiplication by a power-of-ten scale factor is bounded by the IPC contract and safe in practice"
        )]
        let minor = (self.value() * rust_decimal::Decimal::from(10_u64.pow(decimals)))
            .round() // midpoint-nearest-even (banker's rounding)
            .to_i64()
            .unwrap_or(0);
        bc_ipc::Amount::new(minor, code)
    }
}

impl IntoModel for &bc_ipc::Amount {
    type Output = bc_models::Amount;

    /// Converts an IPC [`bc_ipc::Amount`] to a [`bc_models::Amount`].
    ///
    /// Divides `minor_units` by `10 ^ decimals` to recover the decimal value.
    #[inline]
    fn into_model(self) -> bc_models::Amount {
        let decimals = currency_decimals(&self.currency_code);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Decimal division by a power-of-ten scale factor is bounded by the IPC contract and safe in practice"
        )]
        let value = rust_decimal::Decimal::from(self.minor_units)
            / rust_decimal::Decimal::from(10_u64.pow(decimals));
        bc_models::Amount::new(
            value,
            bc_models::CommodityCode::new(self.currency_code.clone()),
        )
    }
}

impl IntoIpc for bc_models::AccountType {
    type Output = bc_ipc::AccountType;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::AccountType and bc_ipc::AccountType are #[non_exhaustive]; \
                  the wildcard fallback to Asset is intentional for future unknown variants"
    )]
    fn into_ipc(self) -> bc_ipc::AccountType {
        match self {
            bc_models::AccountType::Asset => bc_ipc::AccountType::Asset,
            bc_models::AccountType::Liability => bc_ipc::AccountType::Liability,
            bc_models::AccountType::Equity => bc_ipc::AccountType::Equity,
            bc_models::AccountType::Income => bc_ipc::AccountType::Income,
            bc_models::AccountType::Expense => bc_ipc::AccountType::Expense,
            _ => bc_ipc::AccountType::Asset,
        }
    }
}

impl IntoIpc for bc_models::TransactionStatus {
    type Output = bc_ipc::TxStatus;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::TransactionStatus and bc_ipc::TxStatus are #[non_exhaustive]; \
                  Voided is kept explicit even though the wildcard fallback also maps to Unreconciled"
    )]
    fn into_ipc(self) -> bc_ipc::TxStatus {
        match self {
            bc_models::TransactionStatus::Cleared => bc_ipc::TxStatus::Cleared,
            bc_models::TransactionStatus::Pending => bc_ipc::TxStatus::Pending,
            bc_models::TransactionStatus::Voided => bc_ipc::TxStatus::Unreconciled,
            _ => bc_ipc::TxStatus::Unreconciled,
        }
    }
}

impl IntoModel for bc_ipc::TxStatus {
    type Output = bc_models::TransactionStatus;

    #[inline]
    fn into_model(self) -> bc_models::TransactionStatus {
        match self {
            bc_ipc::TxStatus::Cleared => bc_models::TransactionStatus::Cleared,
            bc_ipc::TxStatus::Pending | bc_ipc::TxStatus::Unreconciled => {
                bc_models::TransactionStatus::Pending
            }
            _ => bc_models::TransactionStatus::Pending,
        }
    }
}

impl IntoIpc for &bc_models::Account {
    type Output = bc_ipc::AccountNode;

    #[inline]
    fn into_ipc(self) -> bc_ipc::AccountNode {
        bc_ipc::AccountNode::new(
            self.id().to_string(),
            self.name(),
            None::<&str>,
            bc_ipc::Amount::new(0, "AUD"), // TODO(ipc): compute via BalanceEngine
            self.parent_id().map(ToString::to_string),
            self.account_type().into_ipc(),
            vec![],
        )
    }
}

impl IntoIpc for &bc_models::Posting {
    type Output = bc_ipc::Posting;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Posting {
        let account_id = self.account_id().to_string();
        bc_ipc::Posting::new(
            account_id.clone(),
            account_id, // TODO(ipc): resolve display path via AccountService
            self.amount().into_ipc(),
            self.memo(),
        )
    }
}

impl IntoIpc for &bc_models::Transaction {
    type Output = bc_ipc::Transaction;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Transaction {
        let postings: Vec<bc_ipc::Posting> =
            self.postings().iter().map(IntoIpc::into_ipc).collect();
        bc_ipc::Transaction::new(
            self.id().to_string(),
            self.date().to_string(),
            self.payee().unwrap_or_default(),
            self.status().into_ipc(),
            vec![], // TODO(ipc): resolve tag paths via TagService
            postings,
            vec![],
        )
    }
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
    Ok(accounts.iter().map(IntoIpc::into_ipc).collect())
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
    Ok(txs.map(|tx| tx.into_ipc()).collect())
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
