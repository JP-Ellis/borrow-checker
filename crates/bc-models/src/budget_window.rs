//! A date-range window for budget queries, with calendar-period constructors.

use jiff::civil::Date;
use serde::Deserialize;
use serde::Serialize;

/// An explicit date range used to scope a budget query.
///
/// All frontends (TUI, CLI, future GUI) build a `BudgetWindow` from these
/// constructors and pass it to [`bc_core::BudgetEngine::status_for_window`].
/// The calendar arithmetic lives here so it is never duplicated across
/// presentation layers.
///
/// # Date convention
///
/// `start` is **inclusive**, `end` is **exclusive** (`[start, end)`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetWindow {
    /// Start of the window (inclusive).
    pub start: Date,
    /// End of the window (exclusive).
    pub end: Date,
    /// Human-readable label (e.g. `"May 2026"`, `"Q2 2026"`).
    pub label: String,
}

impl BudgetWindow {
    /// The calendar month containing `today`.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to determine the current month.
    ///
    /// # Returns
    ///
    /// A window from the first day of the current month (inclusive) to the
    /// first day of the next month (exclusive).
    ///
    /// # Panics
    ///
    /// Never panics — day 1 is always valid for any year/month from a valid
    /// `Date`.
    #[inline]
    #[must_use]
    pub fn this_month(today: Date) -> Self {
        #[expect(
            clippy::expect_used,
            reason = "year and month from a valid Date; day=1 is always valid"
        )]
        let start = Date::new(today.year(), today.month(), 1).expect("day 1 is always valid");
        let end = start.saturating_add(jiff::Span::new().months(1_i32));
        let label = format!("{} {}", month_name(today.month()), today.year());
        Self { start, end, label }
    }

    /// The calendar month immediately before `today`'s month.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to determine the previous month.
    ///
    /// # Returns
    ///
    /// A window from the first day of last month (inclusive) to the first
    /// day of this month (exclusive).
    ///
    /// # Panics
    ///
    /// Never panics — day 1 is always valid for any year/month from a valid
    /// `Date`.
    #[inline]
    #[must_use]
    pub fn last_month(today: Date) -> Self {
        #[expect(
            clippy::expect_used,
            reason = "year and month from a valid Date; day=1 is always valid"
        )]
        let end = Date::new(today.year(), today.month(), 1).expect("day 1 is always valid");
        let start = end.saturating_sub(jiff::Span::new().months(1_i32));
        let label = format!("{} {}", month_name(start.month()), start.year());
        Self { start, end, label }
    }

    /// The calendar quarter (Jan/Apr/Jul/Oct) containing `today`.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to determine the current quarter.
    ///
    /// # Returns
    ///
    /// A window from the first day of the current quarter (inclusive) to the
    /// first day of the next quarter (exclusive).
    ///
    /// # Panics
    ///
    /// Never panics — quarter-start months (1, 4, 7, 10) with day 1 are
    /// always valid dates.
    #[inline]
    #[must_use]
    #[expect(
        clippy::arithmetic_side_effects,
        clippy::integer_division,
        clippy::integer_division_remainder_used,
        reason = "quarter-start arithmetic: (month-1)/3*3+1 maps months to 1,4,7,10; bounded"
    )]
    pub fn this_quarter(today: Date) -> Self {
        let q_month = ((i16::from(today.month()) - 1) / 3) * 3 + 1;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::as_conversions,
            reason = "q_month is always 1, 4, 7, or 10 — fits in i8"
        )]
        let q_month_i8 = q_month as i8;
        #[expect(
            clippy::expect_used,
            reason = "quarter-start months 1,4,7,10 with day=1 are always valid"
        )]
        let start = Date::new(today.year(), q_month_i8, 1).expect("quarter start is valid");
        let end = start.saturating_add(jiff::Span::new().months(3_i32));
        let q_num = (i16::from(today.month()) - 1) / 3 + 1;
        let label = format!("Q{q_num} {}", today.year());
        Self { start, end, label }
    }

    /// The calendar quarter immediately before `today`'s quarter.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to determine the previous quarter.
    ///
    /// # Returns
    ///
    /// A window for the previous calendar quarter.
    #[inline]
    #[must_use]
    pub fn last_quarter(today: Date) -> Self {
        let this = Self::this_quarter(today);
        let end = this.start;
        let start = end.saturating_sub(jiff::Span::new().months(3_i32));
        #[expect(
            clippy::arithmetic_side_effects,
            clippy::integer_division,
            clippy::integer_division_remainder_used,
            reason = "same quarter numbering arithmetic as this_quarter"
        )]
        let q_num = (i16::from(start.month()) - 1) / 3 + 1;
        let label = format!("Q{q_num} {}", start.year());
        Self { start, end, label }
    }

    /// The calendar year containing `today`.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to determine the current year.
    ///
    /// # Returns
    ///
    /// A window from Jan 1 of the current year to Jan 1 of the next year.
    ///
    /// # Panics
    ///
    /// Never panics — Jan 1 of any year is always a valid date.
    #[inline]
    #[must_use]
    pub fn this_year(today: Date) -> Self {
        #[expect(clippy::expect_used, reason = "Jan 1 of any year is always valid")]
        let start = Date::new(today.year(), 1, 1).expect("Jan 1 is always valid");
        let end = start.saturating_add(jiff::Span::new().years(1_i32));
        let label = format!("{}", today.year());
        Self { start, end, label }
    }

    /// The calendar year immediately before `today`'s year.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to determine the previous year.
    ///
    /// # Returns
    ///
    /// A window from Jan 1 of last year to Jan 1 of this year.
    ///
    /// # Panics
    ///
    /// Never panics — Jan 1 of any year is always a valid date.
    #[inline]
    #[must_use]
    pub fn last_year(today: Date) -> Self {
        #[expect(clippy::expect_used, reason = "Jan 1 of any year is always valid")]
        let end = Date::new(today.year(), 1, 1).expect("Jan 1 is always valid");
        let start = end.saturating_sub(jiff::Span::new().years(1_i32));
        let label = format!("{}", start.year());
        Self { start, end, label }
    }

    /// An explicit date range with a caller-supplied label.
    ///
    /// # Arguments
    ///
    /// * `start` - Inclusive start date.
    /// * `end`   - Exclusive end date.
    /// * `label` - Display label for this window.
    ///
    /// # Returns
    ///
    /// A `BudgetWindow` covering the given range.
    #[inline]
    #[must_use]
    pub fn custom(start: Date, end: Date, label: impl Into<String>) -> Self {
        Self {
            start,
            end,
            label: label.into(),
        }
    }

    /// Returns the six standard calendar presets for the given `today`.
    ///
    /// Order: This Month, Last Month, This Quarter, Last Quarter,
    /// This Year, Last Year.
    ///
    /// # Arguments
    ///
    /// * `today` - The reference date used to compute each preset.
    ///
    /// # Returns
    ///
    /// A `Vec<BudgetWindow>` with six entries.
    #[inline]
    #[must_use]
    pub fn standard_presets(today: Date) -> Vec<Self> {
        vec![
            Self::this_month(today),
            Self::last_month(today),
            Self::this_quarter(today),
            Self::last_quarter(today),
            Self::this_year(today),
            Self::last_year(today),
        ]
    }

    /// Number of days in this window.
    ///
    /// # Returns
    ///
    /// `(end - start)` expressed as a count of days.
    #[inline]
    #[must_use]
    pub fn days(&self) -> i64 {
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "Date - Date returns a Span; get_days() is safe for any realistic date range"
        )]
        i64::from((self.end - self.start).get_days())
    }
}

