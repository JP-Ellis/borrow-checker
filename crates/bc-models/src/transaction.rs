//! Transaction, posting, cost and link domain types.

use core::fmt;
use core::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

use crate::TagId;
use crate::money::Amount;

crate::define_id!(TransactionId, "transaction");
crate::define_id!(PostingId, "posting");
crate::define_id!(TransactionLinkId, "transaction_link");

/// The lifecycle status of a transaction.
///
/// Re-exported from the crate root as [`crate::TransactionStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Not yet confirmed.
    Pending,
    /// Confirmed and included in balances.
    Cleared,
    /// Cancelled; excluded from balances.
    Voided,
}

/// The kind of relationship between linked transactions.
///
/// Re-exported from the crate root as [`crate::TransactionLinkType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum LinkType {
    /// Two transactions representing the same inter-account movement.
    Transfer,
    /// One transaction cancels a prior one.
    Reversal,
}

/// A link grouping related transactions (e.g. a transfer pair or reversal).
///
/// Re-exported from the crate root as [`crate::TransactionLink`].
///
/// # Example
///
/// ```
/// use bc_models::{TransactionLink, TransactionLinkId, TransactionLinkType};
/// use jiff::Timestamp;
///
/// let link = TransactionLink::builder()
///     .id(TransactionLinkId::new())
///     .link_type(TransactionLinkType::Transfer)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert!(link.member_transaction_ids().is_empty());
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[expect(
    clippy::struct_field_names,
    reason = "`link_type` is the idiomatic name for this domain field"
)]
pub struct Link {
    /// Stable identifier for this link. Assigned by `bc-core` on creation; do not
    /// generate ad hoc outside of the persistence layer.
    id: TransactionLinkId,

    /// The nature of the relationship between linked transactions.
    /// [`crate::TransactionLinkType::Transfer`] matches two legs of the same inter-account movement;
    /// [`crate::TransactionLinkType::Reversal`] marks one transaction as cancelling a prior one.
    link_type: LinkType,

    /// IDs of all transactions that belong to this link. Defaults to empty;
    /// `bc-core` appends each member transaction's ID after persisting it.
    #[builder(default)]
    member_transaction_ids: Vec<TransactionId>,

    /// Timestamp recorded when this link was first persisted.
    created_at: Timestamp,
}

