//! Account and transaction types shared between Tauri backend and Leptos frontend.

use serde::Deserialize;
use serde::Serialize;

use crate::money::Amount;

/// The five canonical account types in double-entry accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AccountType {
    /// A resource owned by the entity (e.g. bank account, brokerage).
    Asset,
    /// An obligation owed by the entity (e.g. credit card, loan).
    Liability,
    /// Net assets / retained earnings.
    Equity,
    /// Amount flowing into the entity.
    Income,
    /// Amount flowing out of the entity.
    Expense,
}

/// A single node in the account tree sidebar.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountNode {
    /// Stable identifier used in routing (`/accounts/:id`).
    pub id: String,
    /// Display name.
    pub name: String,
    /// Last-four mask, e.g. `"4421"`. `None` for non-bank accounts.
    pub mask: Option<String>,
    /// Account balance (negative = liability).
    pub balance: Amount,
    /// `Some(parent_id)` for child accounts, `None` for top-level groups.
    pub parent_id: Option<String>,
    /// Account type (asset, liability, equity, income, or expense).
    pub account_type: AccountType,
    /// Tag paths attached to this account (colon-joined, e.g. `"institution:commbank"`).
    pub tags: Vec<String>,
}

impl AccountNode {
    /// Creates a new [`AccountNode`].
    ///
    /// # Arguments
    ///
    /// * `id` - Stable identifier used in routing.
    /// * `name` - Display name.
    /// * `mask` - Last-four mask, or `None`.
    /// * `balance` - Account balance.
    /// * `parent_id` - Parent account ID, or `None` for top-level.
    /// * `account_type` - Account type.
    /// * `tags` - Tag paths (colon-joined).
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        mask: Option<impl Into<String>>,
        balance: Amount,
        parent_id: Option<impl Into<String>>,
        account_type: AccountType,
        tags: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            mask: mask.map(Into::into),
            balance,
            parent_id: parent_id.map(Into::into),
            account_type,
            tags,
        }
    }
}

/// Cleared / pending / unreconciled status of a transaction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TxStatus {
    /// Bank-confirmed.
    Cleared,
    /// Not yet confirmed by the bank.
    Pending,
    /// Imported but not reviewed.
    Unreconciled,
}

impl TxStatus {
    /// Returns the display label string for this status.
    #[must_use]
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::Cleared => "cleared",
            Self::Pending => "pending",
            Self::Unreconciled => "unreconciled",
        }
    }
}

/// One leg of a double-entry transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Posting {
    /// Account ID — matches [`AccountNode::id`].
    pub account_id: String,
    /// Full account path for display, e.g. `"Assets :: Smart Access"`.
    pub account_path: String,
    /// Posting amount. Positive = credit; negative = debit.
    pub amount: Amount,
    /// Optional inline comment shown in the TOML view.
    pub note: Option<String>,
}

impl Posting {
    /// Creates a new [`Posting`].
    ///
    /// # Arguments
    ///
    /// * `account_id` - Account ID matching [`AccountNode::id`].
    /// * `account_path` - Full account path for display.
    /// * `amount` - Posting amount.
    /// * `note` - Optional inline comment, or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        account_id: impl Into<String>,
        account_path: impl Into<String>,
        amount: Amount,
        note: Option<impl Into<String>>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            account_path: account_path.into(),
            amount,
            note: note.map(Into::into),
        }
    }
}

/// An entry in the transaction audit log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditEntry {
    /// Timestamp string, e.g. `"09:04"`.
    pub time: String,
    /// Event kind, e.g. `"import"`, `"autocat"`, `"user"`.
    pub kind: String,
    /// Human-readable message.
    pub message: String,
}

impl AuditEntry {
    /// Creates a new [`AuditEntry`].
    ///
    /// # Arguments
    ///
    /// * `time` - Timestamp string (e.g. `"09:04"`).
    /// * `kind` - Event kind (e.g. `"import"`, `"autocat"`).
    /// * `message` - Human-readable message.
    #[must_use]
    #[inline]
    pub fn new(
        time: impl Into<String>,
        kind: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            time: time.into(),
            kind: kind.into(),
            message: message.into(),
        }
    }
}

