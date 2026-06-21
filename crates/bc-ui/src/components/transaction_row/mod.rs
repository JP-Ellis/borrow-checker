//! Shared, posting-aware transaction row used by the accounts and budget pages.

use std::collections::BTreeMap;

use bc_ipc::Amount;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use rust_decimal::Decimal;

/// Determines which postings are focal and how the headline amount is derived.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
// Items in this module are consumed by the transaction-row component added in Task 3.
// Until then they are only referenced from native unit tests, so wasm clippy sees them as dead.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by the transaction-row component added in Task 3"
    )
)]
pub enum RowPerspective {
    /// Accounts page: focal postings are those on `account_id`; headline is
    /// their net sum.
    Account {
        /// The account currently in view.
        account_id: String,
    },
    /// Budget page: focal postings are those on `account_id`; headline is their
    /// period/spread-prorated sum over `[window_start, window_end]`.
    ///
    /// `tag_filter` is carried for future tag-filter narrowing; until tag paths
    /// are resolved through IPC it is unused for matching (see future-works).
    Budget {
        /// The account this budget targets.
        account_id: String,
        /// Optional tag-filter path for a sub-budget (currently informational).
        tag_filter: Option<String>,
        /// Inclusive start of the displayed budget period.
        window_start: jiff::civil::Date,
        /// Inclusive end of the displayed budget period.
        window_end: jiff::civil::Date,
    },
    /// Fallback: headline is the one-sided sum of positive postings.
    Global,
}

/// Returns the focal postings for `account_id` within `tx`.
///
/// # Arguments
///
/// * `tx` - The transaction to search.
/// * `account_id` - The account ID to match against posting accounts.
///
/// # Returns
///
/// An iterator over postings whose account ID matches `account_id`.
pub fn focal_on_account<'a>(
    tx: &'a Transaction,
    account_id: &'a str,
) -> impl Iterator<Item = &'a Posting> {
    tx.postings
        .iter()
        .filter(move |p| p.account.id == account_id)
}

/// Computes the headline [`Amount`] for `tx` under `perspective`.
///
/// Returns an `Amount` with an empty currency code (rendered as `—`) when no
/// focal posting carries a concrete amount.
///
/// # Arguments
///
/// * `tx` - The transaction to compute a headline for.
/// * `perspective` - Determines which postings are focal and how the amount is derived.
///
/// # Returns
///
/// The headline [`Amount`] for the given perspective.
#[must_use]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by the transaction-row component added in Task 3"
    )
)]
pub fn headline_amount(tx: &Transaction, perspective: &RowPerspective) -> Amount {
    match perspective {
        RowPerspective::Account { account_id } => {
            sum_focal(focal_on_account(tx, account_id).filter_map(|p| p.amount.as_ref()))
        }
        RowPerspective::Budget {
            account_id,
            window_start,
            window_end,
            ..
        } => {
            let focal: Vec<&Posting> = focal_on_account(tx, account_id)
                .filter(|p| p.amount.is_some())
                .collect();
            let currency = focal
                .first()
                .and_then(|p| p.amount.as_ref())
                .map_or("", |a| a.currency_code.as_str());
            let total: Decimal = focal
                .iter()
                .map(|p| prorated_value(p, *window_start, *window_end))
                .sum();
            Amount::new(total, currency)
        }
        RowPerspective::Global => sum_focal(
            tx.postings
                .iter()
                .filter_map(|p| p.amount.as_ref())
                .filter(|a| a.value > Decimal::ZERO),
        ),
    }
}

/// Sums a sequence of amounts, taking the currency from the first one.
///
/// Returns an [`Amount`] with zero value and empty currency code when the
/// iterator is empty.
fn sum_focal<'a>(mut amounts: impl Iterator<Item = &'a Amount>) -> Amount {
    let Some(first) = amounts.next() else {
        return Amount::new(Decimal::ZERO, "");
    };
    let currency = first.currency_code.clone();
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "summing amounts of the same commodity within a transaction; overflow not reachable in practice"
    )]
    let total = amounts.fold(first.value, |acc, a| acc + a.value);
    Amount::new(total, currency)
}

