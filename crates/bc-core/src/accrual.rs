//! A spread posting's share of a date range.
//!
//! A posting spread over the inclusive day range `[from, until]` accrues one
//! `N`th of its amount per day, where `N` is the day count. With `C(d)` the
//! running share before day `d`, rounded to `dp` places (the larger of the
//! commodity's precision and the stored amount's scale),
//!
//! ```text
//! C(d)         = round_dp(amount * clamp(days(from, d), 0, N) / N, dp)
//! share([s,e)) = C(e) - C(s)
//! ```
//!
//! Shares are differences of rounded running totals, so they add up exactly
//! to the amount over the whole spread and adjacent ranges add up exactly to
//! their union.

use jiff::civil::Date;
use rust_decimal::Decimal;

/// How a posting's optional spread ends classify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SpreadRule {
    /// Counts on its transaction date: no spread, or only one end set.
    Unspread,
    /// Spreads over a valid inclusive day range.
    Spread(Spread),
    /// Both ends set but inverted: counts toward no period.
    Nowhere,
}

/// An inclusive day range `[from, until]` with `from <= until`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Spread {
    /// First day of the spread.
    pub from: Date,
    /// Last day of the spread (inclusive).
    pub until: Date,
}

/// Whole days from `a` to `b`; negative when `b` precedes `a`.
fn days(a: Date, b: Date) -> i64 {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "Date - Date is bounded by calendar range"
    )]
    i64::from((b - a).get_days())
}

/// `amount * elapsed / total`, or `None` if the result is out of range.
///
/// `amount * elapsed` overflows only for amounts near `Decimal`'s ceiling;
/// then dividing first loses a little precision but cannot overflow.
fn prorate(amount: Decimal, elapsed: Decimal, total: Decimal) -> Option<Decimal> {
    if let Some(product) = amount.checked_mul(elapsed) {
        return product.checked_div(total);
    }
    amount.checked_div(total)?.checked_mul(elapsed)
}

