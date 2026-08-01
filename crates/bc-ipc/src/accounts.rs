//! Account and transaction types shared between Tauri backend and Leptos frontend.

use jiff::civil::Date;
use rust_decimal::Decimal;
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

// MARK: models conversions

#[cfg(feature = "models")]
impl From<bc_models::AccountType> for AccountType {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::AccountType and bc_ipc::AccountType are #[non_exhaustive]; \
                  the wildcard fallback to Asset is intentional for future unknown variants"
    )]
    fn from(value: bc_models::AccountType) -> Self {
        match value {
            bc_models::AccountType::Asset => Self::Asset,
            bc_models::AccountType::Liability => Self::Liability,
            bc_models::AccountType::Equity => Self::Equity,
            bc_models::AccountType::Income => Self::Income,
            bc_models::AccountType::Expense => Self::Expense,
            _ => Self::Asset,
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

/// Reconciliation status of a transaction, matching `bc_models::Reconciliation` 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Reconciliation {
    /// Imported but not reviewed by the user.
    Unreconciled,
    /// Flagged for review (user attention needed).
    Flagged,
    /// Reviewed and confirmed correct.
    Reconciled,
}

impl Reconciliation {
    /// Returns the lowercase display label for this reconciliation state.
    ///
    /// # Example
    ///
    /// ```
    /// # use bc_ipc::Reconciliation;
    /// assert_eq!(Reconciliation::Unreconciled.label(), "unreconciled");
    /// assert_eq!(Reconciliation::Flagged.label(), "flagged");
    /// assert_eq!(Reconciliation::Reconciled.label(), "reconciled");
    /// ```
    #[must_use]
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::Unreconciled => "unreconciled",
            Self::Flagged => "flagged",
            Self::Reconciled => "reconciled",
        }
    }
}

// MARK: models conversions

#[cfg(feature = "models")]
impl From<bc_models::Reconciliation> for Reconciliation {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "bc_models::Reconciliation is #[non_exhaustive]; the wildcard fallback is intentional"
    )]
    fn from(value: bc_models::Reconciliation) -> Self {
        match value {
            bc_models::Reconciliation::Unreconciled => Self::Unreconciled,
            bc_models::Reconciliation::Flagged => Self::Flagged,
            bc_models::Reconciliation::Reconciled => Self::Reconciled,
            _ => Self::Unreconciled,
        }
    }
}

