//! Account and transaction types shared between Tauri backend and Leptos frontend.

use bc_period::Period;
use jiff::civil::Date;
use serde::Deserialize;
use serde::Serialize;

use crate::MetaEntryDto;
use crate::TagInfo;
use crate::money::Amount;
use crate::quote::Cost;
use crate::quote::Quote;

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
    /// Per-commodity totals over this account and every descendant, own
    /// postings included. The first entry is the account's own default
    /// commodity when it has one, then first-seen order. Empty when the
    /// subtree holds no postings.
    pub rollup: Vec<Amount>,
    /// `Some(parent_id)` for child accounts, `None` for top-level groups.
    pub parent_id: Option<String>,
    /// Account type (asset, liability, equity, income, or expense).
    pub account_type: AccountType,
    /// Tags attached to this account.
    pub tags: Vec<TagInfo>,
    /// Date the account was opened, or `None` if not recorded.
    pub opened_on: Option<Date>,
    /// Date the account was closed, or `None` if still open.
    pub closed_on: Option<Date>,
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
    /// * `tags` - Tags attached to this account.
    /// * `opened_on` - Date the account was opened, or `None` if not recorded.
    /// * `closed_on` - Date the account was closed, or `None` if still open.
    #[must_use]
    #[inline]
    #[expect(
        clippy::too_many_arguments,
        reason = "domain record with many required fields"
    )]
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        mask: Option<impl Into<String>>,
        balance: Option<Amount>,
        parent_id: Option<impl Into<String>>,
        account_type: AccountType,
        tags: Vec<TagInfo>,
        opened_on: Option<Date>,
        closed_on: Option<Date>,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            mask: mask.map(Into::into),
            balance,
            rollup: Vec::new(),
            parent_id: parent_id.map(Into::into),
            account_type,
            tags,
            opened_on,
            closed_on,
        }
    }

    /// Returns this node with `rollup` replaced.
    ///
    /// # Arguments
    ///
    /// * `rollup` - Per-commodity subtree totals.
    #[must_use]
    #[inline]
    pub fn with_rollup(mut self, rollup: Vec<Amount>) -> Self {
        self.rollup = rollup;
        self
    }
}

// MARK: Account management

/// How an account is maintained, matching `bc_models::AccountKind` 1:1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AccountKindDto {
    /// Reconciles against a bank, card or brokerage statement.
    DepositAccount,
    /// A real asset valued by hand, with no statement.
    ManualAsset,
    /// Money a third party owes.
    Receivable,
    /// A subdivision of its parent's balance.
    VirtualAllocation,
    /// An organisational node that holds no postings of its own.
    Group,
}

#[cfg(feature = "models")]
impl From<bc_models::AccountKind> for AccountKindDto {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "bc_models::AccountKind is #[non_exhaustive]; the wildcard fallback to the \
                  default kind covers future variants"
    )]
    fn from(value: bc_models::AccountKind) -> Self {
        match value {
            bc_models::AccountKind::DepositAccount => Self::DepositAccount,
            bc_models::AccountKind::ManualAsset => Self::ManualAsset,
            bc_models::AccountKind::Receivable => Self::Receivable,
            bc_models::AccountKind::VirtualAllocation => Self::VirtualAllocation,
            bc_models::AccountKind::Group => Self::Group,
            _ => Self::DepositAccount,
        }
    }
}

#[cfg(feature = "models")]
impl From<AccountKindDto> for bc_models::AccountKind {
    #[inline]
    fn from(value: AccountKindDto) -> Self {
        match value {
            AccountKindDto::DepositAccount => Self::DepositAccount,
            AccountKindDto::ManualAsset => Self::ManualAsset,
            AccountKindDto::Receivable => Self::Receivable,
            AccountKindDto::VirtualAllocation => Self::VirtualAllocation,
            AccountKindDto::Group => Self::Group,
        }
    }
}

#[cfg(feature = "models")]
impl From<AccountType> for bc_models::AccountType {
    #[inline]
    fn from(value: AccountType) -> Self {
        match value {
            AccountType::Asset => Self::Asset,
            AccountType::Liability => Self::Liability,
            AccountType::Equity => Self::Equity,
            AccountType::Income => Self::Income,
            AccountType::Expense => Self::Expense,
        }
    }
}

/// A manual asset's depreciation method, matching
/// `bc_models::DepreciationPolicy` 1:1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DepreciationPolicyDto {
    /// No depreciation.
    None,
    /// Fixed annual depreciation: acquisition cost times `annual_rate`.
    StraightLine {
        /// Annual rate as a fraction, e.g. `0.25` for 25 % a year.
        annual_rate: rust_decimal::Decimal,
    },
    /// Reducing-balance depreciation: book value times `annual_rate`.
    DecliningBalance {
        /// Annual rate as a fraction, e.g. `0.40` for 40 % a year.
        annual_rate: rust_decimal::Decimal,
    },
}

#[cfg(feature = "models")]
impl From<&bc_models::DepreciationPolicy> for DepreciationPolicyDto {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "bc_models::DepreciationPolicy is #[non_exhaustive]; the wildcard fallback \
                  covers future variants"
    )]
    fn from(value: &bc_models::DepreciationPolicy) -> Self {
        match *value {
            bc_models::DepreciationPolicy::None => Self::None,
            bc_models::DepreciationPolicy::StraightLine { annual_rate } => {
                Self::StraightLine { annual_rate }
            }
            bc_models::DepreciationPolicy::DecliningBalance { annual_rate } => {
                Self::DecliningBalance { annual_rate }
            }
            _ => Self::None,
        }
    }
}

