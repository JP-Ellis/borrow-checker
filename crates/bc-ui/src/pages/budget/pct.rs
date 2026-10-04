//! The percent-mode ACTUAL label shared by budget rows and native sub-rows.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

/// The ACTUAL cell in percent mode: core's paced `ratio` as an integer
/// percentage, or an en dash when core sent none.
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "ratio is bounded by budget magnitudes; the product cannot overflow"
)]
pub(crate) fn pct_label(ratio: Option<Decimal>) -> String {
    ratio.map_or_else(
        || "\u{2013}".into(),
        |r| {
            let pct = (r * Decimal::from(100_u32)).round().to_i64().unwrap_or(0);
            format!("{pct}%")
        },
    )
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::pct_label;

    #[rstest]
    #[case(Some(Decimal::new(9, 1)), "90%")]
    #[case(Some(Decimal::new(1005, 3)), "100%")]
    #[case(Some(Decimal::new(-5, 1)), "-50%")]
    #[case(None, "\u{2013}")]
    fn pct_reads_core_ratio(#[case] ratio: Option<Decimal>, #[case] expected: &str) {
        assert_eq!(pct_label(ratio), expected);
    }
}
