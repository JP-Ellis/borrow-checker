//! Day-fraction overlap calculation for mixed-period budget display.

use bc_models::Period;
use jiff::civil::Date;

/// One native period overlapping a display window, with the overlap in days.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PeriodOverlap {
    /// Start of the native period (inclusive).
    pub native_start: Date,
    /// End of the native period (exclusive).
    pub native_end: Date,
    /// Start of the overlap with the display window (inclusive).
    pub overlap_start: Date,
    /// End of the overlap with the display window (exclusive).
    pub overlap_end: Date,
}

impl PeriodOverlap {
    /// Returns the number of days in the full native period.
    #[must_use]
    #[inline]
    pub fn native_days(&self) -> i32 {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date is bounded by calendar range"
        )]
        (self.native_end - self.native_start).get_days()
    }

    /// Returns the number of days in the overlap with the display window.
    #[must_use]
    #[inline]
    pub fn overlap_days(&self) -> i32 {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date is bounded by calendar range"
        )]
        (self.overlap_end - self.overlap_start).get_days()
    }

    /// Returns `overlap_days / native_days` as an `f64` fraction.
    ///
    /// Returns `0.0` if `native_days` is zero.
    #[must_use]
    #[inline]
    pub fn fraction(&self) -> f64 {
        let nd = self.native_days();
        if nd == 0_i32 {
            return 0.0_f64;
        }
        #[expect(clippy::float_arithmetic, reason = "intentional pro-rata fraction")]
        {
            f64::from(self.overlap_days()) / f64::from(nd)
        }
    }
}

/// Returns all native periods of `period` that overlap `[display_start, display_end)`,
/// together with the overlap extent for each.
///
/// The list is ordered chronologically by `native_start`.
///
/// # Errors
///
/// Returns an error string if `display_end <= display_start`.
pub fn overlapping_periods(
    period: &Period,
    display_start: Date,
    display_end: Date,
) -> Result<Vec<PeriodOverlap>, String> {
    if display_end <= display_start {
        return Err(format!(
            "display_end ({display_end}) must be after display_start ({display_start})"
        ));
    }

    let mut result = Vec::new();
    let (mut native_start, mut native_end) = period.range_containing(display_start);

    loop {
        // Overlap is [max(native_start, display_start), min(native_end, display_end))
        let overlap_start = native_start.max(display_start);
        let overlap_end = native_end.min(display_end);

        if overlap_start < overlap_end {
            result.push(PeriodOverlap {
                native_start,
                native_end,
                overlap_start,
                overlap_end,
            });
        }

        if native_end >= display_end {
            break;
        }

        // Advance to the next native period.
        let (next_start, next_end) = period.range_containing(native_end);
        if next_start <= native_start {
            // Guard against infinite loops from pathological Period impls.
            break;
        }
        native_start = next_start;
        native_end = next_end;
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use bc_models::Period;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::overlapping_periods;

    fn date(y: i16, m: i8, d: i8) -> Date {
        Date::new(y, m, d).expect("valid date")
    }

    #[test]
    fn monthly_budget_in_monthly_display_single_period() {
        // Monthly budget, display = June 2026 → exactly one native period.
        let overlaps =
            overlapping_periods(&Period::Monthly, date(2026, 6, 1), date(2026, 7, 1)).expect("ok");
        assert_eq!(overlaps.len(), 1);
        let o = overlaps.first().expect("non-empty");
        assert_eq!(o.native_start, date(2026, 6, 1));
        assert_eq!(o.native_end, date(2026, 7, 1));
        assert_eq!(o.overlap_days(), 30_i32);
        assert_eq!(o.native_days(), 30_i32);
        assert!((o.fraction() - 1.0_f64).abs() < f64::EPSILON);
    }

    #[test]
    fn weekly_budget_in_june_spans_five_partial_and_full_weeks() {
        // Weekly budget (Mon–Sun), June 2026 (Jun 1 = Mon, Jul 1 = Wed).
        // Weeks: w22 Jun 1–7, w23 Jun 8–14, w24 Jun 15–21, w25 Jun 22–28,
        //        w26 Jun 29–Jul 5 (only Jun 29–Jul 1 in June = 2 days).
        let overlaps =
            overlapping_periods(&Period::Weekly, date(2026, 6, 1), date(2026, 7, 1)).expect("ok");
        // 5 native periods should overlap (w22..w26).
        assert_eq!(overlaps.len(), 5);

        // First week fully in June.
        let first = overlaps.first().expect("non-empty");
        assert_eq!(first.overlap_days(), 7_i32);
        assert!((first.fraction() - 1.0_f64).abs() < f64::EPSILON);

        // Last week straddles Jul 1 — only 2 days in June (Jun 29, Jun 30).
        let last = overlaps.last().expect("non-empty");
        assert_eq!(last.overlap_days(), 2_i32);
        assert_eq!(last.native_days(), 7_i32);
        assert!((last.fraction() - 2.0_f64 / 7.0_f64).abs() < 1e-10_f64);
    }

    #[test]
    fn annual_budget_in_october_pro_rates_to_month() {
        // CalendarYear budget (Jan 1 – Dec 31, 365 days), display = October 2026 (31 days).
        let overlaps =
            overlapping_periods(&Period::CalendarYear, date(2026, 10, 1), date(2026, 11, 1))
                .expect("ok");
        assert_eq!(overlaps.len(), 1);
        let o = overlaps.first().expect("non-empty");
        assert_eq!(o.native_start, date(2026, 1, 1));
        assert_eq!(o.native_end, date(2027, 1, 1));
        assert_eq!(o.overlap_days(), 31_i32);
        assert_eq!(o.native_days(), 365_i32);
        assert!((o.fraction() - 31.0_f64 / 365.0_f64).abs() < 1e-10_f64);
    }

    #[test]
    fn inverted_window_returns_error() {
        let result = overlapping_periods(&Period::Monthly, date(2026, 7, 1), date(2026, 6, 1));
        assert!(result.is_err(), "inverted window must return an error");
    }
}