#[cfg(feature = "models")]
impl From<&DepreciationPolicyDto> for bc_models::DepreciationPolicy {
    #[inline]
    fn from(value: &DepreciationPolicyDto) -> Self {
        match *value {
            DepreciationPolicyDto::None => Self::None,
            DepreciationPolicyDto::StraightLine { annual_rate } => {
                Self::StraightLine { annual_rate }
            }
            DepreciationPolicyDto::DecliningBalance { annual_rate } => {
                Self::DecliningBalance { annual_rate }
            }
        }
    }
}

/// A manual asset's acquisition record.
///
/// On a write it replaces the whole record: a `None` field clears it.
#[derive(bon::Builder, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AcquisitionDto {
    /// Date the asset was acquired.
    pub date: Option<Date>,
    /// Cost of acquisition, in the account's default commodity.
    pub cost: Option<rust_decimal::Decimal>,
    /// Depreciation method.
    pub depreciation_policy: Option<DepreciationPolicyDto>,
}

/// One account with every field the account-management page edits.
///
/// Unlike [`AccountNode`] it carries archived accounts, and it carries
/// `event_count`, the base a later `update_account` call hands back.
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[builder(on(String, into))]
#[non_exhaustive]
pub struct AccountDetail {
    /// Stable account ID.
    pub id: String,
    /// Display name, the last path segment.
    pub name: String,
    /// Display path from the root, joined by `" :: "`, e.g.
    /// `Assets :: Bank :: Checking`; the form `AccountRef.name` uses.
    pub path: String,
    /// Parent account ID, or `None` for a root.
    pub parent_id: Option<String>,
    /// Account type.
    pub account_type: AccountType,
    /// How the account is maintained.
    pub kind: AccountKindDto,
    /// Free-text description.
    pub description: Option<String>,
    /// Allowed commodity codes, in order; the first is the display default
    /// and an empty list means unrestricted.
    pub commodity_codes: Vec<String>,
    /// Tags attached to this account.
    pub tags: Vec<TagInfo>,
    /// Date the account opened, or `None` if undeclared.
    pub opened_on: Option<Date>,
    /// Date the account closed, or `None` while open.
    pub closed_on: Option<Date>,
    /// Whether the account is archived.
    #[builder(default)]
    pub archived: bool,
    /// Acquisition record; `Some` only for a manual asset.
    pub acquisition: Option<AcquisitionDto>,
    /// Events recorded against the account, the optimistic-concurrency base.
    pub event_count: u64,
}

/// What closing an account would do, for the confirmation dialog.
///
/// The write command enforces every rule itself; this only informs.
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ClosePreviewDto {
    /// The open descendants a cascading close would also close, ordered by
    /// path. `AccountRef::name` carries the full path.
    pub cascade: Vec<AccountRef>,
    /// Each account's balance as of the close date: the account first, then
    /// the cascade accounts in order. An account that holds nothing is left
    /// out.
    pub balances: Vec<(AccountRef, Vec<Amount>)>,
    /// Postings dated after the close date on the account and its cascade.
    pub postings_after: u64,
}

/// What archiving an account would do, for the confirmation dialog.
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ArchivePreviewDto {
    /// The active descendants a cascading archive would also archive,
    /// ordered by path. `AccountRef::name` carries the full path.
    pub cascade: Vec<AccountRef>,
    /// Whether the account has no closing date.
    pub no_close_date: bool,
}

/// Deserialises a present field, `null` included, as `Some`.
///
/// With `#[serde(default)]`, an absent field stays `None`, so a double
/// option distinguishes "unchanged" (absent) from "clear" (`null`).
///
/// # Errors
///
/// Returns the deserialiser's error if the inner value does not parse.
#[expect(
    clippy::option_option,
    reason = "the outer option is presence on the wire, the inner one the value"
)]
fn double_option<'de, T, D>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A set of changes to one account; each `None` field is left unchanged.
///
/// A double-option field is absent on the wire when unchanged and `null`
/// when cleared.
#[derive(bon::Builder, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[builder(on(String, into))]
#[non_exhaustive]
pub struct AccountPatchDto {
    /// New display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// New parent ID; `Some(None)` makes the account a root.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub parent_id: Option<Option<String>>,
    /// New description; `Some(None)` clears it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub description: Option<Option<String>>,
    /// Replacement tag ID set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag_ids: Option<Vec<String>>,
    /// Replacement commodity codes, in order; the first is the display
    /// default and an empty list means unrestricted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commodity_codes: Option<Vec<String>>,
    /// New opening date; `Some(None)` clears it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    pub opened_on: Option<Option<Date>>,
    /// Corrected closing date; valid only on an account already closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_on: Option<Date>,
    /// Replacement acquisition record; valid only on a manual asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acquisition: Option<AcquisitionDto>,
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
    /// Typed key-value metadata in display order. Repeated keys are legal.
    pub metadata: Vec<MetaEntryDto>,
    /// Tags attached to this posting itself; transaction tags are not repeated here.
    pub tags: Vec<TagInfo>,
    /// Accrual spread start date. `None` means no spreading applied.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end date (inclusive — the last day of the spread). `None` means no spreading applied.
    pub spread_until: Option<jiff::civil::Date>,
    /// Price annotation (`@` per unit, `@@` total), or `None`.
    pub price: Option<Quote>,
    /// Cost basis (`{}` per unit, `{{}}` total), or `None`.
    pub cost: Option<Cost>,
}

