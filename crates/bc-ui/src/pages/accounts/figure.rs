//! Pure selection of the balance a sidebar row shows: the account's own
//! balance or its subtree roll-up. Kept target-agnostic so it is
//! native-testable.

use bc_ipc::Amount;
use rust_decimal::Decimal;

/// The balance a sidebar row displays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowFigure {
    /// The amount shown, or `None` for the placeholder dash.
    pub amount: Option<Amount>,
    /// Roll-up commodities beyond the one shown; drives the `+N` badge.
    pub extra: usize,
}

impl RowFigure {
    /// Whether the shown amount is below zero.
    #[must_use]
    pub fn is_negative(&self) -> bool {
        self.amount
            .as_ref()
            .is_some_and(|a| a.value < Decimal::ZERO)
    }
}

/// Picks the figure for a row.
///
/// With `include_descendants` the first roll-up entry is shown and the
/// remaining entries are counted as extra; without it the account's own
/// balance is shown and nothing is extra.
///
/// # Arguments
///
/// * `own` - The account's own default-commodity balance.
/// * `rollup` - The subtree roll-up, default commodity first.
/// * `include_descendants` - The page-level "include sub-accounts" toggle.
#[must_use]
pub fn row_figure(own: Option<&Amount>, rollup: &[Amount], include_descendants: bool) -> RowFigure {
    if include_descendants {
        RowFigure {
            amount: rollup.first().cloned(),
            extra: rollup.len().saturating_sub(1),
        }
    } else {
        RowFigure {
            amount: own.cloned(),
            extra: 0,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn amt(value: i64, code: &str) -> Amount {
        Amount::new(Decimal::new(value, 2), code)
    }

    #[rstest]
    #[case::own_only(Some(amt(1_000, "AUD")), vec![], false, Some(amt(1_000, "AUD")), 0)]
    #[case::own_ignores_rollup(
        Some(amt(1_000, "AUD")),
        vec![amt(5_000, "AUD"), amt(200, "USD"), amt(300, "EUR")],
        false,
        Some(amt(1_000, "AUD")),
        0
    )]
    #[case::rollup_first_and_extra(
        Some(amt(1_000, "AUD")),
        vec![amt(5_000, "AUD"), amt(200, "USD"), amt(300, "EUR")],
        true,
        Some(amt(5_000, "AUD")),
        2
    )]
    #[case::rollup_single(None, vec![amt(5_000, "AUD")], true, Some(amt(5_000, "AUD")), 0)]
    #[case::rollup_empty(Some(amt(1_000, "AUD")), vec![], true, None, 0)]
    #[case::nothing(None, vec![], false, None, 0)]
    fn picks_own_or_rollup(
        #[case] own: Option<Amount>,
        #[case] rollup: Vec<Amount>,
        #[case] include_descendants: bool,
        #[case] amount: Option<Amount>,
        #[case] extra: usize,
    ) {
        assert_eq!(
            row_figure(own.as_ref(), &rollup, include_descendants),
            RowFigure { amount, extra }
        );
    }

    #[rstest]
    #[case::negative(Some(amt(-1, "AUD")), true)]
    #[case::zero(Some(amt(0, "AUD")), false)]
    #[case::positive(Some(amt(1, "AUD")), false)]
    #[case::absent(None, false)]
    fn negative_flag(#[case] amount: Option<Amount>, #[case] expected: bool) {
        assert_eq!(RowFigure { amount, extra: 0 }.is_negative(), expected);
    }
}
