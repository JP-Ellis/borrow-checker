//! Pure bar geometry for [`super`]: where the zero line sits and which span the
//! bar fills, as percentages of the cell width.

use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

/// Percentages of the cell width.
#[derive(Clone, Copy, Debug, PartialEq)]
#[expect(
    clippy::struct_field_names,
    reason = "field names are intentionally suffixed with `pct` to denote unit"
)]
#[expect(
    clippy::module_name_repetitions,
    reason = "BarGeometry is the correct name per the task brief"
)]
#[cfg_attr(
    target_arch = "wasm32",
    expect(dead_code, reason = "used by balance_cell component in Task 15")
)]
pub struct BarGeometry {
    /// Position of the zero line.
    pub zero_pct: f64,
    /// Left edge of the bar.
    pub left_pct: f64,
    /// Width of the bar.
    pub width_pct: f64,
}

/// Places `value` on the axis `[lo, hi]`, where `lo ≤ 0 ≤ hi`.
///
/// A positive value fills from the zero line rightward, a negative one
/// leftward. Every output is clamped to `[0, 100]` so a value outside a stale
/// axis (between an append and the axis recompute) cannot overflow the cell.
/// A degenerate axis (`lo == hi`) draws no bar.
///
/// # Arguments
///
/// * `value` - The balance to draw.
/// * `lo` - Axis minimum (≤ 0).
/// * `hi` - Axis maximum (≥ 0).
#[must_use]
#[expect(
    clippy::float_arithmetic,
    reason = "percentages for CSS; exactness is irrelevant at sub-pixel scale"
)]
#[expect(
    clippy::module_name_repetitions,
    reason = "bar_geometry is the correct name per the task brief"
)]
#[cfg_attr(
    target_arch = "wasm32",
    expect(dead_code, reason = "used by balance_cell component in Task 15")
)]
pub fn bar_geometry(value: Decimal, lo: Decimal, hi: Decimal) -> BarGeometry {
    let span = hi.saturating_sub(lo).to_f64().unwrap_or(0.0_f64);
    if span <= 0.0_f64 {
        return BarGeometry {
            zero_pct: 0.0_f64,
            left_pct: 0.0_f64,
            width_pct: 0.0_f64,
        };
    }
    let pct =
        |d: Decimal| (d.to_f64().unwrap_or(0.0_f64) / span * 100.0_f64).clamp(0.0_f64, 100.0_f64);
    let zero_pct = pct(lo.saturating_mul(Decimal::NEGATIVE_ONE));
    let magnitude = pct(value.abs());
    let (left_pct, width_pct) = if value >= Decimal::ZERO {
        let right = (zero_pct + magnitude).min(100.0_f64);
        (zero_pct, right - zero_pct)
    } else {
        let left = (zero_pct - magnitude).max(0.0_f64);
        (left, zero_pct - left)
    };
    BarGeometry {
        zero_pct,
        left_pct,
        width_pct,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    #[rstest]
    // straddling axis −150..100: zero at 60 %
    #[case(dec!(-120), dec!(-150), dec!(100), 60.0_f64, 12.0_f64, 48.0_f64)]
    #[case(dec!(80), dec!(-150), dec!(100), 60.0_f64, 60.0_f64, 32.0_f64)]
    // all-positive axis: zero line at the left edge
    #[case(dec!(50), dec!(0), dec!(200), 0.0_f64, 0.0_f64, 25.0_f64)]
    // all-negative axis: zero line at the right edge
    #[case(dec!(-50), dec!(-200), dec!(0), 100.0_f64, 75.0_f64, 25.0_f64)]
    // value beyond a stale axis clamps to the cell
    #[case(dec!(300), dec!(0), dec!(200), 0.0_f64, 0.0_f64, 100.0_f64)]
    fn geometry(
        #[case] value: rust_decimal::Decimal,
        #[case] lo: rust_decimal::Decimal,
        #[case] hi: rust_decimal::Decimal,
        #[case] zero: f64,
        #[case] left: f64,
        #[case] width: f64,
    ) {
        let g = bar_geometry(value, lo, hi);
        #[expect(
            clippy::float_arithmetic,
            reason = "comparing floating-point test results with tolerance"
        )]
        {
            assert!((g.zero_pct - zero).abs() < 1e-9_f64, "zero {}", g.zero_pct);
            assert!((g.left_pct - left).abs() < 1e-9_f64, "left {}", g.left_pct);
            assert!(
                (g.width_pct - width).abs() < 1e-9_f64,
                "width {}",
                g.width_pct
            );
        }
    }

    #[test]
    fn degenerate_axis_draws_no_bar() {
        assert_eq!(
            bar_geometry(dec!(0), dec!(0), dec!(0)),
            BarGeometry {
                zero_pct: 0.0_f64,
                left_pct: 0.0_f64,
                width_pct: 0.0_f64
            }
        );
    }
}