impl Posting {
    /// Creates a new [`Posting`].
    ///
    /// # Arguments
    ///
    /// * `id` - Stable posting identifier (UUID string).
    /// * `account` - Account reference with ID and display name.
    /// * `amount` - Posting amount, stored or derived.
    /// * `metadata` - Typed key-value metadata in display order.
    /// * `tags` - Tags attached to this posting itself.
    /// * `spread_from` - Accrual spread start date, or `None`.
    /// * `spread_until` - Accrual spread end date (inclusive — the last day of the spread), or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        account: AccountRef,
        amount: PostingAmount,
        metadata: Vec<MetaEntryDto>,
        tags: Vec<TagInfo>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            id: id.into(),
            account,
            amount,
            metadata,
            tags,
            spread_from,
            spread_until,
            price: None,
            cost: None,
        }
    }

    /// Sets the price annotation, returning `self` for chaining.
    ///
    /// # Arguments
    ///
    /// * `price` - Per-unit or total price, or `None` to clear.
    #[must_use]
    #[inline]
    pub fn with_price(mut self, price: Option<Quote>) -> Self {
        self.price = price;
        self
    }

    /// Sets the cost basis, returning `self` for chaining.
    ///
    /// # Arguments
    ///
    /// * `cost` - The cost basis, or `None` to clear.
    #[must_use]
    #[inline]
    pub fn with_cost(mut self, cost: Option<Cost>) -> Self {
        self.cost = cost;
        self
    }

    /// Weighs this leg for balancing, by Beancount's rule.
    ///
    /// # Returns
    ///
    /// `None` when the amount is not [`PostingAmount::Stored`]; otherwise the
    /// cost's `weigh(units)`, else the price's, else the amount itself. `None`
    /// also when a per-unit product overflows.
    #[must_use]
    pub fn weight(&self) -> Option<Amount> {
        let amount = self.amount.stored()?;
        match self.cost.as_ref().map(|c| &c.basis).or(self.price.as_ref()) {
            Some(quote) => quote.weigh(amount.value),
            None => Some(amount.clone()),
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
    /// Free-text description (raw imported narration).
    pub description: String,
    /// Typed key-value metadata in display order. Repeated keys are legal, and
    /// `payee` holds no privileged position among them.
    pub metadata: Vec<MetaEntryDto>,
    /// Reconciliation status.
    pub reconciliation: Reconciliation,
    /// Tags attached to this transaction.
    pub tags: Vec<TagInfo>,
    /// All postings.
    pub postings: Vec<Posting>,
    /// Audit trail entries (chronological).
    pub audit: Vec<AuditEntry>,
    /// Whether the postings balance per commodity by weight, as
    /// `bc_models::Transaction::balanced` decides. Drives the unbalanced pill.
    pub balanced: bool,
    /// ID of the transaction this one reverses, when it is a reversal.
    pub reverses: Option<String>,
}

impl Transaction {
    /// Creates a new [`Transaction`].
    ///
    /// # Arguments
    ///
    /// * `id` - Stable identifier.
    /// * `date` - Transaction date.
    /// * `description` - Free-text description (raw imported narration).
    /// * `metadata` - Typed key-value metadata in display order.
    /// * `reconciliation` - Reconciliation status.
    /// * `tags` - Tags attached to this transaction.
    /// * `postings` - All postings.
    /// * `audit` - Audit trail entries.
    /// * `balanced` - Whether the postings balance.
    #[must_use]
    #[inline]
    #[expect(
        clippy::too_many_arguments,
        reason = "domain record with many required fields"
    )]
    pub fn new(
        id: impl Into<String>,
        date: jiff::civil::Date,
        description: impl Into<String>,
        metadata: Vec<MetaEntryDto>,
        reconciliation: Reconciliation,
        tags: Vec<TagInfo>,
        postings: Vec<Posting>,
        audit: Vec<AuditEntry>,
        balanced: bool,
    ) -> Self {
        Self {
            id: id.into(),
            date,
            description: description.into(),
            metadata,
            reconciliation,
            tags,
            postings,
            audit,
            balanced,
            reverses: None,
        }
    }

    /// Sets the ID of the transaction this one reverses.
    #[must_use]
    #[inline]
    pub fn with_reverses(mut self, reverses: Option<String>) -> Self {
        self.reverses = reverses;
        self
    }
}

/// The user-supplied fields for a new posting leg.
///
/// Omits `account_path` (derived by the backend from the account record).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NewPosting {
    /// Account ID — must reference an existing active account.
    pub account_id: String,
    /// Posting amount. `None` when the amount should be elided (inferred to balance).
    pub amount: Option<Amount>,
    /// Typed key-value metadata in display order. Each entry's
    /// [`MetaEntryDto::mismatched`] is ignored: the backend derives it against
    /// the key's registered type.
    pub metadata: Vec<MetaEntryDto>,
    /// Tags to attach to this posting; only each tag's ID is read.
    pub tags: Vec<TagInfo>,
    /// Accrual spread start date. `None` means no spreading.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end date (inclusive — the last day of the spread). `None` means no spreading.
    pub spread_until: Option<jiff::civil::Date>,
    /// Price annotation (`@` per unit, `@@` total), or `None`.
    pub price: Option<Quote>,
    /// Cost basis (`{}` per unit, `{{}}` total), or `None`.
    pub cost: Option<Cost>,
}

