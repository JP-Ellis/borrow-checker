//! Day-fraction overlap calculation for mixed-period budget display.

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

