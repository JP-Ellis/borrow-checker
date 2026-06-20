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

impl AccountType {
    /// Returns the lowercase display label for this account type.
    ///
    /// # Returns
    ///
    /// A static string such as `"asset"`, `"liability"`, etc.
    ///
    /// # Example
    ///
    /// ```
    /// # use bc_ipc::AccountType;
    /// assert_eq!(AccountType::Asset.label(), "asset");
    /// assert_eq!(AccountType::Liability.label(), "liability");
    /// ```
    #[must_use]
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::Asset => "asset",
            Self::Liability => "liability",
            Self::Equity => "equity",
            Self::Income => "income",
            Self::Expense => "expense",
        }
    }
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
    /// Current balance in the account's default commodity, or `None` if no commodity is configured.
    pub balance: Option<Amount>,
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
        balance: Option<Amount>,
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

/// A reference to an account carrying both its stable identifier and display name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountRef {
    /// Stable account identifier — matches [`AccountNode::id`].
    pub id: String,
    /// Human-readable display name, e.g. `"Assets :: Smart Access"`.
    pub name: String,
}

impl AccountRef {
    /// Creates a new [`AccountRef`].
    ///
    /// # Arguments
    ///
    /// * `id` - Stable account identifier matching [`AccountNode::id`].
    /// * `name` - Human-readable display name.
    ///
    /// # Example
    ///
    /// ```
    /// # use bc_ipc::AccountRef;
    /// let r = AccountRef::new("account_abc", "Assets :: Smart Access");
    /// assert_eq!(r.id, "account_abc");
    /// assert_eq!(r.name, "Assets :: Smart Access");
    /// ```
    #[must_use]
    #[inline]
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// One leg of a double-entry transaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Posting {
    /// Stable posting identifier (UUID string).
    pub id: String,
    /// Account reference — carries the stable ID and human-readable display name.
    pub account: AccountRef,
    /// Posting amount. Positive = credit; negative = debit.
    pub amount: Amount,
    /// Optional inline comment shown in the TOML view.
    pub note: Option<String>,
    /// Accrual spread start date. `None` means no spreading applied.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end date (inclusive — the last day of the spread). `None` means no spreading applied.
    pub spread_until: Option<jiff::civil::Date>,
}

impl Posting {
    /// Creates a new [`Posting`].
    ///
    /// # Arguments
    ///
    /// * `id` - Stable posting identifier (UUID string).
    /// * `account` - Account reference with ID and display name.
    /// * `amount` - Posting amount.
    /// * `note` - Optional inline comment, or `None`.
    /// * `spread_from` - Accrual spread start date, or `None`.
    /// * `spread_until` - Accrual spread end date (inclusive — the last day of the spread), or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        account: AccountRef,
        amount: Amount,
        note: Option<impl Into<String>>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            id: id.into(),
            account,
            amount,
            note: note.map(Into::into),
            spread_from,
            spread_until,
        }
    }
}

/// An entry in the transaction audit log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AuditEntry {
    /// Instant this audit event occurred.
    pub time: jiff::Timestamp,
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
    /// * `time` - Instant the event occurred.
    /// * `kind` - Event kind (e.g. `"import"`, `"autocat"`).
    /// * `message` - Human-readable message.
    #[must_use]
    #[inline]
    pub fn new(time: jiff::Timestamp, kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            time,
            kind: kind.into(),
            message: message.into(),
        }
    }

    /// Returns the event time as a `"HH:MM"` label in the system time zone.
    #[must_use]
    #[inline]
    pub fn time_label(&self) -> String {
        self.time
            .to_zoned(jiff::tz::TimeZone::system())
            .strftime("%H:%M")
            .to_string()
    }
}