/// Returns the English month name for a 1-based month number.
fn month_name(month: i8) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "Unknown",
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn this_month_boundaries() {
        let today = date(2026, 5, 15);
        let w = BudgetWindow::this_month(today);
        assert_eq!(w.start, date(2026, 5, 1));
        assert_eq!(w.end, date(2026, 6, 1));
    }

    #[test]
    fn last_month_boundaries() {
        let today = date(2026, 5, 15);
        let w = BudgetWindow::last_month(today);
        assert_eq!(w.start, date(2026, 4, 1));
        assert_eq!(w.end, date(2026, 5, 1));
    }

    #[test]
    fn this_quarter_boundaries() {
        let today = date(2026, 5, 15);
        let w = BudgetWindow::this_quarter(today);
        assert_eq!(w.start, date(2026, 4, 1));
        assert_eq!(w.end, date(2026, 7, 1));
    }

    #[test]
    fn last_quarter_boundaries() {
        let today = date(2026, 5, 15);
        let w = BudgetWindow::last_quarter(today);
        assert_eq!(w.start, date(2026, 1, 1));
        assert_eq!(w.end, date(2026, 4, 1));
    }

    #[test]
    fn this_year_boundaries() {
        let today = date(2026, 5, 15);
        let w = BudgetWindow::this_year(today);
        assert_eq!(w.start, date(2026, 1, 1));
        assert_eq!(w.end, date(2027, 1, 1));
    }

    #[test]
    fn last_year_boundaries() {
        let today = date(2026, 5, 15);
        let w = BudgetWindow::last_year(today);
        assert_eq!(w.start, date(2025, 1, 1));
        assert_eq!(w.end, date(2026, 1, 1));
    }

    #[test]
    fn days_returns_span_in_days() {
        let w = BudgetWindow::this_month(date(2026, 5, 15));
        assert_eq!(w.days(), 31);
    }

    #[test]
    fn standard_presets_has_six_entries() {
        let presets = BudgetWindow::standard_presets(date(2026, 5, 15));
        assert_eq!(presets.len(), 6);
    }
}
