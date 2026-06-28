//! Pure helpers for rendering and seeding per-posting accrual spreads.

use jiff::civil::Date;

/// How a spread chip should be rendered, per the display rule: show only the
/// end date when the spread starts on the transaction date, otherwise show
/// both endpoints.
#[expect(
    clippy::module_name_repetitions,
    reason = "`SpreadDisplay` reads naturally; the unprefixed `Display` would collide with `std::fmt::Display`"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpreadDisplay {
    /// Start coincides with the transaction date; show `⤳ <until>`.
    UntilOnly(Date),
    /// Start differs; show `<from> ⤳ <until>`.
    FromUntil(Date, Date),
}

/// Chooses the display form for a spread given the owning transaction's date.
#[expect(
    clippy::module_name_repetitions,
    reason = "`spread_display` reads naturally at call sites"
)]
#[must_use]
pub fn spread_display(from: Date, until: Date, tx_date: Date) -> SpreadDisplay {
    if from == tx_date {
        SpreadDisplay::UntilOnly(until)
    } else {
        SpreadDisplay::FromUntil(from, until)
    }
}

/// Seeds a new spread: start at the transaction date, end three months later.
#[expect(
    clippy::module_name_repetitions,
    reason = "`default_spread` reads naturally at call sites"
)]
#[must_use]
pub fn default_spread(tx_date: Date) -> (Date, Date) {
    let until = tx_date
        .checked_add(jiff::Span::new().months(3_i32))
        .unwrap_or(tx_date);
    (tx_date, until)
}

/// Formats a date as `"30 Sep 2026"`.
#[must_use]
pub fn fmt_spread_date(d: Date) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    #[expect(
        clippy::cast_sign_loss,
        reason = "d.month() always returns 1-12, safe to cast to usize"
    )]
    #[expect(
        clippy::as_conversions,
        reason = "d.month() always returns 1-12, safe to cast to usize"
    )]
    let month_idx = (d.month() as usize).saturating_sub(1);
    let month_str = MONTHS.get(month_idx).unwrap_or(&"?");
    format!("{} {} {}", d.day(), month_str, d.year())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn d(y: i16, m: i8, day: i8) -> Date {
        #[expect(clippy::unwrap_used, reason = "test helper with valid hardcoded dates")]
        {
            Date::new(y, m, day).unwrap()
        }
    }

    #[test]
    fn until_only_when_from_equals_tx_date() {
        let got = spread_display(d(2026, 6, 12), d(2026, 9, 30), d(2026, 6, 12));
        assert_eq!(got, SpreadDisplay::UntilOnly(d(2026, 9, 30)));
    }

    #[test]
    fn from_until_when_start_differs() {
        let got = spread_display(d(2026, 7, 1), d(2026, 9, 30), d(2026, 6, 12));
        assert_eq!(got, SpreadDisplay::FromUntil(d(2026, 7, 1), d(2026, 9, 30)));
    }

    #[test]
    fn default_is_three_months_from_tx_date() {
        assert_eq!(
            default_spread(d(2026, 6, 12)),
            (d(2026, 6, 12), d(2026, 9, 12))
        );
    }

    #[test]
    fn formats_date_human_readable() {
        assert_eq!(fmt_spread_date(d(2026, 9, 30)), "30 Sep 2026");
    }
}