impl Link {
    /// Returns the link ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &TransactionLinkId {
        &self.id
    }

    /// Returns the link type.
    #[inline]
    #[must_use]
    pub fn link_type(&self) -> LinkType {
        self.link_type
    }

    /// Returns the member transaction IDs.
    #[inline]
    #[must_use]
    pub fn member_transaction_ids(&self) -> &[TransactionId] {
        &self.member_transaction_ids
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

/// Cost basis for a commodity conversion posting.
///
/// `total` is the cost in the *cost commodity* (the commodity given up).
/// Unit price = `total.value / posting.amount.value` (derive on demand).
/// `bc-core` must ensure `posting.amount.value != 0` when cost is present.
///
/// # Example
///
/// ```
/// use bc_models::{Cost, Amount, CommodityCode};
/// use rust_decimal::Decimal;
///
/// let cost = Cost::builder()
///     .total(Amount::new(Decimal::from(1500), CommodityCode::new("USD")))
///     .build();
///
/// assert_eq!(cost.total().value(), Decimal::from(1500));
/// assert!(cost.date().is_none());
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Cost {
    /// Total acquisition cost expressed in the *cost commodity* — the asset
    /// given up or paid. To derive the per-unit price, divide this by the
    /// posting's `amount.value`; `bc-core` guarantees that value is non-zero
    /// when a cost is attached.
    total: Amount,

    /// Calendar date assigned to this lot for `FIFO`/`LIFO` inventory tracking.
    /// `None` if lot dating is not required for this position.
    date: Option<Date>,

    /// Human-assigned label for manual lot identification (e.g. `"lot-2024-01"`).
    /// `None` if the lot has not been labelled.
    #[builder(into)]
    label: Option<String>,
}

impl Cost {
    /// Returns the total cost amount.
    #[inline]
    #[must_use]
    pub fn total(&self) -> &Amount {
        &self.total
    }

    /// Returns the lot date, if any.
    #[inline]
    #[must_use]
    pub fn date(&self) -> Option<Date> {
        self.date
    }

    /// Returns the lot label, if any.
    #[inline]
    #[must_use]
    pub fn label(&self) -> Option<&str> {
        self.label.as_deref()
    }
}

/// A single leg of a double-entry transaction.
///
/// # Example
///
/// ```
/// use bc_models::{Posting, PostingId, AccountId, Amount, CommodityCode};
/// use rust_decimal::Decimal;
///
/// let posting = Posting::builder()
///     .id(PostingId::new())
///     .account_id(AccountId::new())
///     .amount(Amount::new(Decimal::from(100), CommodityCode::new("AUD")))
///     .build();
///
/// assert_eq!(posting.amount().commodity().to_string(), "AUD");
/// assert!(posting.cost().is_none());
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Posting {
    /// Stable, opaque identifier for this posting. Assigned by `bc-core` when the
    /// parent transaction is persisted; do not generate outside the persistence layer.
    id: PostingId,

    /// The account this posting credits or debits.
    account_id: crate::AccountId,

    /// Monetary amount of this leg. Positive values are debits; negative values are credits.
    /// The sum of all posting amounts in a transaction must be zero per commodity —
    /// enforced by `bc-core`.
    amount: Amount,

    /// Cost basis for a commodity conversion, if applicable. `None` for
    /// same-commodity postings; required when tracking acquisition cost across
    /// currency or asset conversions.
    cost: Option<Cost>,

    /// Optional free-text note for this individual posting leg. `None` means
    /// no memo has been recorded.
    #[builder(into)]
    memo: Option<String>,

    /// Tags applied specifically to this posting leg, in addition to any
    /// transaction-level tags. Defaults to empty.
    #[builder(default)]
    tag_ids: Vec<TagId>,
}

impl Posting {
    /// Returns the posting ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &PostingId {
        &self.id
    }

    /// Returns the account ID.
    #[inline]
    #[must_use]
    pub fn account_id(&self) -> &crate::AccountId {
        &self.account_id
    }

    /// Returns the amount.
    #[inline]
    #[must_use]
    pub fn amount(&self) -> &Amount {
        &self.amount
    }

    /// Returns the cost basis, if any.
    #[inline]
    #[must_use]
    pub fn cost(&self) -> Option<&Cost> {
        self.cost.as_ref()
    }

    /// Returns the memo, if any.
    #[inline]
    #[must_use]
    pub fn memo(&self) -> Option<&str> {
        self.memo.as_deref()
    }

    /// Returns the posting-level tag IDs.
    #[inline]
    #[must_use]
    pub fn tag_ids(&self) -> &[TagId] {
        &self.tag_ids
    }
}