impl NewPosting {
    /// Creates a new [`NewPosting`].
    ///
    /// # Arguments
    ///
    /// * `account_id` - Account ID referencing an existing active account.
    /// * `amount` - Posting amount, or `None` to elide (inferred to balance).
    /// * `metadata` - Typed key-value metadata in display order.
    /// * `tags` - Tags to attach to this posting; only each tag's ID is read.
    /// * `spread_from` - Accrual spread start date, or `None`.
    /// * `spread_until` - Accrual spread end date (inclusive — the last day of the spread), or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        account_id: impl Into<String>,
        amount: Option<Amount>,
        metadata: Vec<MetaEntryDto>,
        tags: Vec<TagInfo>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            amount,
            metadata,
            tags,
            spread_from,
            spread_until,
            price: None,
            cost: None,
        }
    }

    /// Sets the price annotation, returning `self` for chaining.
    ///
    /// # Arguments
    ///
    /// * `price` - Per-unit or total price, or `None` to clear.
    #[must_use]
    #[inline]
    pub fn with_price(mut self, price: Option<Quote>) -> Self {
        self.price = price;
        self
    }

    /// Sets the cost basis, returning `self` for chaining.
    ///
    /// # Arguments
    ///
    /// * `cost` - The cost basis, or `None` to clear.
    #[must_use]
    #[inline]
    pub fn with_cost(mut self, cost: Option<Cost>) -> Self {
        self.cost = cost;
        self
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
    /// Free-text description (raw narration). Empty string if not provided.
    pub description: String,
    /// Typed key-value metadata in display order. Each entry's
    /// [`MetaEntryDto::mismatched`] is ignored: the backend derives it against
    /// the key's registered type.
    pub metadata: Vec<MetaEntryDto>,
    /// Reconciliation status.
    pub reconciliation: Reconciliation,
    /// Tags to attach to this transaction; only each tag's ID is read.
    pub tags: Vec<TagInfo>,
    /// All postings.
    pub postings: Vec<NewPosting>,
}

impl NewTransaction {
    /// Creates a new [`NewTransaction`].
    ///
    /// # Arguments
    ///
    /// * `date` - Transaction date.
    /// * `description` - Free-text description (raw narration).
    /// * `metadata` - Typed key-value metadata in display order.
    /// * `reconciliation` - Reconciliation status.
    /// * `tags` - Tags to attach to this transaction; only each tag's ID is read.
    /// * `postings` - All postings.
    #[must_use]
    #[inline]
    pub fn new(
        date: jiff::civil::Date,
        description: impl Into<String>,
        metadata: Vec<MetaEntryDto>,
        reconciliation: Reconciliation,
        tags: Vec<TagInfo>,
        postings: Vec<NewPosting>,
    ) -> Self {
        Self {
            date,
            description: description.into(),
            metadata,
            reconciliation,
            tags,
            postings,
        }
    }
}

/// An advisory note about a write that succeeded anyway.
///
/// `message` is the core warning's display text. `account_id` anchors the
/// warning to one account when it concerns one, so the UI can mark that leg.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WriteWarning {
    /// The account the warning concerns, if it concerns one.
    pub account_id: Option<String>,
    /// Human-readable description of the warning.
    pub message: String,
}

impl WriteWarning {
    /// Creates a new [`WriteWarning`].
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account the warning concerns, if any.
    /// * `message` - Human-readable description.
    #[must_use]
    #[inline]
    pub fn new(account_id: Option<String>, message: impl Into<String>) -> Self {
        Self {
            account_id,
            message: message.into(),
        }
    }
}

/// The result of creating a transaction: its ID and any write warnings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CreatedTransaction {
    /// The ID assigned to the new transaction.
    pub id: String,
    /// Warnings raised by the write; empty in the common case.
    pub warnings: Vec<WriteWarning>,
}