/// A transaction as shown in the register.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Transaction {
    /// Stable identifier.
    pub id: String,
    /// ISO-8601 date string, e.g. `"2026-04-30"`.
    pub date: String,
    /// Payee display name.
    pub payee: String,
    /// Transaction status.
    pub status: TxStatus,
    /// Tag paths attached to this transaction.
    pub tags: Vec<String>,
    /// All postings. Must sum to zero (double-entry invariant).
    pub postings: Vec<Posting>,
    /// Audit trail entries (chronological).
    pub audit: Vec<AuditEntry>,
}

impl Transaction {
    /// Creates a new [`Transaction`].
    ///
    /// # Arguments
    ///
    /// * `id` - Stable identifier.
    /// * `date` - ISO-8601 date string (e.g. `"2026-04-30"`).
    /// * `payee` - Payee display name.
    /// * `status` - Transaction status.
    /// * `tags` - Tag paths.
    /// * `postings` - All postings (must sum to zero).
    /// * `audit` - Audit trail entries.
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        date: impl Into<String>,
        payee: impl Into<String>,
        status: TxStatus,
        tags: Vec<String>,
        postings: Vec<Posting>,
        audit: Vec<AuditEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            date: date.into(),
            payee: payee.into(),
            status,
            tags,
            postings,
            audit,
        }
    }
}

/// The user-supplied fields for a new posting leg.
///
/// Omits `account_path` (derived by the backend from the account record) and
/// cost/envelope fields (out of scope for v0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewPosting {
    /// Account ID — must reference an existing active account.
    pub account_id: String,
    /// Posting amount. Positive = credit; negative = debit.
    pub amount: Amount,
    /// Optional inline note.
    pub note: Option<String>,
}

impl NewPosting {
    /// Creates a new [`NewPosting`].
    ///
    /// # Arguments
    ///
    /// * `account_id` - Account ID referencing an existing active account.
    /// * `amount` - Posting amount (positive = credit; negative = debit).
    /// * `note` - Optional inline note, or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        account_id: impl Into<String>,
        amount: Amount,
        note: Option<impl Into<String>>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            amount,
            note: note.map(Into::into),
        }
    }
}

/// The user-supplied fields for a new transaction.
///
/// The backend assigns the `id` and appends the initial audit entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewTransaction {
    /// ISO-8601 date string, e.g. `"2026-05-23"`.
    pub date: String,
    /// Payee display name.
    pub payee: String,
    /// Transaction status.
    pub status: TxStatus,
    /// Tag paths attached to this transaction.
    pub tags: Vec<String>,
    /// All postings. Must sum to zero per commodity (enforced by the backend).
    pub postings: Vec<NewPosting>,
}

impl NewTransaction {
    /// Creates a new [`NewTransaction`].
    ///
    /// # Arguments
    ///
    /// * `date` - ISO-8601 date string (e.g. `"2026-05-23"`).
    /// * `payee` - Payee display name.
    /// * `status` - Transaction status.
    /// * `tags` - Tag paths attached to this transaction.
    /// * `postings` - All postings (must sum to zero per commodity).
    #[must_use]
    #[inline]
    pub fn new(
        date: impl Into<String>,
        payee: impl Into<String>,
        status: TxStatus,
        tags: Vec<String>,
        postings: Vec<NewPosting>,
    ) -> Self {
        Self {
            date: date.into(),
            payee: payee.into(),
            status,
            tags,
            postings,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::Amount;

    #[test]
    fn new_posting_constructor_roundtrip() {
        let p = NewPosting::new("acc-1", Amount::new(-1_000, "AUD", 2), Some("test note"));
        assert_eq!(p.account_id, "acc-1");
        assert_eq!(p.amount.minor_units, -1_000);
        assert_eq!(p.note.as_deref(), Some("test note"));
    }

    #[test]
    fn new_transaction_serde_roundtrip() {
        let tx = NewTransaction::new(
            "2026-05-23",
            "Test Payee",
            TxStatus::Pending,
            vec![],
            vec![
                NewPosting::new("acc-a", Amount::new(-500, "AUD", 2), None::<&str>),
                NewPosting::new("acc-b", Amount::new(500, "AUD", 2), None::<&str>),
            ],
        );
        let json = serde_json::to_string(&tx).expect("serialises");
        let tx2: NewTransaction = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(tx, tx2);
    }
}
