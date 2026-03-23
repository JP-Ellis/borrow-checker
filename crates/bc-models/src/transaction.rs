//! Transaction and posting domain types.

use jiff::{Timestamp, civil::Date};

use crate::{
    AccountId,
    ids::{PostingId, TransactionId},
    money::Amount,
};

/// The lifecycle status of a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum TransactionStatus {
    /// Not yet confirmed.
    Pending,
    /// Confirmed and included in balances.
    Cleared,
    /// Cancelled; excluded from balances.
    Voided,
}

/// A single leg of a double-entry transaction.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Posting {
    /// Unique identifier.
    pub id: PostingId,
    /// The account this posting affects.
    pub account_id: AccountId,
    /// The amount (positive = debit, negative = credit in standard accounting).
    pub amount: Amount,
    /// Optional memo for this posting.
    pub memo: Option<String>,
}

impl Posting {
    /// Creates a new [`Posting`] with all fields.
    ///
    /// This constructor is required because the struct is `#[non_exhaustive]`.
    #[inline]
    #[must_use]
    pub fn new(id: PostingId, account_id: AccountId, amount: Amount, memo: Option<String>) -> Self {
        Self {
            id,
            account_id,
            amount,
            memo,
        }
    }
}

/// A double-entry accounting transaction.
///
/// All postings must sum to zero per commodity (enforced by `bc-core`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Transaction {
    /// Unique identifier.
    pub id: TransactionId,
    /// The date on which the transaction occurred (no time component).
    pub date: Date,
    /// Optional payee name.
    pub payee: Option<String>,
    /// Description or narration.
    pub description: String,
    /// All postings; must sum to zero per commodity.
    pub postings: Vec<Posting>,
    /// Lifecycle status.
    pub status: TransactionStatus,
    /// Arbitrary tags for search and filtering.
    pub tags: Vec<String>,
    /// When this record was created in the system.
    pub created_at: Timestamp,
}

impl Transaction {
    /// Creates a new [`Transaction`] with all fields.
    ///
    /// This constructor is required because the struct is `#[non_exhaustive]`.
    #[inline]
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "domain struct requires all fields"
    )]
    pub fn new(
        id: TransactionId,
        date: Date,
        payee: Option<String>,
        description: String,
        postings: Vec<Posting>,
        status: TransactionStatus,
        tags: Vec<String>,
        created_at: Timestamp,
    ) -> Self {
        Self {
            id,
            date,
            payee,
            description,
            postings,
            status,
            tags,
            created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::{
        AccountId,
        ids::PostingId,
        money::{Amount, CommodityCode},
    };

    #[test]
    fn transaction_status_variants_exist() {
        _ = (
            TransactionStatus::Pending,
            TransactionStatus::Cleared,
            TransactionStatus::Voided,
        );
    }

    #[test]
    fn posting_stores_amount() {
        let posting = Posting {
            id: PostingId::new(),
            account_id: AccountId::new(),
            amount: Amount::new(dec!(100.00), CommodityCode::new("AUD")),
            memo: None,
        };
        assert_eq!(posting.amount.commodity.to_string(), "AUD");
    }
}