/// Returns the contribution of `p` to the period `[window_start, window_end]`.
///
/// A posting with a `spread_from`/`spread_until` range contributes its value
/// scaled by the fraction of spread days that fall inside the window. A posting
/// with no full spread range contributes its whole value.
///
/// # Arguments
///
/// * `p` - The posting to prorate.
/// * `window_start` - Inclusive start of the window.
/// * `window_end` - Inclusive end of the window.
///
/// # Returns
///
/// The prorated decimal value for the given window.
#[must_use]
pub fn prorated_value(
    p: &Posting,
    window_start: jiff::civil::Date,
    window_end: jiff::civil::Date,
) -> Decimal {
    let Some(value) = p.amount.as_ref().map(|a| a.value) else {
        return Decimal::ZERO;
    };
    let (Some(from), Some(until)) = (p.spread_from, p.spread_until) else {
        return value;
    };
    let total_days = inclusive_days(from, until);
    if total_days <= 0 {
        return value;
    }
    let overlap_start = from.max(window_start);
    let overlap_end = until.min(window_end);
    let overlap_days = inclusive_days(overlap_start, overlap_end).max(0);
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "proration arithmetic: Decimal multiplication and division by bounded day counts; practical values never overflow"
    )]
    {
        value * Decimal::from(overlap_days) / Decimal::from(total_days)
    }
}

/// Returns the inclusive day count between two civil dates (`a`..=`b`).
///
/// Returns `0` when `b < a`.
fn inclusive_days(a: jiff::civil::Date, b: jiff::civil::Date) -> i64 {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "jiff Date subtraction returns a Span bounded by calendar range; +1 for inclusive count cannot overflow i64"
    )]
    {
        let days = i64::from((b - a).get_days());
        if days < 0 { 0 } else { days + 1 }
    }
}

/// Returns whether `tx` is structurally balanced.
///
/// Mirrors `bc_models::Transaction::balanced`: false with no concrete legs or
/// two-or-more elided legs; a single elided leg auto-balances; otherwise every
/// commodity's concrete legs must sum to zero.
///
/// # Arguments
///
/// * `tx` - The transaction to check.
///
/// # Returns
///
/// `true` if the transaction is balanced, `false` otherwise.
#[must_use]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed by the transaction-row component added in Task 3"
    )
)]
pub fn is_balanced(tx: &Transaction) -> bool {
    let elided = tx.postings.iter().filter(|p| p.amount.is_none()).count();
    if elided >= 2 {
        return false;
    }
    let mut totals: BTreeMap<&str, Decimal> = BTreeMap::new();
    for a in tx.postings.iter().filter_map(|p| p.amount.as_ref()) {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "balance check: summing monetary values of the same commodity within a single transaction"
        )]
        {
            *totals.entry(a.currency_code.as_str()).or_default() += a.value;
        }
    }
    if totals.is_empty() {
        return false;
    }
    if elided == 1 {
        return true;
    }
    totals.values().all(Decimal::is_zero)
}

