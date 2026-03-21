//! Transaction and posting domain types.

use jiff::{civil::Date, Timestamp};

use crate::{
    ids::{AccountId, PostingId, TransactionId},
    money::Amount,
};

/// The lifecycle status of a transaction.
#[expect(
    clippy::module_name_repetitions,
    reason = "TransactionStatus is the canonical domain name regardless of module path"
)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ids::{AccountId, PostingId},
        money::{Amount, CommodityCode},
    };
    use rust_decimal_macros::dec;

    #[test]
    fn transaction_status_variants_exist() {
        let _p = TransactionStatus::Pending;
        let _c = TransactionStatus::Cleared;
        let _v = TransactionStatus::Voided;
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
