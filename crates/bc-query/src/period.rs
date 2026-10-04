//! Calendar periods: `2026`, `2026-03`, `2026-03-15`.

use jiff::civil::Date;

/// The half-open interval `[start, end)` a period covers. `end` is `None`
/// past the last representable date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Period {
    /// The first day.
    pub start: Date,
    /// The day after the last.
    pub end: Option<Date>,
}

/// Parses `YYYY`, `YYYY-MM` or `YYYY-MM-DD`; anything else is `None`.
pub(crate) fn parse(text: &str) -> Option<Period> {
    let parts: Vec<&str> = text.split('-').collect();
    match parts.as_slice() {
        [year] => {
            let y = digits(year, 4)?;
            let start = Date::new(y, 1, 1).ok()?;
            let end = y.checked_add(1).and_then(|next| Date::new(next, 1, 1).ok());
            Some(Period { start, end })
        }
        [year, month] => {
            let start = Date::new(digits(year, 4)?, digits(month, 2)?.try_into().ok()?, 1).ok()?;
            let end = start.checked_add(jiff::Span::new().months(1)).ok();
            Some(Period { start, end })
        }
        [year, month, day] => {
            let start = Date::new(
                digits(year, 4)?,
                digits(month, 2)?.try_into().ok()?,
                digits(day, 2)?.try_into().ok()?,
            )
            .ok()?;
            Some(Period {
                start,
                end: start.tomorrow().ok(),
            })
        }
        _ => None,
    }
}

/// Parses exactly `width` ASCII digits.
fn digits(text: &str, width: usize) -> Option<i16> {
    (text.len() == width && text.bytes().all(|b| b.is_ascii_digit()))
        .then(|| text.parse().ok())
        .flatten()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case("2026", date(2026, 1, 1), Some(date(2027, 1, 1)))]
    #[case("2026-03", date(2026, 3, 1), Some(date(2026, 4, 1)))]
    #[case("2026-12", date(2026, 12, 1), Some(date(2027, 1, 1)))]
    #[case("2026-03-15", date(2026, 3, 15), Some(date(2026, 3, 16)))]
    #[case("2024-02-29", date(2024, 2, 29), Some(date(2024, 3, 1)))]
    #[case("9999", date(9999, 1, 1), None)]
    #[case("9999-12", date(9999, 12, 1), None)]
    #[case("9999-12-31", date(9999, 12, 31), None)]
    fn parses(#[case] text: &str, #[case] start: Date, #[case] end: Option<Date>) {
        assert_eq!(parse(text), Some(Period { start, end }));
    }

    #[rstest]
    #[case("26")]
    #[case("2026-3")]
    #[case("2026-13")]
    #[case("2025-02-29")]
    #[case("2026-03-15-01")]
    #[case("march")]
    #[case("")]
    fn rejects(#[case] text: &str) {
        assert_eq!(parse(text), None);
    }
}