/// A double-entry accounting transaction.
///
/// All postings must sum to zero per commodity (enforced by `bc-core`).
///
/// # Builder design — `id` and `created_at` are required
///
/// Unlike [`crate::Tag`], [`crate::Account`], and [`crate::Commodity`],
/// `Transaction`, [`Posting`], and [`Link`] do **not** use `#[builder(default)]`
/// on `id` or `created_at`. The reason is stability: transaction IDs must survive
/// event-sourcing replay (re-processing the same import batch must produce the
/// same IDs), so callers are required to supply a deterministic or pre-allocated
/// ID rather than letting the model generate a fresh random one. `created_at` is
/// similarly explicit so that import replays can preserve the original timestamp.
///
/// # Example
///
/// ```
/// use bc_models::{Transaction, TransactionId, TransactionStatus};
/// use jiff::{civil::date, Timestamp};
///
/// let tx = Transaction::builder()
///     .id(TransactionId::new())
///     .date(date(2026, 1, 15))
///     .description("Groceries")
///     .status(TransactionStatus::Cleared)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(tx.description(), "Groceries");
/// assert!(tx.postings().is_empty());
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Transaction {
    /// Stable, opaque identifier for this transaction. Assigned by `bc-core` on
    /// persistence; do not generate outside the persistence layer.
    id: TransactionId,

    /// Calendar date on which the transaction occurred (no time-of-day component).
    /// This is the *value date*, not the date the record was created in the system.
    date: Date,

    /// Name of the counterparty (e.g. a merchant or payee). `None` if the payee is
    /// unknown or not applicable for this transaction type.
    #[builder(into)]
    payee: Option<String>,

    /// Free-text description or narration summarising the purpose of this transaction.
    #[builder(into)]
    description: String,

    /// All posting legs of this transaction. Must sum to zero per commodity —
    /// `bc-core` enforces this invariant before persistence. Defaults to empty.
    #[builder(default)]
    postings: Vec<Posting>,

    /// Lifecycle status of this transaction (pending, cleared, or voided).
    status: Status,

    /// Tags applied at the transaction level, shared across all posting legs.
    /// Defaults to empty.
    #[builder(default)]
    tag_ids: Vec<TagId>,

    /// IDs of [`crate::TransactionLink`]s this transaction participates in (e.g.
    /// a transfer pair or a reversal). Defaults to empty; managed by `bc-core`.
    #[builder(default)]
    link_ids: Vec<TransactionLinkId>,

    /// Timestamp recorded when this transaction was first persisted. Callers
    /// constructing a new transaction should pass [`jiff::Timestamp::now()`].
    created_at: Timestamp,
}