#[cfg(feature = "models")]
impl From<Reconciliation> for bc_models::Reconciliation {
    #[inline]
    fn from(value: Reconciliation) -> Self {
        match value {
            Reconciliation::Unreconciled => Self::Unreconciled,
            Reconciliation::Flagged => Self::Flagged,
            Reconciliation::Reconciled => Self::Reconciled,
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

/// The amount a posting carries, stored or derived.
///
/// A leg whose amount the source document elides has no stored amount; it
/// absorbs its transaction's residual instead, derived on read and never
/// persisted (`docs/DESIGN.md` §4.4). This type keeps the two cases distinct so
/// a derived value can never be mistaken for a stated one — in particular, no
/// variant here can be sent back through [`EditPosting`], whose `amount` stays
/// `Option<Amount>`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum PostingAmount {
    /// The source document stated this amount, and it is what is persisted.
    Stored(Amount),
    /// Derived from the transaction's other legs; nothing is persisted. One
    /// entry per commodity, empty when the residual is zero.
    Derived(Vec<Amount>),
    /// Two or more legs are elided, so the residual — though real — is not
    /// attributable to any single leg.
    Ambiguous,
}

impl PostingAmount {
    /// Returns the stored amount, or `None` when this posting's amount is derived.
    ///
    /// # Returns
    ///
    /// The stated [`Amount`], or `None`.
    #[must_use]
    #[inline]
    pub fn stored(&self) -> Option<&Amount> {
        match *self {
            Self::Stored(ref amount) => Some(amount),
            Self::Derived(_) | Self::Ambiguous => None,
        }
    }

    /// Returns `true` when the source document elided this leg's amount.
    ///
    /// # Returns
    ///
    /// `true` for [`Self::Derived`] and [`Self::Ambiguous`].
    #[must_use]
    #[inline]
    pub fn is_elided(&self) -> bool {
        !matches!(*self, Self::Stored(_))
    }

    /// Returns the single amount to display, stored or derived.
    ///
    /// A multi-commodity residual has no single display amount and yields
    /// `None`, as does [`Self::Ambiguous`].
    ///
    /// # Returns
    ///
    /// The [`Amount`] to render, or `None`.
    #[must_use]
    #[inline]
    pub fn display_amount(&self) -> Option<&Amount> {
        match *self {
            Self::Stored(ref amount) => Some(amount),
            Self::Derived(ref amounts) => match amounts.as_slice() {
                [single] => Some(single),
                _ => None,
            },
            Self::Ambiguous => None,
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
    /// Posting amount — stored, or derived when the source document elided it.
    pub amount: PostingAmount,
    /// Optional inline comment shown in the TOML view.
    pub note: Option<String>,
    /// Resolved tag paths attached to this posting (colon-joined; includes inherited transaction tags).
    pub tags: Vec<String>,
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
    /// * `amount` - Posting amount, stored or derived.
    /// * `note` - Optional inline comment, or `None`.
    /// * `tags` - Tag IDs attached to this posting.
    /// * `spread_from` - Accrual spread start date, or `None`.
    /// * `spread_until` - Accrual spread end date (inclusive — the last day of the spread), or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        account: AccountRef,
        amount: PostingAmount,
        note: Option<impl Into<String>>,
        tags: Vec<String>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            id: id.into(),
            account,
            amount,
            note: note.map(Into::into),
            tags,
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
    /// Free-text description (raw imported narration).
    pub description: String,
    /// User's free-text note, distinct from the imported `description`.
    pub note: Option<String>,
    /// Extra named dates attached to this transaction (e.g. `("effective", 2026-04-01)`).
    pub extra_dates: Vec<(String, jiff::civil::Date)>,
    /// Reconciliation status.
    pub reconciliation: Reconciliation,
    /// Tag paths attached to this transaction (colon-joined).
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
    /// * `description` - Free-text description (raw imported narration).
    /// * `note` - User free-text note, or `None`.
    /// * `extra_dates` - Extra named dates (label + date pairs).
    /// * `reconciliation` - Reconciliation status.
    /// * `tags` - Tag paths (colon-joined).
    /// * `postings` - All postings (must sum to zero).
    /// * `audit` - Audit trail entries.
    #[must_use]
    #[inline]
    #[expect(
        clippy::too_many_arguments,
        reason = "domain record with many required fields"
    )]
    pub fn new(
        id: impl Into<String>,
        date: jiff::civil::Date,
        payee: impl Into<String>,
        description: impl Into<String>,
        note: Option<impl Into<String>>,
        extra_dates: Vec<(String, jiff::civil::Date)>,
        reconciliation: Reconciliation,
        tags: Vec<String>,
        postings: Vec<Posting>,
        audit: Vec<AuditEntry>,
    ) -> Self {
        Self {
            id: id.into(),
            date,
            payee: payee.into(),
            description: description.into(),
            note: note.map(Into::into),
            extra_dates,
            reconciliation,
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
    /// Posting amount. `None` when the amount should be elided (inferred to balance).
    pub amount: Option<Amount>,
    /// Optional inline note.
    pub note: Option<String>,
    /// Tag paths to attach to this posting (must reference existing tags).
    pub tags: Vec<String>,
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
    /// * `amount` - Posting amount, or `None` to elide (inferred to balance).
    /// * `note` - Optional inline note, or `None`.
    /// * `tags` - Tag IDs to attach to this posting.
    /// * `spread_from` - Accrual spread start date, or `None`.
    /// * `spread_until` - Accrual spread end date (inclusive — the last day of the spread), or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        account_id: impl Into<String>,
        amount: Option<Amount>,
        note: Option<impl Into<String>>,
        tags: Vec<String>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            amount,
            note: note.map(Into::into),
            tags,
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
    /// Free-text description (raw narration). Empty string if not provided.
    pub description: String,
    /// User's free-text note. `None` if not provided.
    pub note: Option<String>,
    /// Reconciliation status.
    pub reconciliation: Reconciliation,
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
    /// * `description` - Free-text description (raw narration).
    /// * `note` - User's free-text note, or `None`.
    /// * `reconciliation` - Reconciliation status.
    /// * `tags` - Tag paths attached to this transaction.
    /// * `postings` - All postings (must sum to zero per commodity).
    #[must_use]
    #[inline]
    pub fn new(
        date: jiff::civil::Date,
        payee: impl Into<String>,
        description: impl Into<String>,
        note: Option<impl Into<String>>,
        reconciliation: Reconciliation,
        tags: Vec<String>,
        postings: Vec<NewPosting>,
    ) -> Self {
        Self {
            date,
            payee: payee.into(),
            description: description.into(),
            note: note.map(Into::into),
            reconciliation,
            tags,
            postings,
        }
    }
}

/// A single posting in an [`EditTransaction`].
///
/// `id` identifies an existing posting to update in place; `None` adds a new leg.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EditPosting {
    /// Existing posting ID, or `None` for a newly added leg.
    pub id: Option<String>,
    /// Account this posting hits.
    pub account_id: String,
    /// Posting amount, or `None` if elided (inferred to balance).
    pub amount: Option<Amount>,
    /// Free-text note, or `None`.
    pub note: Option<String>,
    /// Tag paths to attach (colon-joined; resolved to existing tags on save).
    pub tags: Vec<String>,
    /// Accrual spread start (inclusive), or `None`.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end (exclusive), or `None`.
    pub spread_until: Option<jiff::civil::Date>,
}

impl EditPosting {
    /// Creates a new [`EditPosting`].
    ///
    /// # Arguments
    ///
    /// * `id` - Existing posting ID to update in place, or `None` to add a new leg.
    /// * `account_id` - Account this posting hits.
    /// * `amount` - Posting amount, or `None` if elided (inferred to balance).
    /// * `note` - Free-text note, or `None`.
    /// * `tags` - Tag paths to attach (resolved to existing tags on save).
    /// * `spread_from` - Accrual spread start, or `None`.
    /// * `spread_until` - Accrual spread end, or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        id: Option<String>,
        account_id: impl Into<String>,
        amount: Option<Amount>,
        note: Option<String>,
        tags: Vec<String>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            id,
            account_id: account_id.into(),
            amount,
            note,
            tags,
            spread_from,
            spread_until,
        }
    }
}

/// The desired state for editing an existing transaction.
///
/// The backend diffs this against the stored state to record decomposed
/// semantic events, then rewrites the projection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct EditTransaction {
    /// ID of the transaction being edited.
    pub id: String,
    /// Transaction date.
    pub date: jiff::civil::Date,
    /// Payee display name.
    pub payee: String,
    /// Free-text description.
    pub description: String,
    /// User's free-text note, or `None`.
    pub note: Option<String>,
    /// Reconciliation status (read-only in the editor, echoed back unchanged).
    pub reconciliation: Reconciliation,
    /// Transaction-level tag paths (resolved to existing tags on save).
    pub tags: Vec<String>,
    /// All postings in display order.
    pub postings: Vec<EditPosting>,
    /// Extra named dates (label + date pairs); replaces the stored set on save.
    pub extra_dates: Vec<(String, jiff::civil::Date)>,
}