impl CreatedTransaction {
    /// Creates a new [`CreatedTransaction`].
    ///
    /// # Arguments
    ///
    /// * `id` - The assigned transaction ID.
    /// * `warnings` - Warnings raised by the write.
    #[must_use]
    #[inline]
    pub fn new(id: impl Into<String>, warnings: Vec<WriteWarning>) -> Self {
        Self {
            id: id.into(),
            warnings,
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
    /// Typed key-value metadata in display order; replaces the stored list on
    /// save. Each entry's [`MetaEntryDto::mismatched`] is ignored: the backend
    /// derives it against the key's registered type.
    pub metadata: Vec<MetaEntryDto>,
    /// Tags to attach; only each tag's ID is read.
    pub tags: Vec<TagInfo>,
    /// Accrual spread start (inclusive), or `None`.
    pub spread_from: Option<jiff::civil::Date>,
    /// Accrual spread end (inclusive, the last day), or `None`.
    pub spread_until: Option<jiff::civil::Date>,
    /// Price annotation (`@` per unit, `@@` total), or `None`.
    pub price: Option<Quote>,
    /// Cost basis (`{}` per unit, `{{}}` total), or `None`.
    pub cost: Option<Cost>,
}

impl EditPosting {
    /// Creates a new [`EditPosting`].
    ///
    /// # Arguments
    ///
    /// * `id` - Existing posting ID to update in place, or `None` to add a new leg.
    /// * `account_id` - Account this posting hits.
    /// * `amount` - Posting amount, or `None` if elided (inferred to balance).
    /// * `metadata` - Typed key-value metadata; replaces the stored list on save.
    /// * `tags` - Tags to attach; only each tag's ID is read.
    /// * `spread_from` - Accrual spread start, or `None`.
    /// * `spread_until` - Accrual spread end, or `None`.
    #[must_use]
    #[inline]
    pub fn new(
        id: Option<String>,
        account_id: impl Into<String>,
        amount: Option<Amount>,
        metadata: Vec<MetaEntryDto>,
        tags: Vec<TagInfo>,
        spread_from: Option<jiff::civil::Date>,
        spread_until: Option<jiff::civil::Date>,
    ) -> Self {
        Self {
            id,
            account_id: account_id.into(),
            amount,
            metadata,
            tags,
            spread_from,
            spread_until,
            price: None,
            cost: None,
        }
    }

    /// Sets the price annotation, returning `self` for chaining.
    ///
    /// # Arguments
    ///
    /// * `price` - Per-unit or total price, or `None` to clear.
    #[must_use]
    #[inline]
    pub fn with_price(mut self, price: Option<Quote>) -> Self {
        self.price = price;
        self
    }

    /// Sets the cost basis, returning `self` for chaining.
    ///
    /// # Arguments
    ///
    /// * `cost` - The cost basis, or `None` to clear.
    #[must_use]
    #[inline]
    pub fn with_cost(mut self, cost: Option<Cost>) -> Self {
        self.cost = cost;
        self
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
    /// Free-text description.
    pub description: String,
    /// Typed key-value metadata in display order; replaces the stored list on
    /// save. Each entry's [`MetaEntryDto::mismatched`] is ignored: the backend
    /// derives it against the key's registered type.
    pub metadata: Vec<MetaEntryDto>,
    /// Reconciliation status (read-only in the editor, echoed back unchanged).
    pub reconciliation: Reconciliation,
    /// Transaction-level tags; only each tag's ID is read.
    pub tags: Vec<TagInfo>,
    /// All postings in display order.
    pub postings: Vec<EditPosting>,
}

impl EditTransaction {
    /// Creates a new [`EditTransaction`] describing the desired post-edit state.
    ///
    /// # Arguments
    ///
    /// * `id` - ID of the transaction being edited.
    /// * `date` - Transaction date.
    /// * `description` - Free-text description.
    /// * `metadata` - Typed key-value metadata; replaces the stored list on save.
    /// * `reconciliation` - Reconciliation status (echoed back unchanged).
    /// * `tags` - Transaction-level tags; only each tag's ID is read.
    /// * `postings` - All postings in display order.
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        date: jiff::civil::Date,
        description: impl Into<String>,
        metadata: Vec<MetaEntryDto>,
        reconciliation: Reconciliation,
        tags: Vec<TagInfo>,
        postings: Vec<EditPosting>,
    ) -> Self {
        Self {
            id: id.into(),
            date,
            description: description.into(),
            metadata,
            reconciliation,
            tags,
            postings,
        }
    }
}

/// The global transaction filter: query text in the transaction query language
/// plus the page's date window. The server parses and resolves the text, so
/// grammar extensions never change this type.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Filter {
    /// The query text; blank matches everything.
    #[serde(default)]
    pub query: String,
    /// Inclusive lower bound of the page's window, joined to the query by `and`.
    /// The palette never sets it.
    pub date_from: Option<Date>,
    /// Exclusive upper bound of the page's window.
    pub date_until: Option<Date>,
}

impl Filter {
    /// Creates a filter.
    ///
    /// # Arguments
    ///
    /// * `query` - The query text.
    /// * `date_from` - Inclusive lower bound of the window.
    /// * `date_until` - Exclusive upper bound of the window.
    #[must_use]
    pub fn new(
        query: impl Into<String>,
        date_from: Option<Date>,
        date_until: Option<Date>,
    ) -> Self {
        Self {
            query: query.into(),
            date_from,
            date_until,
        }
    }
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

/// Position of the last row of a register page: the next page starts after it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RegisterCursor {
    /// Date of the last row.
    pub date: Date,
    /// Transaction id of the last row.
    pub id: String,
}

impl RegisterCursor {
    /// Creates a new [`RegisterCursor`].
    ///
    /// # Arguments
    ///
    /// * `date` - Date of the last row.
    /// * `id` - Transaction id of the last row.
    #[must_use]
    #[inline]
    pub fn new(date: Date, id: String) -> Self {
        Self { date, id }
    }
}

/// One page of the register for a sidebar account.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RegisterRequest {
    /// The global filter; the UI has already resolved the display window into
    /// its date bounds (none for all time).
    pub filter: Filter,
    /// The account whose page is shown.
    pub account_id: String,
    /// `true` folds the account's subtree into the scope.
    pub include_descendants: bool,
    /// Resume after this row; `None` for the first page.
    pub cursor: Option<RegisterCursor>,
    /// Maximum rows to return.
    pub limit: u32,
}

impl RegisterRequest {
    /// Creates a new [`RegisterRequest`].
    ///
    /// # Arguments
    ///
    /// * `filter` - The global filter with the display window resolved.
    /// * `account_id` - The account whose page is shown.
    /// * `include_descendants` - Whether the subtree joins the scope.
    /// * `cursor` - Resume point, `None` for the first page.
    /// * `limit` - Maximum rows to return.
    #[must_use]
    #[inline]
    pub fn new(
        filter: Filter,
        account_id: String,
        include_descendants: bool,
        cursor: Option<RegisterCursor>,
        limit: u32,
    ) -> Self {
        Self {
            filter,
            account_id,
            include_descendants,
            cursor,
            limit,
        }
    }
}

