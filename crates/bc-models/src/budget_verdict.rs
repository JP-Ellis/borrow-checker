//! Budget intent and the traffic-light verdict it implies.

use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::AccountType;

/// What a budget's target is for, which decides whether overshooting is good.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BudgetIntent {
    /// Stay within the target: spending, a drawdown.
    Limit,
    /// Reach at least the target: savings, expected income.
    Goal,
    /// Land near the target: interest accrual, a known bill.
    Estimate,
}

/// A row's traffic light. Ordered so that `max` is the worst.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verdict {
    /// On track.
    Good,
    /// Close to the edge of the band.
    Warn,
    /// Outside the band.
    Bad,
}

impl BudgetIntent {
    /// The intent a new budget on an account of `account_type` starts with.
    #[inline]
    #[must_use]
    pub fn default_for(account_type: AccountType) -> Self {
        if account_type == AccountType::Expense {
            Self::Limit
        } else {
            Self::Goal
        }
    }

    /// Classifies `ratio = actual ÷ reference` into this intent's bands.
    #[inline]
    #[must_use]
    pub fn verdict(self, ratio: Decimal) -> Verdict {
        let warn_low = Decimal::new(85, 2);
        let one = Decimal::ONE;
        match self {
            Self::Limit if ratio > one => Verdict::Bad,
            Self::Limit if ratio >= warn_low => Verdict::Warn,
            Self::Limit => Verdict::Good,
            Self::Goal if ratio >= one => Verdict::Good,
            Self::Goal if ratio >= warn_low => Verdict::Warn,
            Self::Estimate if (Decimal::new(95, 2)..=Decimal::new(105, 2)).contains(&ratio) => {
                Verdict::Good
            }
            Self::Estimate if (warn_low..=Decimal::new(115, 2)).contains(&ratio) => Verdict::Warn,
            Self::Goal | Self::Estimate => Verdict::Bad,
        }
    }
}

/// Days of `[start, end)` elapsed by the end of `today`, and the window's length.
///
/// `None` before the window opens; `(1, 1)` once it has closed (or the window
/// is empty). Today counts as elapsed, so an open window never yields zero.
fn elapsed_days(start: Date, end: Date, today: Date) -> Option<(Decimal, Decimal)> {
    if today < start {
        return None;
    }
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "Date subtraction is bounded by calendar range"
    )]
    let total = (end - start).get_days();
    if total <= 0_i32 || today >= end {
        return Some((Decimal::ONE, Decimal::ONE));
    }
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "Date subtraction is bounded by calendar range"
    )]
    let elapsed = (today - start).get_days();
    Some((
        Decimal::from(elapsed.saturating_add(1)),
        Decimal::from(total),
    ))
}

/// Fraction of `[start, end)` elapsed by the end of `today`.
///
/// `None` before the window opens; `1` once it has closed.
#[inline]
#[must_use]
pub fn elapsed_fraction(start: Date, end: Date, today: Date) -> Option<Decimal> {
    elapsed_days(start, end, today).map(|(elapsed, total)| {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Decimal division; total is guarded to be non-zero by elapsed_days"
        )]
        {
            elapsed / total
        }
    })
}

/// The share of `target` a budget should have reached by the end of `today`.
///
/// Multiplies before dividing, so `300 × 1 ÷ 30` is exactly `10`; dividing
/// the fraction first would give `9.99…`.
#[inline]
#[must_use]
pub fn pace_reference(target: Decimal, start: Date, end: Date, today: Date) -> Option<Decimal> {
    elapsed_days(start, end, today).map(|(elapsed, total)| {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Decimal multiplication/division; total is guarded to be non-zero by elapsed_days"
        )]
        {
            target * elapsed / total
        }
    })
}