impl EditTransaction {
    /// Creates a new [`EditTransaction`] describing the desired post-edit state.
    ///
    /// # Arguments
    ///
    /// * `id` - ID of the transaction being edited.
    /// * `date` - Transaction date.
    /// * `payee` - Payee display name.
    /// * `description` - Free-text description.
    /// * `note` - User's free-text note, or `None`.
    /// * `reconciliation` - Reconciliation status (echoed back unchanged).
    /// * `tags` - Transaction-level tag paths (resolved to existing tags on save).
    /// * `postings` - All postings in display order.
    /// * `extra_dates` - Extra named dates (label + date pairs); replaces the stored set on save.
    #[must_use]
    #[inline]
    #[expect(
        clippy::too_many_arguments,
        reason = "mirrors the public DTO field set; a builder would add ceremony for a flat data carrier"
    )]
    pub fn new(
        id: impl Into<String>,
        date: jiff::civil::Date,
        payee: impl Into<String>,
        description: impl Into<String>,
        note: Option<String>,
        reconciliation: Reconciliation,
        tags: Vec<String>,
        postings: Vec<EditPosting>,
        extra_dates: Vec<(String, jiff::civil::Date)>,
    ) -> Self {
        Self {
            id: id.into(),
            date,
            payee: payee.into(),
            description: description.into(),
            note,
            reconciliation,
            tags,
            postings,
            extra_dates,
        }
    }
}