impl Transaction {
    /// Returns the transaction ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &TransactionId {
        &self.id
    }

    /// Returns the transaction date.
    #[inline]
    #[must_use]
    pub fn date(&self) -> Date {
        self.date
    }

    /// Returns the payee, if any.
    #[inline]
    #[must_use]
    pub fn payee(&self) -> Option<&str> {
        self.payee.as_deref()
    }

    /// Returns the description.
    #[inline]
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// Returns the postings.
    #[inline]
    #[must_use]
    pub fn postings(&self) -> &[Posting] {
        &self.postings
    }

    /// Returns the status.
    #[inline]
    #[must_use]
    pub fn status(&self) -> Status {
        self.status
    }

    /// Returns the transaction-level tag IDs.
    #[inline]
    #[must_use]
    pub fn tag_ids(&self) -> &[TagId] {
        &self.tag_ids
    }

    /// Returns the link IDs this transaction participates in.
    #[inline]
    #[must_use]
    pub fn link_ids(&self) -> &[TransactionLinkId] {
        &self.link_ids
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::money::Amount;
    use crate::money::CommodityCode;

    // --- TransactionId ---

    #[test]
    fn transaction_id_has_correct_prefix() {
        let id = TransactionId::new();
        assert!(id.to_string().starts_with("transaction_"));
    }

    #[test]
    fn transaction_id_round_trips_through_string() {
        let id = TransactionId::new();
        let s = id.to_string();
        let parsed: TransactionId = s.parse().expect("parse should succeed");
        assert_eq!(id, parsed);
    }

    #[test]
    fn transaction_id_serializes_to_bare_string() {
        let id = TransactionId::new();
        let json = serde_json::to_string(&id).expect("serialize should succeed");
        assert!(json.starts_with('"'));
        assert!(!json.contains('{'));
    }

    // --- PostingId ---

    #[test]
    fn posting_id_has_correct_prefix() {
        let id = PostingId::new();
        assert!(id.to_string().starts_with("posting_"));
    }

    #[test]
    fn posting_id_round_trips_through_string() {
        let id = PostingId::new();
        let s = id.to_string();
        let parsed: PostingId = s.parse().expect("parse should succeed");
        assert_eq!(id, parsed);
    }

    #[test]
    fn posting_id_serializes_to_bare_string() {
        let id = PostingId::new();
        let json = serde_json::to_string(&id).expect("serialize should succeed");
        assert!(json.starts_with('"'));
        assert!(!json.contains('{'));
    }

    // --- TransactionLinkId ---

    #[test]
    fn transaction_link_id_has_correct_prefix() {
        let id = TransactionLinkId::new();
        assert!(id.to_string().starts_with("transaction_link_"));
    }

    #[test]
    fn transaction_link_id_round_trips_through_string() {
        let id = TransactionLinkId::new();
        let s = id.to_string();
        let parsed: TransactionLinkId = s.parse().expect("parse should succeed");
        assert_eq!(id, parsed);
    }

    #[test]
    fn transaction_link_id_serializes_to_bare_string() {
        let id = TransactionLinkId::new();
        let json = serde_json::to_string(&id).expect("serialize should succeed");
        assert!(json.starts_with('"'));
        assert!(!json.contains('{'));
    }

    #[test]
    fn transaction_status_variants_exist() {
        _ = (Status::Pending, Status::Cleared, Status::Voided);
    }

    #[test]
    fn posting_stores_amount() {
        let posting = Posting::builder()
            .id(PostingId::new())
            .account_id(crate::AccountId::new())
            .amount(Amount::new(dec!(100.00), CommodityCode::new("AUD")))
            .build();
        assert_eq!(posting.amount().commodity().to_string(), "AUD");
    }

    #[test]
    fn posting_builder_works() {
        let p = Posting::builder()
            .id(PostingId::new())
            .account_id(crate::AccountId::new())
            .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
            .build();
        assert!(p.cost().is_none());
        assert!(p.tag_ids().is_empty());
    }

    #[test]
    fn cost_stores_total_in_cost_commodity() {
        let cost = Cost::builder()
            .total(Amount::new(dec!(1500), CommodityCode::new("USD")))
            .build();
        assert_eq!(cost.total().value().to_string(), "1500");
    }

    #[test]
    fn transaction_link_members_stored() {
        use jiff::Timestamp;

        let link = Link::builder()
            .id(TransactionLinkId::new())
            .link_type(LinkType::Transfer)
            .member_transaction_ids(vec![TransactionId::new()])
            .created_at(Timestamp::now())
            .build();
        assert_eq!(link.member_transaction_ids().len(), 1);
    }

    #[test]
    fn transaction_has_link_ids_and_tag_ids() {
        use jiff::Timestamp;

        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 1))
            .description("Test")
            .status(Status::Cleared)
            .created_at(Timestamp::now())
            .build();
        assert!(tx.link_ids().is_empty());
        assert!(tx.tag_ids().is_empty());
    }

    #[test]
    fn posting_with_memo_cost_and_tag_ids_round_trips() {
        use crate::TagId;

        let tag_id = TagId::new();
        let cost_basis = Cost::builder()
            .total(Amount::new(dec!(500), CommodityCode::new("USD")))
            .build();
        let posting = Posting::builder()
            .id(PostingId::new())
            .account_id(crate::AccountId::new())
            .amount(Amount::new(dec!(10), CommodityCode::new("AAPL")))
            .memo("lot purchase memo")
            .cost(cost_basis)
            .tag_ids(vec![tag_id.clone()])
            .build();

        assert_eq!(posting.memo(), Some("lot purchase memo"));
        let cost = posting.cost().expect("cost should be set");
        assert_eq!(cost.total().value(), dec!(500));
        assert_eq!(cost.total().commodity().to_string(), "USD");
        assert_eq!(posting.tag_ids().len(), 1);
        assert_eq!(
            posting.tag_ids().first().expect("tag should exist"),
            &tag_id
        );
    }
}