impl Spread {
    /// Classifies a posting's `spread_from` / `spread_until` pair.
    #[must_use]
    #[expect(
        clippy::new_ret_no_self,
        reason = "the constructor classifies the pair before building a Spread"
    )]
    pub(crate) fn new(from: Option<Date>, until: Option<Date>) -> SpreadRule {
        match (from, until) {
            (Some(first), Some(last)) if first <= last => SpreadRule::Spread(Self {
                from: first,
                until: last,
            }),
            (Some(_), Some(_)) => SpreadRule::Nowhere,
            _ => SpreadRule::Unspread,
        }
    }

    /// Number of days in the spread, `N`.
    fn day_count(self) -> i64 {
        days(self.from, self.until).saturating_add(1)
    }

    /// The running share `C(d)` before day `d`.
    fn running(self, amount: Decimal, dp: u32, d: Date) -> Decimal {
        let n = self.day_count();
        let k = days(self.from, d).clamp(0, n);
        if k == 0 {
            return Decimal::ZERO;
        }
        if k == n {
            return amount.round_dp(dp);
        }
        let (total_days, elapsed) = (Decimal::from(n), Decimal::from(k));
        let raw = prorate(amount, elapsed, total_days).unwrap_or(amount);
        raw.round_dp(dp)
    }

    /// The share of `amount` that falls in the half-open range `[start, end)`,
    /// rounded to `dp` decimal places. An empty range has no share.
    #[must_use]
    pub(crate) fn share(self, amount: Decimal, dp: u32, start: Date, end: Date) -> Decimal {
        if end <= start {
            return Decimal::ZERO;
        }
        self.running(amount, dp, end)
            .checked_sub(self.running(amount, dp, start))
            .unwrap_or(Decimal::ZERO)
    }

    /// Whether the spread has any day in the half-open range `[start, end)`.
    #[must_use]
    pub(crate) fn overlaps(self, start: Date, end: Date) -> bool {
        start < end && start <= self.until && end > self.from
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::ToSpan as _;
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use proptest::sample::Index;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    fn d(y: i16, m: i8, day: i8) -> Date {
        Date::new(y, m, day).expect("valid date")
    }

    fn year_2026() -> Spread {
        Spread {
            from: d(2026, 1, 1),
            until: d(2026, 12, 31),
        }
    }

    #[rstest]
    #[case::january(d(2026, 1, 1), d(2026, 2, 1), dec!(101.92))]
    #[case::february(d(2026, 2, 1), d(2026, 3, 1), dec!(92.05))]
    #[case::full_year(d(2026, 1, 1), d(2027, 1, 1), dec!(1200.00))]
    #[case::wider_than_spread(d(2025, 1, 1), d(2028, 1, 1), dec!(1200.00))]
    #[case::before(d(2025, 1, 1), d(2026, 1, 1), dec!(0))]
    #[case::after(d(2027, 1, 1), d(2027, 6, 1), dec!(0))]
    #[case::empty_range(d(2026, 3, 1), d(2026, 3, 1), dec!(0))]
    fn share_of_a_year_spread(#[case] start: Date, #[case] end: Date, #[case] expected: Decimal) {
        assert_eq!(year_2026().share(dec!(1200.00), 2, start, end), expected);
    }

    #[test]
    fn whole_yen_shares_are_33_34_33() {
        let spread = Spread {
            from: d(2026, 1, 1),
            until: d(2026, 1, 3),
        };
        let shares: Vec<Decimal> = (1_i8..=3_i8)
            .map(|day| {
                spread.share(
                    dec!(100),
                    0,
                    d(2026, 1, day),
                    d(2026, 1, day.saturating_add(1)),
                )
            })
            .collect();
        assert_eq!(shares, vec![dec!(33), dec!(34), dec!(33)]);
    }

    #[rstest]
    #[case::both_ordered(Some(d(2026, 1, 1)), Some(d(2026, 1, 1)), true)]
    #[case::one_end(Some(d(2026, 1, 1)), None, false)]
    #[case::neither(None, None, false)]
    fn new_classifies_spread_and_unspread(
        #[case] from: Option<Date>,
        #[case] until: Option<Date>,
        #[case] spreads: bool,
    ) {
        assert_eq!(
            matches!(Spread::new(from, until), SpreadRule::Spread(_)),
            spreads
        );
        if !spreads {
            assert_eq!(Spread::new(from, until), SpreadRule::Unspread);
        }
    }

    #[test]
    fn new_inverted_counts_nowhere() {
        assert_eq!(
            Spread::new(Some(d(2026, 2, 1)), Some(d(2026, 1, 1))),
            SpreadRule::Nowhere
        );
    }

    #[rstest]
    #[case::inside(d(2026, 3, 1), d(2026, 4, 1), true)]
    #[case::ends_before_start(d(2025, 1, 1), d(2026, 1, 1), false)]
    #[case::starts_after_end(d(2027, 1, 1), d(2027, 2, 1), false)]
    #[case::touches_last_day(d(2026, 12, 31), d(2027, 1, 1), true)]
    #[case::empty(d(2026, 3, 1), d(2026, 3, 1), false)]
    fn overlaps_half_open(#[case] start: Date, #[case] end: Date, #[case] expected: bool) {
        assert_eq!(year_2026().overlaps(start, end), expected);
    }

    #[test]
    fn extreme_amount_does_not_panic() {
        let share = year_2026().share(Decimal::MAX, 2, d(2026, 1, 1), d(2026, 2, 1));
        assert!(share > Decimal::ZERO);
    }

    prop_compose! {
        fn spread_and_cuts()(
            start_offset in 0_i32..4_000_i32,
            len in 1_i32..800_i32,
            before in 1_i32..40_i32,
            after in 1_i32..40_i32,
            raw_cuts in prop::collection::vec(any::<Index>(), 0..8),
        ) -> (Spread, Vec<Date>) {
            let from = d(2020, 1, 1).checked_add(start_offset.days()).expect("in range");
            let until = from.checked_add(len.saturating_sub(1).days()).expect("in range");
            let mut offsets: Vec<usize> = raw_cuts
                .iter()
                .map(|cut| cut.index(usize::try_from(len).expect("positive").saturating_add(1)))
                .collect();
            offsets.sort_unstable();
            let mut bounds = vec![from.checked_sub(before.days()).expect("in range")];
            for raw in offsets {
                let cut = i32::try_from(raw).expect("small");
                bounds.push(from.checked_add(cut.days()).expect("in range"));
            }
            bounds.push(until.checked_add(after.days()).expect("in range"));
            (Spread { from, until }, bounds)
        }
    }

    proptest! {
        #[test]
        fn shares_partition_the_amount_exactly(
            (spread, bounds) in spread_and_cuts(),
            mantissa in -1_000_000_000_000_i64..1_000_000_000_000_i64,
            dp in 0_u32..=4_u32,
        ) {
            let amount = Decimal::new(mantissa, dp);
            let total: Decimal = bounds
                .iter()
                .zip(bounds.iter().skip(1))
                .map(|(&start, &end)| spread.share(amount, dp, start, end))
                .sum();
            prop_assert_eq!(total, amount);
        }

        #[test]
        fn adjacent_shares_sum_to_their_union(
            (spread, bounds) in spread_and_cuts(),
            mantissa in -1_000_000_000_000_i64..1_000_000_000_000_i64,
            dp in 0_u32..=4_u32,
        ) {
            let amount = Decimal::new(mantissa, dp);
            for window in bounds.windows(3) {
                if let &[a, b, c] = window {
                    let parts = spread
                        .share(amount, dp, a, b)
                        .checked_add(spread.share(amount, dp, b, c));
                    prop_assert_eq!(parts, Some(spread.share(amount, dp, a, c)));
                }
            }
        }
    }

    #[test]
    fn leap_year_spread_partitions() {
        let spread = Spread {
            from: d(2027, 3, 1),
            until: d(2028, 2, 29),
        };
        let amount = dec!(1000.00);
        let first = spread.share(amount, 2, d(2027, 3, 1), d(2027, 9, 1));
        let second = spread.share(amount, 2, d(2027, 9, 1), d(2028, 3, 1));
        assert_eq!(first.checked_add(second), Some(amount));
    }
}