/// Magnitude predicate for the amount filter dimension.
///
/// Compares the absolute value of a posting's amount against an inclusive
/// `[min, max]` range. Either bound may be omitted. When `commodity` is set,
/// only postings in that currency are considered.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AmountFilter {
    /// Inclusive lower bound on the magnitude, if any.
    pub min: Option<Decimal>,
    /// Inclusive upper bound on the magnitude, if any.
    pub max: Option<Decimal>,
    /// Restrict to a single currency code when set.
    pub commodity: Option<String>,
}

/// A global, structured transaction filter built in the UI and applied server-side.
///
/// All fields are optional/empty by default; an empty filter matches everything.
/// Dimensions combine with AND; the repeatable dimensions (`accounts`, `tags`)
/// OR within themselves. `text` matches a case-insensitive substring against
/// either the payee or the narration (description).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Filter {
    /// Inclusive lower bound on the transaction date.
    pub date_from: Option<Date>,
    /// Exclusive upper bound on the transaction date.
    pub date_until: Option<Date>,
    /// Account ids; each matches its subtree; multiple entries union (OR).
    pub accounts: Vec<String>,
    /// Tag ids; multiple entries union (OR).
    pub tags: Vec<String>,
    /// Case-insensitive substring over payee OR narration.
    pub text: Option<String>,
    /// Magnitude predicate; a transaction matches if any posting qualifies.
    pub amount: Option<AmountFilter>,
    /// Exact reconciliation status.
    pub reconciliation: Option<Reconciliation>,
}

/// A matched transaction plus the ids of the legs that satisfied the
/// posting-scoped filter predicates (all legs when the match was
/// transaction-scoped). The whole transaction is always returned; consumers use
/// the matched set to visually distinguish the non-matching legs (e.g. dimming
/// them), never to prune legs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FilteredTransaction {
    /// The whole matched transaction (never pruned server-side).
    pub transaction: Transaction,
    /// Posting ids of the legs that matched the posting-scoped predicates.
    pub matched_postings: Vec<String>,
}

impl FilteredTransaction {
    /// Creates a new [`FilteredTransaction`].
    ///
    /// # Arguments
    ///
    /// * `transaction` - The whole matched transaction (never pruned server-side).
    /// * `matched_postings` - Posting ids of the legs that matched the posting-scoped predicates.
    #[must_use]
    #[inline]
    pub fn new(transaction: Transaction, matched_postings: Vec<String>) -> Self {
        Self {
            transaction,
            matched_postings,
        }
    }
}

/// Windowed account statistics for the dashboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountStats {
    /// In-window inflow (money entering the account).
    pub income: Amount,
    /// In-window outflow, as a positive magnitude.
    pub expenses: Amount,
    /// `income − expenses` (signed).
    pub net: Amount,
    /// Running balance at the window start.
    pub opening_balance: Amount,
    /// Running balance at the window end (period-end closing balance).
    pub closing_balance: Amount,
    /// Count of distinct in-window transactions involving the account
    /// (commodity-agnostic; matches the register's row count).
    pub tx_count: u32,
    /// Real (unfiltered) running balance at the window start; `Some` only when a
    /// filter was active, for a muted reference alongside the filtered figure.
    pub real_opening: Option<Amount>,
    /// Real (unfiltered) running balance at the window end; `Some` only when a
    /// filter was active.
    pub real_closing: Option<Amount>,
}