#[cfg(test)]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Amount;
    use bc_ipc::Posting;
    use bc_ipc::Reconciliation;
    use bc_ipc::Transaction;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::RowPerspective;
    use super::headline_amount;
    use super::is_balanced;
    use super::prorated_value;

    fn posting(id: &str, acct: &str, minor: Option<i64>) -> Posting {
        Posting::new(
            id,
            AccountRef::new(acct, acct),
            minor.map(|m| Amount::new(Decimal::new(m, 2), "AUD")),
            None::<&str>,
            vec![],
            None,
            None,
        )
    }

    fn tx(postings: Vec<Posting>) -> Transaction {
        Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            "Coles",
            "",
            None::<&str>,
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            postings,
            vec![],
        )
    }

    #[test]
    fn account_headline_sums_focal_postings() {
        let t = tx(vec![
            posting("a", "checking", Some(-8_420)),
            posting("b", "groceries", Some(8_420)),
        ]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_id: "checking".to_owned(),
            },
        );
        assert_eq!(amt.value, Decimal::new(-8_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn account_headline_unknown_account_is_empty() {
        let t = tx(vec![posting("a", "checking", Some(-8_420))]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Account {
                account_id: "savings".to_owned(),
            },
        );
        assert_eq!(amt.value, Decimal::ZERO);
        assert_eq!(amt.currency_code, "");
    }

    #[test]
    fn global_headline_sums_positive_legs() {
        let t = tx(vec![
            posting("a", "checking", Some(-8_420)),
            posting("b", "groceries", Some(8_420)),
        ]);
        let amt = headline_amount(&t, &RowPerspective::Global);
        assert_eq!(amt.value, Decimal::new(8_420, 2));
        assert_eq!(amt.currency_code, "AUD");
    }

    #[test]
    fn balanced_zero_sum_is_true() {
        let t = tx(vec![
            posting("a", "checking", Some(-8_420)),
            posting("b", "groceries", Some(8_420)),
        ]);
        assert!(is_balanced(&t));
    }

    #[test]
    fn one_sided_import_is_unbalanced() {
        let t = tx(vec![posting("a", "checking", Some(-5_000))]);
        assert!(!is_balanced(&t));
    }

    #[test]
    fn single_elided_leg_is_balanced() {
        let t = tx(vec![
            posting("a", "checking", Some(-5_000)),
            posting("b", "groceries", None),
        ]);
        assert!(is_balanced(&t));
    }

    #[test]
    fn two_elided_legs_is_unbalanced() {
        let t = tx(vec![
            posting("a", "checking", None),
            posting("b", "groceries", None),
        ]);
        assert!(!is_balanced(&t));
    }

    #[test]
    fn prorate_full_overlap_returns_full_value() {
        let mut p = posting("a", "insurance", Some(12_000));
        p.spread_from = Some(Date::constant(2026, 1, 1));
        p.spread_until = Some(Date::constant(2026, 1, 31));
        let v = prorated_value(&p, Date::constant(2026, 1, 1), Date::constant(2026, 1, 31));
        assert_eq!(v, Decimal::new(12_000, 2));
    }

    #[test]
    fn prorate_half_overlap_halves_value() {
        // 30-day spread (Jun 1-30); window covers Jun 1-15 = 15 of 30 days.
        let mut p = posting("a", "insurance", Some(30_000));
        p.spread_from = Some(Date::constant(2026, 6, 1));
        p.spread_until = Some(Date::constant(2026, 6, 30));
        let v = prorated_value(&p, Date::constant(2026, 6, 1), Date::constant(2026, 6, 15));
        assert_eq!(v, Decimal::new(15_000, 2));
    }

    #[test]
    fn prorate_no_spread_returns_full_value_inside_window() {
        let p = posting("a", "groceries", Some(8_420));
        let v = prorated_value(&p, Date::constant(2026, 4, 1), Date::constant(2026, 4, 30));
        assert_eq!(v, Decimal::new(8_420, 2));
    }

    #[test]
    fn budget_headline_prorates_spread_postings() {
        let mut p = posting("a", "insurance", Some(30_000));
        p.spread_from = Some(Date::constant(2026, 6, 1));
        p.spread_until = Some(Date::constant(2026, 6, 30));
        let t = tx(vec![p, posting("b", "expenses", Some(-30_000))]);
        let amt = headline_amount(
            &t,
            &RowPerspective::Budget {
                account_id: "insurance".to_owned(),
                tag_filter: None,
                window_start: Date::constant(2026, 6, 1),
                window_end: Date::constant(2026, 6, 15),
            },
        );
        // 15 of 30 days → half value
        assert_eq!(amt.value, Decimal::new(15_000, 2));
        assert_eq!(amt.currency_code, "AUD");
    }
}