/// A register row: the matched transaction plus the scope's running balances
/// after it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RegisterRow {
    /// The whole matched transaction (never pruned server-side).
    pub transaction: Transaction,
    /// Posting ids of the legs that matched the posting-scoped predicates.
    pub matched_postings: Vec<String>,
    /// Real scope balance after this transaction, in the row's focal
    /// commodity; `None` when the transaction moved two or more commodities on
    /// the scope.
    pub balance_after: Option<Amount>,
    /// Running sum of the matching rows' focal movements up to and including
    /// this one, anchored at the oldest match; `None` under the same condition.
    pub filtered_sum_after: Option<Amount>,
}

impl RegisterRow {
    /// Creates a new [`RegisterRow`].
    ///
    /// # Arguments
    ///
    /// * `transaction` - The matched transaction.
    /// * `matched_postings` - Ids of the matching legs.
    /// * `balance_after` - Real scope balance after this row.
    /// * `filtered_sum_after` - Running sum over matching rows.
    #[must_use]
    #[inline]
    pub fn new(
        transaction: Transaction,
        matched_postings: Vec<String>,
        balance_after: Option<Amount>,
        filtered_sum_after: Option<Amount>,
    ) -> Self {
        Self {
            transaction,
            matched_postings,
            balance_after,
            filtered_sum_after,
        }
    }
}

/// A page of register rows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct RegisterPage {
    /// The rows, newest first.
    pub rows: Vec<RegisterRow>,
    /// Total matching rows across every page.
    pub total: u32,
    /// Cursor for the next page; `None` when this page reached the end.
    pub next_cursor: Option<RegisterCursor>,
}

impl RegisterPage {
    /// Creates a new [`RegisterPage`].
    ///
    /// # Arguments
    ///
    /// * `rows` - The rows, newest first.
    /// * `total` - Total matching rows.
    /// * `next_cursor` - Resume point for the next page.
    #[must_use]
    #[inline]
    pub fn new(rows: Vec<RegisterRow>, total: u32, next_cursor: Option<RegisterCursor>) -> Self {
        Self {
            rows,
            total,
            next_cursor,
        }
    }
}

/// Windowed account statistics for the dashboard.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountStats {
    /// In-window net inflow. Per transaction, the scope's legs net; a positive
    /// net counts here.
    pub inflow: Amount,
    /// In-window net outflow, as a non-negative magnitude. Per transaction, a
    /// negative net of the scope's legs counts here.
    pub outflow: Amount,
    /// In-window movement between legs of the scope (for a rolled-up parent,
    /// transfers between its descendants); non-negative.
    pub internal: Amount,
    /// `inflow − outflow` (signed).
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
    /// Earliest posting date over the scope, unwindowed; `None` for an
    /// account with no postings. Spans the all-time sparkline.
    pub first_activity: Option<Date>,
}