/// A transaction as shown in the register.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Transaction {
    /// Stable identifier.
    pub id: String,
    /// Transaction date.
    pub date: jiff::civil::Date,
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
    /// * `date` - Transaction date.
    /// * `payee` - Payee display name.
    /// * `status` - Transaction status.
    /// * `tags` - Tag paths.
    /// * `postings` - All postings (must sum to zero).
    /// * `audit` - Audit trail entries.
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        date: jiff::civil::Date,
        payee: impl Into<String>,
        status: TxStatus,
        tags: Vec<String>,
        postings: Vec<Posting>,
        audit: Vec<AuditEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            date,
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
/// cost basis fields (out of scope for v0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewPosting {
    /// Account ID — must reference an existing active account.
    pub account_id: String,
    /// Posting amount. Positive = credit; negative = debit.
    pub amount: Amount,
    /// Optional inline note.
    pub note: Option<String>,
    /// Accrual spread start date. `None` means no spreading.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end date (inclusive — the last day of the spread). `None` means no spreading.
    pub spread_until: Option<jiff::civil::Date>,
}

impl NewPosting {
    /// Creates a new [`NewPosting`].
    ///
    /// # Arguments
    ///
    /// * `account_id` - Account ID referencing an existing active account.
    /// * `amount` - Posting amount (positive = credit; negative = debit).
    /// * `note` - Optional inline note, or `None`.
    /// * `spread_from` - Accrual spread start date, or `None`.
    /// * `spread_until` - Accrual spread end date (inclusive — the last day of the spread), or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        account_id: impl Into<String>,
        amount: Amount,
        note: Option<impl Into<String>>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            amount,
            note: note.map(Into::into),
            spread_from,
            spread_until,
        }
    }
}

