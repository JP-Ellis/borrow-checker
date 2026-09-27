//! Pure layout for the budget bar: segment widths, tick positions, glyphs.

use bc_ipc::BudgetIntent;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

/// Widths (percent of the bar) of the three stacked segments.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Segments {
    /// Spend owned by the budget or its sub-budgets.
    pub claimed: u32,
    /// An envelope's spend no sub-budget claims.
    pub unallocated: u32,
    /// Spend no budget covers.
    pub unbudgeted: u32,
    /// `true` when the segments would run past the bar's end.
    pub overflows: bool,
}

/// Percent of the bar a `part` of `target` occupies; 100% of target = 80% of the bar.
///
/// A part whose sign opposes the target's (a net refund against a Limit)
/// makes no progress towards it and draws nothing.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "target is checked non-zero by the only caller; the product is bounded by realistic budget magnitudes"
)]
fn width(part: Decimal, target: Decimal) -> u32 {
    if part.is_sign_negative() != target.is_sign_negative() {
        return 0;
    }
    (part.abs() * Decimal::from(80_u32) / target.abs())
        .round()
        .to_u32()
        .unwrap_or(u32::MAX)
}

/// Lays out the stacked segments against `target`.
#[must_use]
pub(crate) fn segments(
    target: Option<Decimal>,
    claimed: Decimal,
    unallocated: Decimal,
    unbudgeted: Decimal,
) -> Segments {
    let Some(nonzero_target) = target.filter(|value| !value.is_zero()) else {
        return Segments::default();
    };
    let mut room = 100_u32;
    let mut take = |w: u32| {
        let got = w.min(room);
        room = room.saturating_sub(got);
        got
    };
    let raw = [
        width(claimed, nonzero_target),
        width(unallocated, nonzero_target),
        width(unbudgeted, nonzero_target),
    ];
    let [claimed_w, unallocated_w, unbudgeted_w] = raw;
    let overflows = raw.iter().fold(0_u32, |a, &w| a.saturating_add(w)) > 100;
    Segments {
        claimed: take(claimed_w),
        unallocated: take(unallocated_w),
        unbudgeted: take(unbudgeted_w),
        overflows,
    }
}

/// Bar position of a fraction of the target (the pace tick).
#[expect(
    clippy::arithmetic_side_effects,
    reason = "fraction is a bounded 0..=1 ratio; the product cannot overflow"
)]
#[must_use]
pub(crate) fn tick_percent(fraction: Decimal) -> u32 {
    (fraction * Decimal::from(80_u32))
        .round()
        .to_u32()
        .unwrap_or(0)
        .min(100)
}

/// The glyph shown beside a target for its intent.
#[must_use]
pub(crate) fn intent_glyph(intent: Option<BudgetIntent>) -> &'static str {
    match intent {
        Some(BudgetIntent::Limit) => "\u{2264}",
        Some(BudgetIntent::Goal) => "\u{2265}",
        Some(BudgetIntent::Estimate) => "\u{2248}",
        _ => "",
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn envelope_segments_stack() {
        // Food: target 800, claimed 581, unallocated 31
        let s = segments(Some(dec!(800)), dec!(581), dec!(31), dec!(0));
        assert_eq!(
            (s.claimed, s.unallocated, s.unbudgeted, s.overflows),
            (58, 3, 0, false)
        );
    }

    #[test]
    fn unbudgeted_overflow_is_clamped() {
        // Pets: target 80, claimed 95, unbudgeted 120
        let s = segments(Some(dec!(80)), dec!(95), dec!(0), dec!(120));
        assert_eq!((s.claimed, s.unbudgeted, s.overflows), (95, 5, true));
    }

    #[test]
    fn negative_targets_use_magnitudes() {
        let s = segments(Some(dec!(-493.15)), dec!(-222), dec!(0), dec!(0));
        assert_eq!(s.claimed, 36);
    }

    #[rstest]
    #[case::income_against_expense_sign(dec!(-500), dec!(500), dec!(0), dec!(0), (0, 0, 0))]
    #[case::net_refund_on_a_limit(dec!(300), dec!(-50), dec!(0), dec!(0), (0, 0, 0))]
    #[case::only_the_opposing_segment_vanishes(dec!(400), dec!(200), dec!(-40), dec!(40), (40, 0, 8))]
    fn segments_opposing_the_target_draw_nothing(
        #[case] target: Decimal,
        #[case] claimed: Decimal,
        #[case] unallocated: Decimal,
        #[case] unbudgeted: Decimal,
        #[case] expected: (u32, u32, u32),
    ) {
        let s = segments(Some(target), claimed, unallocated, unbudgeted);
        assert_eq!((s.claimed, s.unallocated, s.unbudgeted), expected);
    }

    #[test]
    fn no_target_draws_nothing() {
        let s = segments(None, dec!(50), dec!(0), dec!(0));
        assert_eq!((s.claimed, s.unallocated, s.unbudgeted), (0, 0, 0));
    }

    #[test]
    fn ticks_and_glyphs() {
        assert_eq!(tick_percent(dec!(1)), 80);
        assert_eq!(tick_percent(dec!(0.5)), 40);
        assert_eq!(intent_glyph(Some(BudgetIntent::Limit)), "\u{2264}");
        assert_eq!(intent_glyph(Some(BudgetIntent::Goal)), "\u{2265}");
        assert_eq!(intent_glyph(Some(BudgetIntent::Estimate)), "\u{2248}");
        assert_eq!(intent_glyph(None), "");
    }
}