impl AccountStats {
    /// Creates a new [`AccountStats`].
    ///
    /// # Arguments
    ///
    /// * `inflow` - In-window net inflow.
    /// * `outflow` - In-window net outflow magnitude.
    /// * `internal` - In-window movement inside the scope.
    /// * `net` - Signed net movement.
    /// * `opening_balance` - Balance at window start.
    /// * `closing_balance` - Balance at window end.
    /// * `tx_count` - Count of distinct in-window transactions (matches the register).
    #[must_use]
    #[inline]
    pub fn new(
        inflow: Amount,
        outflow: Amount,
        internal: Amount,
        net: Amount,
        opening_balance: Amount,
        closing_balance: Amount,
        tx_count: u32,
    ) -> Self {
        Self {
            inflow,
            outflow,
            internal,
            net,
            opening_balance,
            closing_balance,
            tx_count,
            real_opening: None,
            real_closing: None,
            first_activity: None,
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

    /// Attaches the scope's earliest posting date.
    ///
    /// # Arguments
    ///
    /// * `first_activity` - Earliest posting date, or `None` with no postings.
    #[must_use]
    #[inline]
    pub fn with_first_activity(mut self, first_activity: Option<Date>) -> Self {
        self.first_activity = first_activity;
        self
    }
}

/// A single data point in a cash-flow sparkline: a time-bucket label plus net
/// inflow and outflow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SparkPoint {
    /// X-axis label, e.g. `"apr"`, `"w03"`, `"Q2"`.
    pub label: String,
    /// Net inflow for the bucket (non-negative).
    pub inflow: Amount,
    /// Net outflow for the bucket (non-negative magnitude, plotted separately).
    pub outflow: Amount,
}

impl SparkPoint {
    /// Creates a new [`SparkPoint`].
    ///
    /// # Arguments
    ///
    /// * `label`   - X-axis label.
    /// * `inflow`  - Net inflow amount.
    /// * `outflow` - Net outflow amount (non-negative magnitude).
    #[must_use]
    #[inline]
    pub fn new(label: impl Into<String>, inflow: Amount, outflow: Amount) -> Self {
        Self {
            label: label.into(),
            inflow,
            outflow,
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::Span;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::*;
    use crate::Amount;
    use crate::MetaValueDto;
    use crate::quote::Cost;
    use crate::quote::Quote;

    #[test]
    fn account_patch_tells_absent_from_null() {
        let patch: AccountPatchDto = serde_json::from_value(serde_json::json!({
            "name": "Everyday",
            "parent_id": null,
        }))
        .expect("de");
        assert_eq!(patch.name.as_deref(), Some("Everyday"));
        assert_eq!(patch.parent_id, Some(None), "null clears the parent");
        assert_eq!(patch.description, None, "absent leaves it unchanged");
    }

    #[test]
    fn account_patch_round_trips_through_json() {
        let patch = AccountPatchDto::builder()
            .parent_id(None)
            .description(Some("Main account".to_owned()))
            .commodity_codes(vec!["AUD".to_owned()])
            .opened_on(Some(date(2026, 1, 15)))
            .build();
        let json = serde_json::to_value(&patch).expect("ser");
        assert_eq!(
            json,
            serde_json::json!({
                "parent_id": null,
                "description": "Main account",
                "commodity_codes": ["AUD"],
                "opened_on": "2026-01-15",
            }),
            "unchanged fields are left off the wire"
        );
        let back: AccountPatchDto = serde_json::from_value(json).expect("de");
        assert_eq!(back, patch);
    }

    /// A one-entry metadata list carrying `key` as text.
    fn meta(key: &str, text: &str) -> Vec<MetaEntryDto> {
        vec![MetaEntryDto::new(key, MetaValueDto::Text(text.to_owned()))]
    }

    #[test]
    fn with_rollup_sets_field_and_new_leaves_it_empty() {
        let mut node = AccountNode::new(
            "assets",
            "Assets",
            None::<&str>,
            None,
            None::<&str>,
            AccountType::Asset,
            vec![],
            None,
            None,
        );
        assert_eq!(node.rollup, []);

        node = node.with_rollup(vec![
            Amount::new(Decimal::new(10_500, 2), "AUD"),
            Amount::new(Decimal::new(3_000, 2), "USD"),
        ]);
        assert_eq!(node.rollup.len(), 2);
        assert_eq!(
            node.rollup.first().map(|a| &a.currency_code),
            Some(&String::from("AUD"))
        );
    }

    #[test]
    fn new_posting_constructor_roundtrip() {
        let p = NewPosting::new(
            "acc-1",
            Some(Amount::new(Decimal::new(-1_000, 2), "AUD")),
            meta("note", "test note"),
            vec![],
            None,
            None,
        );
        assert_eq!(p.account_id, "acc-1");
        assert_eq!(
            p.amount.as_ref().map(|a| a.value),
            Some(rust_decimal::Decimal::new(-1_000, 2))
        );
        assert_eq!(p.metadata, meta("note", "test note"));
    }

    #[rstest]
    #[case(Reconciliation::Unreconciled)]
    #[case(Reconciliation::Reconciled)]
    fn new_transaction_serde_roundtrip(#[case] reconciliation: Reconciliation) {
        let tx = NewTransaction::new(
            jiff::civil::Date::constant(2026, 5, 23),
            "",
            meta("payee", "Generic Grocer"),
            reconciliation,
            vec![],
            vec![
                NewPosting::new(
                    "acc-a",
                    Some(Amount::new(Decimal::new(-500, 2), "AUD")),
                    vec![],
                    vec![],
                    None,
                    None,
                ),
                NewPosting::new(
                    "acc-b",
                    Some(Amount::new(Decimal::new(500, 2), "AUD")),
                    vec![],
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
            meta("note", "posting note"),
            vec![TagInfo::new("tag-abc", "person")],
            None,
            None,
        );
        let tx = Transaction::new(
            "tx-1",
            jiff::civil::Date::constant(2026, 6, 1),
            "raw narration",
            vec![
                MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
                MetaEntryDto::new(
                    "effective",
                    MetaValueDto::Date(jiff::civil::Date::constant(2026, 6, 15)),
                ),
            ],
            Reconciliation::Flagged,
            vec![],
            vec![posting],
            vec![],
            true,
        );
        let json = serde_json::to_string(&tx).expect("serialises");
        let tx2: Transaction = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(tx, tx2);
        assert_eq!(tx2.reconciliation, Reconciliation::Flagged);
        assert_eq!(tx2.description, "raw narration");
        assert_eq!(
            tx2.metadata
                .iter()
                .map(|e| e.key.as_str())
                .collect::<Vec<_>>(),
            vec!["payee", "effective"],
            "order is the display order and survives the round trip"
        );
        assert!(matches!(
            tx2.postings.first().map(|p| &p.amount),
            Some(PostingAmount::Ambiguous)
        ));
        assert_eq!(
            tx2.postings.first().map(|p| p.tags.as_slice()),
            Some([TagInfo::new("tag-abc", "person")].as_slice())
        );
        assert_eq!(
            tx2.postings.first().map(|p| p.metadata.clone()),
            Some(meta("note", "posting note"))
        );
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
            "",
            meta("payee", "Generic Grocer"),
            reconciliation,
            vec![
                TagInfo::new("tag-groceries", "category:groceries"),
                TagInfo::new("tag-food", "budget:food"),
            ],
            vec![
                NewPosting::new(
                    "acc-a",
                    Some(Amount::new(Decimal::new(-3_000, 2), "AUD")),
                    vec![],
                    vec![],
                    None,
                    None,
                ),
                NewPosting::new(
                    "acc-b",
                    Some(Amount::new(Decimal::new(3_000, 2), "AUD")),
                    vec![],
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
            tx2.tags.first().map(|t| t.path.as_str()),
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
        assert_eq!(back.inflow.currency_code, "AUD");
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
            description: "salary".to_owned(),
            metadata: meta("payee", "Generic Employer"),
            reconciliation: Reconciliation::Unreconciled,
            tags: vec![TagInfo::new("tag-work", "work")],
            postings: vec![EditPosting {
                id: Some("p-1".to_owned()),
                account_id: "acc-1".to_owned(),
                amount: None,
                metadata: vec![],
                tags: vec![],
                spread_from: None,
                spread_until: None,
                price: None,
                cost: None,
            }],
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
            "raw narration",
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            vec![],
            vec![],
            true,
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
    fn filter_round_trips() {
        let f = Filter::new(
            "@payee:cafe amount:>=100",
            Some(date(2026, 1, 1)),
            Some(date(2026, 2, 1)),
        );
        let json = serde_json::to_string(&f).expect("serialize");
        let back: Filter = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, f);
    }

    #[test]
    fn filter_without_a_query_reads_as_blank() {
        let back: Filter =
            serde_json::from_str(r#"{"date_from":null,"date_until":null}"#).expect("deserialize");
        assert_eq!(back, Filter::default());
    }

    #[test]
    fn edit_transaction_new_builds_expected() {
        use jiff::civil::Date;

        let p = EditPosting::new(
            Some("p-1".to_owned()),
            "acct-checking",
            Some(Amount::new(rust_decimal::Decimal::new(-5_000, 2), "AUD")),
            vec![],
            vec![TagInfo::new("tag-1", "person")],
            None,
            None,
        );
        let tx = EditTransaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "weekly shop",
            meta("payee", "Generic Grocer"),
            Reconciliation::Unreconciled,
            vec![TagInfo::new("tag-work", "work")],
            vec![p.clone()],
        );

        assert_eq!(tx.id, "tx-1");
        assert_eq!(tx.date, Date::constant(2026, 4, 30));
        assert_eq!(tx.metadata, meta("payee", "Generic Grocer"));
        assert_eq!(tx.postings, vec![p]);
    }

    #[test]
    fn account_node_round_trips_its_dates() {
        let node = AccountNode::new(
            "acc-1",
            "Smart Access",
            None::<&str>,
            None,
            None::<&str>,
            AccountType::Asset,
            vec![],
            Some(jiff::civil::date(2020, 1, 1)),
            Some(jiff::civil::date(2024, 6, 30)),
        );

        let json = serde_json::to_string(&node).expect("serialise");
        let back: AccountNode = serde_json::from_str(&json).expect("deserialise");

        assert_eq!(back.opened_on, Some(jiff::civil::date(2020, 1, 1)));
        assert_eq!(back.closed_on, Some(jiff::civil::date(2024, 6, 30)));
    }

    #[test]
    fn account_stats_real_balances_default_none_and_set() {
        let z = Amount::new(rust_decimal::Decimal::ZERO, "AUD");
        let stats = AccountStats::new(
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
            z.clone(),
            0,
        );
        assert_eq!(stats.internal, z);
        assert_eq!(stats.real_opening, None);
        assert_eq!(stats.real_closing, None);

        let real = Amount::new(rust_decimal::Decimal::new(4210, 0), "AUD");
        let stats_with_real = stats.with_real_balances(real.clone(), real.clone());
        assert_eq!(stats_with_real.real_opening, Some(real.clone()));
        assert_eq!(stats_with_real.real_closing, Some(real));
    }

    fn stored_posting(units: Decimal, code: &str) -> Posting {
        Posting::new(
            "p1",
            AccountRef::new("acct", "Assets :: Brokerage"),
            PostingAmount::Stored(Amount::new(units, code)),
            vec![],
            vec![],
            None,
            None,
        )
    }

    #[test]
    fn weight_without_annotation_is_the_amount() {
        let p = stored_posting(Decimal::TWO, "AAPL");
        assert_eq!(p.weight(), Some(Amount::new(Decimal::TWO, "AAPL")));
    }

    #[test]
    fn weight_uses_price_when_no_cost() {
        let p = stored_posting(Decimal::new(400, 2), "USD")
            .with_price(Some(Quote::Total(Amount::new(Decimal::new(637, 2), "AUD"))));
        assert_eq!(p.weight(), Some(Amount::new(Decimal::new(637, 2), "AUD")));
    }

    #[test]
    fn weight_prefers_cost_over_price() {
        let p = stored_posting(-Decimal::TWO, "AAPL")
            .with_cost(Some(Cost::new(
                Quote::PerUnit(Amount::new(Decimal::new(10_500, 2), "AUD")),
                None,
                None,
            )))
            .with_price(Some(Quote::PerUnit(Amount::new(
                Decimal::new(15_000, 2),
                "AUD",
            ))));
        assert_eq!(
            p.weight(),
            Some(Amount::new(Decimal::new(-21_000, 2), "AUD"))
        );
    }

    #[test]
    fn weight_of_an_elided_leg_is_none() {
        let derived = Posting::new(
            "p1",
            AccountRef::new("acct", "Assets :: Cash"),
            PostingAmount::Derived(vec![Amount::new(Decimal::ONE, "AUD")]),
            vec![],
            vec![],
            None,
            None,
        )
        .with_price(Some(Quote::PerUnit(Amount::new(Decimal::ONE, "AUD"))));
        assert_eq!(derived.weight(), None);
        let ambiguous = Posting::new(
            "p2",
            AccountRef::new("acct", "Assets :: Cash"),
            PostingAmount::Ambiguous,
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(ambiguous.weight(), None);
    }

    #[test]
    fn edit_posting_setters_round_trip() {
        let price = Some(Quote::PerUnit(Amount::new(Decimal::ONE, "AUD")));
        let e = EditPosting::new(None, "acct", None, vec![], vec![], None, None)
            .with_price(price.clone());
        assert_eq!(e.price, price);
        assert_eq!(e.cost, None);
        let n = NewPosting::new("acct", None, vec![], vec![], None, None).with_price(price.clone());
        assert_eq!(n.price, price);
    }
}
