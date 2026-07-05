//! Transfer resolution: merge/unmerge two single-posting transactions and
//! suggest candidate transfer pairs.

use bc_models::Transaction;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// Merges, unmerges, and suggests transfer pairs.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Service {
    /// SQLite connection pool.
    #[expect(dead_code, reason = "read by merge/unmerge methods added in Tasks 4-5")]
    pool: SqlitePool,
}

impl Service {
    /// Creates a new transfer service.
    ///
    /// # Arguments
    ///
    /// * `pool` - A SQLite connection pool.
    #[inline]
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

/// Validates that two transactions may be merged.
///
/// Each must have exactly one concrete posting; the two postings must share a
/// commodity and be equal in magnitude and opposite in sign.
///
/// # Arguments
///
/// * `survivor` - The transaction that will survive the merge.
/// * `absorbed` - The transaction that will be fused into the survivor.
///
/// # Returns
///
/// `Ok(())` if the pair may be merged.
///
/// # Errors
///
/// Returns [`BcError::NotMergeable`] describing the first failed precondition.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "called by the merge method added in Task 4; already exercised by unit tests"
    )
)]
fn check_mergeable(survivor: &Transaction, absorbed: &Transaction) -> BcResult<()> {
    let reject = |reason: &str| {
        Err(BcError::NotMergeable {
            reason: reason.to_owned(),
        })
    };

    let (Some(a), Some(b)) = (survivor.postings().first(), absorbed.postings().first()) else {
        return reject("both transactions must have a posting");
    };
    if survivor.postings().len() != 1 || absorbed.postings().len() != 1 {
        return reject("each transaction must have exactly one posting");
    }
    let (Some(amount_a), Some(amount_b)) = (a.amount(), b.amount()) else {
        return reject("both postings must have a concrete amount");
    };
    if amount_a.commodity() != amount_b.commodity() {
        return reject("postings must share a commodity");
    }
    if amount_a.value().is_zero() {
        return reject("posting amount must be non-zero");
    }
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "financial negation: Decimal is bounded by the type"
    )]
    let opposite = amount_a.value() == -amount_b.value();
    if !opposite {
        return reject("postings must be equal and opposite");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use rust_decimal::Decimal;

    use super::*;

    fn tx(_account: &str, amount: i64, commodity: &str) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 6, 27))
            .description("row")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .amount(Amount::new(
                        Decimal::from(amount),
                        CommodityCode::new(commodity),
                    ))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    fn accepts_equal_opposite_same_commodity() {
        check_mergeable(&tx("a", -100, "AUD"), &tx("b", 100, "AUD")).expect("should be mergeable");
    }

    #[test]
    fn rejects_same_sign() {
        assert!(matches!(
            check_mergeable(&tx("a", -100, "AUD"), &tx("b", -100, "AUD")),
            Err(BcError::NotMergeable { .. })
        ));
    }

    #[test]
    fn rejects_unequal_magnitude() {
        assert!(matches!(
            check_mergeable(&tx("a", -100, "AUD"), &tx("b", 90, "AUD")),
            Err(BcError::NotMergeable { .. })
        ));
    }

    #[test]
    fn rejects_different_commodity() {
        assert!(matches!(
            check_mergeable(&tx("a", -100, "AUD"), &tx("b", 100, "USD")),
            Err(BcError::NotMergeable { .. })
        ));
    }

    #[test]
    fn rejects_zero_amount() {
        assert!(matches!(
            check_mergeable(&tx("a", 0, "AUD"), &tx("b", 0, "AUD")),
            Err(BcError::NotMergeable { .. })
        ));
    }
}