impl AccountStats {
    /// Creates a new [`AccountStats`].
    ///
    /// # Arguments
    ///
    /// * `income` - In-window inflow.
    /// * `expenses` - In-window outflow magnitude.
    /// * `net` - Signed net movement.
    /// * `opening_balance` - Balance at window start.
    /// * `closing_balance` - Balance at window end.
    /// * `tx_count` - Count of distinct in-window transactions (matches the register).
    #[must_use]
    #[inline]
    pub fn new(
        income: Amount,
        expenses: Amount,
        net: Amount,
        opening_balance: Amount,
        closing_balance: Amount,
        tx_count: u32,
    ) -> Self {
        Self {
            income,
            expenses,
            net,
            opening_balance,
            closing_balance,
            tx_count,
            real_opening: None,
            real_closing: None,
        }
    }

    /// Attaches the real (unfiltered) opening and closing balances.
    ///
    /// # Arguments
    ///
    /// * `opening` - Real running balance at the window start.
    /// * `closing` - Real running balance at the window end.
    ///
    /// # Returns
    ///
    /// `self` with `real_opening`/`real_closing` populated.
    #[must_use]
    #[inline]
    pub fn with_real_balances(mut self, opening: Amount, closing: Amount) -> Self {
        self.real_opening = Some(opening);
        self.real_closing = Some(closing);
        self
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

/// Chooses a sparkline bucketing for the overarching span `[start, end)`.
///
/// Picks the finest bucket [`Period`] that yields a readable count for the
/// span, then `count = ceil(span_days / nominal_bucket_days)` clamped to `>= 1`.
/// Edge buckets may be partial. Nominal lengths: Daily = 1, Weekly = 7,
/// Monthly = 31, Quarterly = 92, `CalendarYear` = 365 — chosen so the
/// `PeriodNav` spans reproduce the established sparkline densities exactly
/// (the widest nav span is 366 days, which stays on Monthly) while very wide
/// filter spans degrade to coarser buckets instead of hundreds of bars.
///
/// # Arguments
///
/// * `start` - Inclusive span start.
/// * `end` - Exclusive span end.
///
/// # Returns
///
/// The `(bucket_period, count)` to fetch.
#[must_use]
#[inline]
pub fn sparkline_bucketing_for(start: Date, end: Date) -> (Period, u32) {
    let span_days = start
        .until(end)
        .map_or(0, |span| u64::try_from(span.get_days()).unwrap_or(0));
    let (bucket, nominal) = if span_days <= 21 {
        (Period::Daily, 1_u64)
    } else if span_days <= 120 {
        (Period::Weekly, 7_u64)
    } else if span_days <= 730 {
        (Period::Monthly, 31_u64)
    } else if span_days <= 1830 {
        (Period::Quarterly, 92_u64)
    } else {
        (Period::CalendarYear, 365_u64)
    };
    let count = span_days.div_ceil(nominal).max(1);
    (bucket, u32::try_from(count).unwrap_or(u32::MAX))
}

// MARK: models conversions

#[cfg(feature = "models")]
impl From<&bc_models::Period> for Period {
    #[inline]
    fn from(value: &bc_models::Period) -> Self {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "bc_models::Period is #[non_exhaustive]; unknown future variants fall back to Monthly"
        )]
        match value {
            bc_models::Period::Weekly => Self::Weekly,
            bc_models::Period::Fortnightly { .. } => Self::Fortnightly,
            bc_models::Period::Monthly => Self::Monthly,
            bc_models::Period::Quarterly => Self::Quarterly,
            bc_models::Period::CalendarYear => Self::CalendarYear,
            bc_models::Period::FinancialYear {
                start_month,
                start_day,
            } => Self::FinancialYear {
                start_month: *start_month,
                start_day: *start_day,
            },
            bc_models::Period::FinancialQuarter {
                start_month,
                start_day,
            } => Self::FinancialQuarter {
                start_month: *start_month,
                start_day: *start_day,
            },
            bc_models::Period::Custom {
                days: Some(1),
                weeks: None,
                months: None,
            } => Self::Daily,
            other => {
                tracing::warn!(
                    ?other,
                    "Period has no bc_ipc equivalent; defaulting to monthly"
                );
                Self::Monthly
            }
        }
    }
}