/// Verdict for `actual` against `reference`; `None` when there is no usable
/// reference or the ratio overflows.
#[inline]
#[must_use]
pub fn verdict_for(
    intent: BudgetIntent,
    actual: Decimal,
    reference: Option<Decimal>,
) -> Option<Verdict> {
    let nonzero_reference = reference.filter(|r| !r.is_zero())?;
    actual
        .checked_div(nonzero_reference)
        .map(|ratio| intent.verdict(ratio))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::AccountType;

    #[rstest]
    #[case::limit_under(BudgetIntent::Limit, dec!(0.8499), Verdict::Good)]
    #[case::limit_warn_low(BudgetIntent::Limit, dec!(0.85), Verdict::Warn)]
    #[case::limit_warn_high(BudgetIntent::Limit, dec!(1.00), Verdict::Warn)]
    #[case::limit_over(BudgetIntent::Limit, dec!(1.0001), Verdict::Bad)]
    #[case::limit_reversed(BudgetIntent::Limit, dec!(-1), Verdict::Good)]
    #[case::goal_short(BudgetIntent::Goal, dec!(0.8499), Verdict::Bad)]
    #[case::goal_warn_low(BudgetIntent::Goal, dec!(0.85), Verdict::Warn)]
    #[case::goal_warn_high(BudgetIntent::Goal, dec!(0.9999), Verdict::Warn)]
    #[case::goal_met(BudgetIntent::Goal, dec!(1.00), Verdict::Good)]
    #[case::goal_exceeded(BudgetIntent::Goal, dec!(2), Verdict::Good)]
    #[case::goal_reversed(BudgetIntent::Goal, dec!(-0.5), Verdict::Bad)]
    #[case::est_far_low(BudgetIntent::Estimate, dec!(0.8499), Verdict::Bad)]
    #[case::est_warn_low_edge(BudgetIntent::Estimate, dec!(0.85), Verdict::Warn)]
    #[case::est_warn_low(BudgetIntent::Estimate, dec!(0.9499), Verdict::Warn)]
    #[case::est_good_low(BudgetIntent::Estimate, dec!(0.95), Verdict::Good)]
    #[case::est_good_high(BudgetIntent::Estimate, dec!(1.05), Verdict::Good)]
    #[case::est_warn_high(BudgetIntent::Estimate, dec!(1.0501), Verdict::Warn)]
    #[case::est_warn_high_edge(BudgetIntent::Estimate, dec!(1.15), Verdict::Warn)]
    #[case::est_far_high(BudgetIntent::Estimate, dec!(1.1501), Verdict::Bad)]
    #[case::est_reversed(BudgetIntent::Estimate, dec!(-1), Verdict::Bad)]
    fn bands(#[case] intent: BudgetIntent, #[case] ratio: Decimal, #[case] expected: Verdict) {
        assert_eq!(intent.verdict(ratio), expected);
    }

    #[rstest]
    #[case(AccountType::Expense, BudgetIntent::Limit)]
    #[case(AccountType::Income, BudgetIntent::Goal)]
    #[case(AccountType::Asset, BudgetIntent::Goal)]
    #[case(AccountType::Liability, BudgetIntent::Goal)]
    #[case(AccountType::Equity, BudgetIntent::Goal)]
    fn defaults(#[case] ty: AccountType, #[case] expected: BudgetIntent) {
        assert_eq!(BudgetIntent::default_for(ty), expected);
    }

    const SEP_1: Date = Date::constant(2026, 9, 1);
    const OCT_1: Date = Date::constant(2026, 10, 1);

    #[rstest]
    #[case::closed(Date::constant(2026, 10, 5), Some(dec!(300)))]
    #[case::closed_on_end(OCT_1, Some(dec!(300)))]
    #[case::first_day(SEP_1, Some(dec!(10)))]
    #[case::mid(Date::constant(2026, 9, 15), Some(dec!(150)))]
    #[case::last_day(Date::constant(2026, 9, 30), Some(dec!(300)))]
    #[case::future(Date::constant(2026, 8, 31), None)]
    fn pace(#[case] today: Date, #[case] expected: Option<Decimal>) {
        assert_eq!(pace_reference(dec!(300), SEP_1, OCT_1, today), expected);
    }

    #[rstest]
    #[case::normal(dec!(50), Some(dec!(100)), Some(Verdict::Good))]
    #[case::zero_reference(dec!(50), Some(dec!(0)), None)]
    #[case::no_reference(dec!(50), None, None)]
    #[case::negative_target(dec!(-500), Some(dec!(-500)), Some(Verdict::Warn))]
    #[case::overflow(Decimal::MAX, Some(dec!(0.1)), None)]
    fn verdict_for_handles_edges(
        #[case] actual: Decimal,
        #[case] reference: Option<Decimal>,
        #[case] expected: Option<Verdict>,
    ) {
        assert_eq!(
            verdict_for(BudgetIntent::Limit, actual, reference),
            expected
        );
    }

    #[test]
    fn worst_is_max() {
        assert_eq!(
            [Verdict::Good, Verdict::Bad, Verdict::Warn]
                .into_iter()
                .max(),
            Some(Verdict::Bad)
        );
    }
}
