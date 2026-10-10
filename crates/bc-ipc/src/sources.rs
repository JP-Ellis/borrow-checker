//! Delete outcome, provenance and rejected-source DTOs.

use jiff::civil::Date;

/// What a transaction delete did to the transaction's import provenance.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct DeleteOutcome {
    /// Imported legs whose references were kept, so a re-import skips them.
    pub references_kept: u64,
    /// Imported legs whose references were dropped, so a re-import offers
    /// them again.
    pub references_forgotten: u64,
}

impl DeleteOutcome {
    /// Creates a [`DeleteOutcome`].
    ///
    /// # Arguments
    ///
    /// * `references_kept` - Imported legs whose references were kept.
    /// * `references_forgotten` - Imported legs whose references were dropped.
    #[must_use]
    #[inline]
    pub const fn new(references_kept: u64, references_forgotten: u64) -> Self {
        Self {
            references_kept,
            references_forgotten,
        }
    }
}

/// How much import provenance a transaction carries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TransactionProvenance {
    /// Imported legs attached to the transaction, one reference each.
    pub rows: u64,
    /// Names of the accounts those legs were imported into.
    pub accounts: Vec<String>,
}

impl TransactionProvenance {
    /// Creates a [`TransactionProvenance`].
    ///
    /// # Arguments
    ///
    /// * `rows` - Imported legs attached to the transaction.
    /// * `accounts` - Names of the accounts those legs were imported into.
    #[must_use]
    #[inline]
    pub const fn new(rows: u64, accounts: Vec<String>) -> Self {
        Self { rows, accounts }
    }
}

/// One rejected statement leg: a source reference with no live posting.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct RejectedLeg {
    /// The `source_ref_…` ID to release.
    pub reference: String,
    /// Name of the account the statement row was on.
    pub account: String,
    /// The statement row's date.
    pub date: Date,
    /// The statement row's narration.
    pub narration: String,
    /// The statement row's amount as a decimal string; `None` for an elided leg.
    pub amount: Option<String>,
    /// The amount's commodity code; `None` for an elided leg.
    pub commodity: Option<String>,
}

impl RejectedLeg {
    /// Creates a [`RejectedLeg`].
    ///
    /// # Arguments
    ///
    /// * `reference` - The `source_ref_…` ID to release.
    /// * `account` - Name of the account the row was on.
    /// * `date` - The row's date.
    /// * `narration` - The row's narration.
    /// * `value` - The row's decimal amount and commodity code; `None` for an elided leg.
    #[must_use]
    #[inline]
    pub fn new(
        reference: String,
        account: String,
        date: Date,
        narration: String,
        value: Option<(String, String)>,
    ) -> Self {
        let (amount, commodity) = value.unzip();
        Self {
            reference,
            account,
            date,
            narration,
            amount,
            commodity,
        }
    }
}

/// A rejected statement row, as the user would release it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RejectedRow {
    /// A leg deleted from a transaction that still exists.
    Leg {
        /// The live transaction the leg was deleted from.
        transaction_id: String,
        /// The deleted leg.
        leg: RejectedLeg,
    },
    /// Every leg of a deleted transaction.
    Transaction {
        /// The deleted transaction's ID.
        deleted_transaction_id: String,
        /// Its rejected legs.
        legs: Vec<RejectedLeg>,
    },
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;

    use super::*;

    #[test]
    fn a_rejected_transaction_row_carries_its_kind_tag() {
        let row = RejectedRow::Transaction {
            deleted_transaction_id: "transaction_1".to_owned(),
            legs: vec![RejectedLeg::new(
                "source_ref_1".to_owned(),
                "Checking".to_owned(),
                date(2026, 1, 3),
                "COFFEE".to_owned(),
                Some(("-4.50".to_owned(), "AUD".to_owned())),
            )],
        };
        insta::assert_json_snapshot!(row);
    }
}