/// The user-supplied fields for a new transaction.
///
/// The backend assigns the `id` and appends the initial audit entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewTransaction {
    /// Transaction date.
    pub date: jiff::civil::Date,
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
    /// * `date` - Transaction date.
    /// * `payee` - Payee display name.
    /// * `status` - Transaction status.
    /// * `tags` - Tag paths attached to this transaction.
    /// * `postings` - All postings (must sum to zero per commodity).
    #[must_use]
    #[inline]
    pub fn new(
        date: jiff::civil::Date,
        payee: impl Into<String>,
        status: TxStatus,
        tags: Vec<String>,
        postings: Vec<NewPosting>,
    ) -> Self {
        Self {
            date,
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
/// income and expense totals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SparkPoint {
    /// X-axis label, e.g. `"apr"`, `"w03"`, `"Q2"`.
    pub label: String,
    /// Income for the bucket (positive).
    pub income: Amount,
    /// Expenses for the bucket (positive magnitude — plotted separately).
    pub expenses: Amount,
}

impl SparkPoint {
    /// Creates a new [`SparkPoint`].
    ///
    /// # Arguments
    ///
    /// * `label`    - X-axis label.
    /// * `income`   - Income amount.
    /// * `expenses` - Expenses amount (positive magnitude).
    #[must_use]
    #[inline]
    pub fn new(label: impl Into<String>, income: Amount, expenses: Amount) -> Self {
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
    /// Every calendar day.
    Daily,
    /// Every 7 days.
    Weekly,
    /// Every 14 days.
    Fortnightly,
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
    /// Financial quarter aligned to the configured financial year start (e.g. `start_month: 7,
    /// start_day: 1` for Australian FY).
    FinancialQuarter {
        /// 1-based start month of the financial year (1–12).
        start_month: u8,
        /// 1-based start day of the financial year (1–28).
        start_day: u8,
    },
}

impl Period {
    /// Returns the default number of buckets to display in a sparkline for this period.
    #[must_use]
    #[inline]
    pub fn default_sparkline_count(&self) -> u32 {
        match self {
            Self::Daily => 14,
            Self::Weekly | Self::Fortnightly => 8,
            Self::Monthly
            | Self::Quarterly
            | Self::CalendarYear
            | Self::FinancialYear { .. }
            | Self::FinancialQuarter { .. } => 6,
        }
    }

    /// Returns a compact human-readable label for this period.
    #[must_use]
    #[inline]
    pub fn label(&self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Fortnightly => "fortnightly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::CalendarYear => "calendar year",
            Self::FinancialYear { .. } => "financial year",
            Self::FinancialQuarter { .. } => "financial quarter",
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::Amount;

    #[test]
    fn new_posting_constructor_roundtrip() {
        let p = NewPosting::new(
            "acc-1",
            Amount::from_minor(-1_000, "AUD", 2),
            Some("test note"),
            None,
            None,
        );
        assert_eq!(p.account_id, "acc-1");
        assert_eq!(p.amount.value, rust_decimal::Decimal::new(-1_000, 2));
        assert_eq!(p.note.as_deref(), Some("test note"));
    }

    #[rstest]
    #[case(TxStatus::Pending)]
    #[case(TxStatus::Cleared)]
    fn new_transaction_serde_roundtrip(#[case] status: TxStatus) {
        let tx = NewTransaction::new(
            jiff::civil::Date::constant(2026, 5, 23),
            "Test Payee",
            status,
            vec![],
            vec![
                NewPosting::new(
                    "acc-a",
                    Amount::from_minor(-500, "AUD", 2),
                    None::<&str>,
                    None,
                    None,
                ),
                NewPosting::new(
                    "acc-b",
                    Amount::from_minor(500, "AUD", 2),
                    None::<&str>,
                    None,
                    None,
                ),
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
    fn period_daily_serde_roundtrip() {
        let p = Period::Daily;
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(json, r#"{"type":"daily"}"#);
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_fortnightly_serde_roundtrip() {
        let p = Period::Fortnightly;
        let json = serde_json::to_string(&p).expect("serialises");
        assert_eq!(json, r#"{"type":"fortnightly"}"#);
        let p2: Period = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(p, p2);
    }

    #[test]
    fn period_default_sparkline_count() {
        assert_eq!(Period::Daily.default_sparkline_count(), 14);
        assert_eq!(Period::Weekly.default_sparkline_count(), 8);
        assert_eq!(Period::Fortnightly.default_sparkline_count(), 8);
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

    #[rstest]
    #[case(TxStatus::Pending)]
    #[case(TxStatus::Cleared)]
    fn new_transaction_serde_roundtrip_with_tags(#[case] status: TxStatus) {
        let tx = NewTransaction::new(
            jiff::civil::Date::constant(2026, 5, 23),
            "Payee With Tags",
            status,
            vec!["category:groceries".to_owned(), "budget:food".to_owned()],
            vec![
                NewPosting::new(
                    "acc-a",
                    Amount::from_minor(-3_000, "AUD", 2),
                    None::<&str>,
                    None,
                    None,
                ),
                NewPosting::new(
                    "acc-b",
                    Amount::from_minor(3_000, "AUD", 2),
                    None::<&str>,
                    None,
                    None,
                ),
            ],
        );
        let json = serde_json::to_string(&tx).expect("serialises");
        let tx2: NewTransaction = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(tx, tx2);
        assert_eq!(tx2.tags.len(), 2);
        assert_eq!(
            tx2.tags.first().map(String::as_str),
            Some("category:groceries")
        );
    }

    #[test]
    fn spark_point_carries_amount() {
        let p = SparkPoint::new(
            "apr",
            Amount::from_minor(64_000, "AUD", 2),
            Amount::from_minor(12_300, "AUD", 2),
        );
        let json = serde_json::to_string(&p).expect("ser");
        let back: SparkPoint = serde_json::from_str(&json).expect("de");
        assert_eq!(p, back);
        assert_eq!(back.income.currency_code, "AUD");
    }

    #[test]
    fn audit_entry_serde_roundtrip() {
        let ts: jiff::Timestamp = "2026-06-20T09:04:00Z".parse().expect("valid timestamp");
        let e = AuditEntry::new(ts, "import", "from commbank-au.wasm@1.4.2");
        let json = serde_json::to_string(&e).expect("ser");
        let back: AuditEntry = serde_json::from_str(&json).expect("de");
        assert_eq!(e, back);
        assert_eq!(back.time, ts);
    }

    #[test]
    fn period_label_is_human_readable() {
        assert_eq!(Period::Weekly.label(), "weekly");
        assert_eq!(Period::Monthly.label(), "monthly");
        assert_eq!(
            Period::FinancialYear {
                start_month: 7,
                start_day: 1
            }
            .label(),
            "financial year"
        );
    }
}
