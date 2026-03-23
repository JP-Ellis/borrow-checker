//! Transaction, posting, cost and link domain types.

use core::{fmt, str::FromStr};

use jiff::{Timestamp, civil::Date};
use mti::prelude::*;
use serde::{Deserialize, Serialize};

use crate::{TagId, money::Amount};

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
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[expect(
    clippy::struct_field_names,
    reason = "`link_type` is the idiomatic name for this domain field"
)]
pub struct Link {
    /// Unique identifier.
    id: TransactionLinkId,
    /// The type of relationship between linked transactions.
    link_type: LinkType,
    /// All transaction IDs that belong to this link; populated by `bc-core`.
    #[builder(default)]
    member_transaction_ids: Vec<TransactionId>,
    /// When this link was created in the system.
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
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Cost {
    /// Total acquisition cost in the cost commodity (given up / paid).
    total: Amount,
    /// Optional lot date for FIFO/LIFO tracking.
    date: Option<Date>,
    /// Optional lot label.
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
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Posting {
    /// Unique identifier.
    id: PostingId,
    /// The account this posting affects.
    account_id: crate::AccountId,
    /// The amount (positive = debit, negative = credit in standard accounting).
    amount: Amount,
    /// Optional cost basis for commodity conversions.
    cost: Option<Cost>,
    /// Optional memo for this posting.
    #[builder(into)]
    memo: Option<String>,
    /// Posting-level tags (in addition to transaction-level tags).
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
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Transaction {
    /// Unique identifier.
    id: TransactionId,
    /// The date on which the transaction occurred (no time component).
    date: Date,
    /// Optional payee name.
    #[builder(into)]
    payee: Option<String>,
    /// Description or narration.
    #[builder(into)]
    description: String,
    /// All postings; must sum to zero per commodity.
    #[builder(default)]
    postings: Vec<Posting>,
    /// Lifecycle status.
    status: Status,
    /// Transaction-level tag IDs.
    #[builder(default)]
    tag_ids: Vec<TagId>,
    /// Link IDs this transaction participates in.
    #[builder(default)]
    link_ids: Vec<TransactionLinkId>,
    /// When this record was created in the system.
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
    use crate::money::{Amount, CommodityCode};

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
        assert_eq!(posting.amount().commodity.to_string(), "AUD");
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
        assert_eq!(cost.total().value.to_string(), "1500");
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
}