#[cfg(feature = "models")]
impl From<Period> for bc_models::Period {
    #[inline]
    fn from(value: Period) -> Self {
        match value {
            Period::Daily => Self::Custom {
                days: Some(1),
                weeks: None,
                months: None,
            },
            Period::Weekly => Self::Weekly,
            Period::Fortnightly => {
                // TODO: use the globally-configured fortnightly anchor (Milestone 5 config).
                // 2026-01-05 (Monday) is a placeholder; any user whose pay cycle does not
                // align to this anchor will see misaligned fortnightly buckets.
                tracing::warn!(
                    anchor = "2026-01-05",
                    "fortnightly anchor is hardcoded; user pay cycles may not align"
                );
                #[expect(
                    clippy::expect_used,
                    reason = "2026-01-05 is a valid date; this can never panic"
                )]
                let anchor =
                    jiff::civil::Date::new(2026, 1, 5).expect("2026-01-05 is a valid date");
                Self::Fortnightly { anchor }
            }
            Period::Monthly => Self::Monthly,
            Period::Quarterly => Self::Quarterly,
            Period::CalendarYear => Self::CalendarYear,
            Period::FinancialYear {
                start_month,
                start_day,
            } => Self::FinancialYear {
                start_month,
                start_day,
            },
            Period::FinancialQuarter {
                start_month,
                start_day,
            } => Self::FinancialQuarter {
                start_month,
                start_day,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use jiff::Span;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;
    use crate::Amount;

    #[test]
    fn new_posting_constructor_roundtrip() {
        let p = NewPosting::new(
            "acc-1",
            Some(Amount::new(Decimal::new(-1_000, 2), "AUD")),
            Some("test note"),
            vec![],
            None,
            None,
        );
        assert_eq!(p.account_id, "acc-1");
        assert_eq!(
            p.amount.as_ref().map(|a| a.value),
            Some(rust_decimal::Decimal::new(-1_000, 2))
        );
        assert_eq!(p.note.as_deref(), Some("test note"));
    }

    #[rstest]
    #[case(Reconciliation::Unreconciled)]
    #[case(Reconciliation::Reconciled)]
    fn new_transaction_serde_roundtrip(#[case] reconciliation: Reconciliation) {
        let tx = NewTransaction::new(
            jiff::civil::Date::constant(2026, 5, 23),
            "Test Payee",
            "",
            None::<&str>,
            reconciliation,
            vec![],
            vec![
                NewPosting::new(
                    "acc-a",
                    Some(Amount::new(Decimal::new(-500, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
                NewPosting::new(
                    "acc-b",
                    Some(Amount::new(Decimal::new(500, 2), "AUD")),
                    None::<&str>,
                    vec![],
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
    fn transaction_serde_roundtrip_with_new_fields() {
        let posting = Posting::new(
            "posting-1",
            AccountRef::new("acc-1", "Assets :: Checking"),
            PostingAmount::Ambiguous,
            Some("posting note"),
            vec!["tag-abc".to_owned()],
            None,
            None,
        );
        let tx = Transaction::new(
            "tx-1",
            jiff::civil::Date::constant(2026, 6, 1),
            "Test Payee",
            "raw narration",
            Some("user note"),
            vec![(
                "effective".to_owned(),
                jiff::civil::Date::constant(2026, 6, 15),
            )],
            Reconciliation::Flagged,
            vec![],
            vec![posting],
            vec![],
        );
        let json = serde_json::to_string(&tx).expect("serialises");
        let tx2: Transaction = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(tx, tx2);
        assert_eq!(tx2.reconciliation, Reconciliation::Flagged);
        assert_eq!(tx2.description, "raw narration");
        assert_eq!(tx2.note.as_deref(), Some("user note"));
        assert_eq!(tx2.extra_dates.len(), 1);
        assert_eq!(
            tx2.extra_dates.first().map(|(l, _)| l.as_str()),
            Some("effective")
        );
        assert!(matches!(
            tx2.postings.first().map(|p| &p.amount),
            Some(PostingAmount::Ambiguous)
        ));
        assert_eq!(
            tx2.postings.first().map(|p| p.tags.as_slice()),
            Some(["tag-abc".to_owned()].as_slice())
        );
        assert_eq!(
            tx2.postings.first().and_then(|p| p.note.as_deref()),
            Some("posting note")
        );
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

    #[rstest]
    /* Zero/negative span clamps count to ≥ 1. */
    #[case(0, Period::Daily, 1)]
    /* PeriodNav densities: Daily/Weekly → 14 days, Fortnightly → 8 weeks,
     * Monthly → 13 weeks, Quarterly → 6 months, yearly → 12 months. These must
     * not shift when the ladder gains tiers. */
    #[case(14, Period::Daily, 14)]
    #[case(56, Period::Weekly, 8)]
    #[case(91, Period::Weekly, 13)]
    #[case(181, Period::Monthly, 6)]
    #[case(184, Period::Monthly, 6)]
    #[case(365, Period::Monthly, 12)]
    #[case(366, Period::Monthly, 12)]
    /* Arbitrary filter range inside the weekly tier. */
    #[case(45, Period::Weekly, 7)]
    /* Tier boundaries and the first day past each. */
    #[case(21, Period::Daily, 21)]
    #[case(22, Period::Weekly, 4)]
    #[case(120, Period::Weekly, 18)]
    #[case(121, Period::Monthly, 4)]
    #[case(730, Period::Monthly, 24)]
    #[case(731, Period::Quarterly, 8)]
    #[case(1830, Period::Quarterly, 20)]
    #[case(1831, Period::CalendarYear, 6)]
    fn sparkline_bucketing_for_spans(
        #[case] span_days: i64,
        #[case] expected_bucket: Period,
        #[case] expected_count: u32,
    ) {
        let start = date(2025, 1, 1);
        let end = start.saturating_add(Span::new().days(span_days));

        assert_eq!(
            super::sparkline_bucketing_for(start, end),
            (expected_bucket, expected_count)
        );
    }

    #[rstest]
    #[case(Reconciliation::Unreconciled)]
    #[case(Reconciliation::Reconciled)]
    fn new_transaction_serde_roundtrip_with_tags(#[case] reconciliation: Reconciliation) {
        let tx = NewTransaction::new(
            jiff::civil::Date::constant(2026, 5, 23),
            "Payee With Tags",
            "",
            None::<&str>,
            reconciliation,
            vec!["category:groceries".to_owned(), "budget:food".to_owned()],
            vec![
                NewPosting::new(
                    "acc-a",
                    Some(Amount::new(Decimal::new(-3_000, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
                NewPosting::new(
                    "acc-b",
                    Some(Amount::new(Decimal::new(3_000, 2), "AUD")),
                    None::<&str>,
                    vec![],
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
            Amount::new(Decimal::new(64_000, 2), "AUD"),
            Amount::new(Decimal::new(12_300, 2), "AUD"),
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

    #[test]
    fn reconciliation_labels() {
        assert_eq!(Reconciliation::Unreconciled.label(), "unreconciled");
        assert_eq!(Reconciliation::Flagged.label(), "flagged");
        assert_eq!(Reconciliation::Reconciled.label(), "reconciled");
    }

    #[test]
    fn edit_transaction_serde_roundtrip() {
        let dto = EditTransaction {
            id: "tx-1".to_owned(),
            date: "2026-04-30".parse().expect("date"),
            payee: "Atlassian".to_owned(),
            description: "salary".to_owned(),
            note: None,
            reconciliation: Reconciliation::Unreconciled,
            tags: vec!["work".to_owned()],
            postings: vec![EditPosting {
                id: Some("p-1".to_owned()),
                account_id: "acc-1".to_owned(),
                amount: None,
                note: None,
                tags: vec![],
                spread_from: None,
                spread_until: None,
            }],
            extra_dates: vec![],
        };
        let json = serde_json::to_string(&dto).expect("ser");
        let back: EditTransaction = serde_json::from_str(&json).expect("de");
        assert_eq!(dto, back);
    }

    #[test]
    fn filtered_transaction_round_trips() {
        let tx = Transaction::new(
            "tx-1",
            jiff::civil::Date::constant(2026, 6, 1),
            "Test Payee",
            "raw narration",
            None::<&str>,
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            vec![],
            vec![],
        );
        let ft = FilteredTransaction {
            transaction: tx,
            matched_postings: vec!["p-1".to_owned(), "p-2".to_owned()],
        };
        let json = serde_json::to_string(&ft).expect("serialize");
        let back: FilteredTransaction = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, ft);
    }

    #[test]
    fn filter_default_is_empty_and_round_trips() {
        let f = Filter::default();
        assert_eq!(f.accounts, Vec::<String>::new());
        assert_eq!(f.tags, Vec::<String>::new());
        assert_eq!(f.text, None);
        assert_eq!(f.amount, None);
        assert_eq!(f.reconciliation, None);

        let json = serde_json::to_string(&f).expect("serialize");
        let back: Filter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, f);
    }

    #[test]
    fn filter_with_amount_round_trips() {
        let f = Filter {
            date_from: Some(date(2026, 1, 1)),
            date_until: Some(date(2026, 2, 1)),
            accounts: vec!["acc-1".to_owned()],
            tags: vec!["tag-1".to_owned(), "tag-2".to_owned()],
            text: Some("amazon".to_owned()),
            amount: Some(AmountFilter {
                min: Some(Decimal::new(100, 0)),
                max: Some(Decimal::new(200, 0)),
                commodity: Some("AUD".to_owned()),
            }),
            reconciliation: Some(Reconciliation::Reconciled),
        };
        let json = serde_json::to_string(&f).expect("serialize");
        let back: Filter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, f);
    }

    #[test]
    fn edit_transaction_new_builds_expected() {
        use jiff::civil::Date;

        let p = EditPosting::new(
            Some("p-1".to_owned()),
            "acct-checking",
            Some(Amount::new(rust_decimal::Decimal::new(-5_000, 2), "AUD")),
            None,
            vec!["tag-1".to_owned()],
            None,
            None,
        );
        let tx = EditTransaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "Coles",
            "weekly shop",
            Some("note".to_owned()),
            Reconciliation::Unreconciled,
            vec!["work".to_owned()],
            vec![p.clone()],
            vec![],
        );

        assert_eq!(tx.id, "tx-1");
        assert_eq!(tx.date, Date::constant(2026, 4, 30));
        assert_eq!(tx.payee, "Coles");
        assert_eq!(tx.note.as_deref(), Some("note"));
        assert_eq!(tx.postings, vec![p]);
    }

    #[test]
    fn account_stats_real_balances_default_none_and_set() {
        let z = Amount::new(rust_decimal::Decimal::ZERO, "AUD");
        let stats = AccountStats::new(z.clone(), z.clone(), z.clone(), z.clone(), z.clone(), 0);
        assert_eq!(stats.real_opening, None);
        assert_eq!(stats.real_closing, None);

        let real = Amount::new(rust_decimal::Decimal::new(4210, 0), "AUD");
        let stats_with_real = stats.with_real_balances(real.clone(), real.clone());
        assert_eq!(stats_with_real.real_opening, Some(real.clone()));
        assert_eq!(stats_with_real.real_closing, Some(real));
    }
}

#[cfg(test)]
#[cfg(feature = "models")]
mod models_tests {
    use pretty_assertions::assert_eq;

    use super::Period;

    #[test]
    fn model_period_into_ipc_maps_known_variants() {
        assert_eq!(Period::from(&bc_models::Period::Weekly), Period::Weekly);
        assert_eq!(Period::from(&bc_models::Period::Monthly), Period::Monthly);
        assert_eq!(
            Period::from(&bc_models::Period::Custom {
                days: Some(1),
                weeks: None,
                months: None
            }),
            Period::Daily
        );
    }
}
