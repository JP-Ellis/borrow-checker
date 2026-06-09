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

/// Aggregate income and expense totals for a time window.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountStats {
    /// Sum of positive postings (money entering the account).
    pub income: Amount,
    /// Sum of negative postings, as a positive magnitude (money leaving the account).
    pub expenses: Amount,
}

impl AccountStats {
    /// Creates a new [`AccountStats`].
    ///
    /// # Arguments
    ///
    /// * `income`   - Total inflow amount.
    /// * `expenses` - Total outflow amount (positive magnitude).
    #[must_use]
    #[inline]
    pub fn new(income: Amount, expenses: Amount) -> Self {
        Self { income, expenses }
    }
}

/// A single data point in a cash-flow sparkline: a time-bucket label plus
/// income and expense totals in the account's minor unit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SparkPoint {
    /// X-axis label, e.g. `"apr"`, `"w03"`, `"Q2"`.
    pub label: String,
    /// Income in the currency's minor unit (positive).
    pub income: i64,
    /// Expenses in the currency's minor unit (positive magnitude — plotted separately).
    pub expenses: i64,
}

impl SparkPoint {
    /// Creates a new [`SparkPoint`].
    ///
    /// # Arguments
    ///
    /// * `label`    - X-axis label.
    /// * `income`   - Income in minor units.
    /// * `expenses` - Expenses in minor units (positive magnitude).
    #[must_use]
    #[inline]
    pub fn new(label: impl Into<String>, income: i64, expenses: i64) -> Self {
        Self {
            label: label.into(),
            income,
            expenses,
        }
    }
}

/// A time-bucket granularity for period-based data (sparklines, reports, budgets).
///
/// Maps to [`bc_models::Period`] on the backend; only variants meaningful to a
/// frontend caller are exposed here.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Period {
    /// Every 7 days.
    Weekly,
    /// Calendar month.
    Monthly,
    /// Calendar quarter (Jan/Apr/Jul/Oct).
    Quarterly,
    /// Calendar year (1 January).
    CalendarYear,
    /// Financial year with configurable start.
    FinancialYear {
        /// 1-based start month (1–12).
        start_month: u8,
        /// 1-based start day (1–28).
        start_day: u8,
    },
}

impl Period {
    /// Returns the default number of buckets to display in a sparkline for this period.
    ///
    /// Weekly shows 8 weeks; all other periods default to 6 buckets.
    #[must_use]
    #[inline]
    pub fn default_sparkline_count(&self) -> u32 {
        match self {
            Self::Weekly => 8,
            Self::Monthly | Self::Quarterly | Self::CalendarYear | Self::FinancialYear { .. } => 6,
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

    #[test]
    fn period_weekly_serde_roundtrip() {
        let p = Period::Weekly;
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(json, r#"{"type":"weekly"}"#);
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_monthly_serde_roundtrip() {
        let p = Period::Monthly;
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(json, r#"{"type":"monthly"}"#);
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_quarterly_serde_roundtrip() {
        let p = Period::Quarterly;
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(json, r#"{"type":"quarterly"}"#);
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_calendar_year_serde_roundtrip() {
        let p = Period::CalendarYear;
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(json, r#"{"type":"calendar_year"}"#);
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_financial_year_serde_roundtrip() {
        let p = Period::FinancialYear {
            start_month: 7,
            start_day: 1,
        };
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(
            json,
            r#"{"type":"financial_year","start_month":7,"start_day":1}"#
        );
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_default_sparkline_count() {
        assert_eq!(Period::Weekly.default_sparkline_count(), 8);
        assert_eq!(Period::Monthly.default_sparkline_count(), 6);
        assert_eq!(Period::Quarterly.default_sparkline_count(), 6);
        assert_eq!(Period::CalendarYear.default_sparkline_count(), 6);
        assert_eq!(
            Period::FinancialYear {
                start_month: 7,
                start_day: 1,
            }
            .default_sparkline_count(),
            6
        );
    }
}
