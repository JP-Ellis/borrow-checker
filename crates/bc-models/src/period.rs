//! Budget period types with embedded anchor data and jiff computation methods.

use jiff::civil::Date;
use serde::{Deserialize, Serialize};

/// Error returned when constructing a validated [`Period`] variant.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum BuildError {
    /// A `Custom` period requires at least one of `days`, `weeks`, or `months`.
    #[error("custom period must specify at least one of days, weeks, or months")]
    CustomNoDuration,
    /// `start_month` must be 1–12.
    #[error("invalid month {0}: must be 1–12")]
    InvalidMonth(u8),
    /// `start_day` must be 1–28.
    #[error("invalid day {0}: must be 1–28")]
    InvalidDay(u8),
}

/// A recurring budget period.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Period {
    /// Every 7 days.
    Weekly,
    /// Every 14 days, anchored to a specific date.
    ///
    /// `anchor` is any date within the desired fortnightly cycle;
    /// arithmetic uses distance-from-anchor modulo 14.
    #[serde(alias = "biweekly")]
    Fortnightly {
        /// Any date within the cycle; defines the phase.
        anchor: Date,
    },
    /// Calendar month.
    Monthly,
    /// Calendar quarter (Jan/Apr/Jul/Oct).
    Quarterly,
    /// Financial year with configurable start.
    FinancialYear {
        /// 1-based month (1–12).
        start_month: u8,
        /// 1-based day (1–28).
        start_day: u8,
    },
    /// Calendar year (1 January).
    CalendarYear,
    /// Arbitrary duration; at least one field is `Some`.
    Custom {
        /// Days component.
        days: Option<u32>,
        /// Weeks component.
        weeks: Option<u32>,
        /// Months component.
        months: Option<u32>,
    },
}

impl Period {
    /// Constructs a validated `Custom` period.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::CustomNoDuration`] if all three are `None`.
    #[inline]
    pub fn custom(
        days: Option<u32>,
        weeks: Option<u32>,
        months: Option<u32>,
    ) -> Result<Self, BuildError> {
        if days.is_none() && weeks.is_none() && months.is_none() {
            return Err(BuildError::CustomNoDuration);
        }
        Ok(Self::Custom {
            days,
            weeks,
            months,
        })
    }

    /// Constructs a validated `FinancialYear` period.
    ///
    /// # Errors
    ///
    /// Returns [`BuildError::InvalidMonth`] if `start_month` is outside 1–12.
    /// Returns [`BuildError::InvalidDay`] if `start_day` is outside 1–28.
    #[inline]
    pub fn financial_year(start_month: u8, start_day: u8) -> Result<Self, BuildError> {
        if !(1..=12).contains(&start_month) {
            return Err(BuildError::InvalidMonth(start_month));
        }
        if !(1..=28).contains(&start_day) {
            return Err(BuildError::InvalidDay(start_day));
        }
        Ok(Self::FinancialYear {
            start_month,
            start_day,
        })
    }

    /// Returns the `[start, end)` date range of the period containing `date`.
    #[inline]
    #[must_use]
    pub fn range_containing(&self, date: Date) -> (Date, Date) {
        let start = self.period_start_on_or_before(date);
        let end = self.advance(start);
        (start, end)
    }

    /// Returns the start date of the next period after `date`.
    #[inline]
    #[must_use]
    pub fn next_after(&self, date: Date) -> Date {
        self.advance(self.period_start_on_or_before(date))
    }

    /// Yields the start date of each period from `start` onwards.
    ///
    /// Returns a `'static` iterator because all variant data (`Date`, `u8`, `u32`)
    /// is `Copy` and captured by value; no borrow of `self` escapes.
    #[inline]
    pub fn iter_from(&self, start: Date) -> impl Iterator<Item = Date> + 'static {
        let period = self.clone();
        let mut current = period.period_start_on_or_before(start);
        if current < start {
            current = period.advance(current);
        }
        core::iter::from_fn(move || {
            let result = current;
            current = period.advance(current);
            Some(result)
        })
    }

    /// Computes the period start date on or before `date`.
    #[expect(
        clippy::too_many_lines,
        reason = "match arms for each variant are clear and self-contained; extracting would obscure the logic"
    )]
    fn period_start_on_or_before(&self, date: Date) -> Date {
        match self {
            Self::Weekly => {
                // Start of ISO week: to_monday_one_offset() returns i8 with Monday=1, Sunday=7.
                // Subtract (offset - 1) days to reach Monday.
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "to_monday_one_offset() returns 1–7; subtracting 1 is always in range [0, 6]"
                )]
                let dow = i64::from(date.weekday().to_monday_one_offset()) - 1;
                date.saturating_sub(jiff::Span::new().days(dow))
            }
            Self::Fortnightly { anchor } => {
                // get_days() returns i32; cast to i64 for rem_euclid arithmetic.
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "Date - Date returns a Span; get_days() is i32 and i64::from is safe"
                )]
                let diff = i64::from((date - *anchor).get_days());
                // rem_euclid guarantees a non-negative result in [0, 13]
                let phase = diff.rem_euclid(14);
                date.saturating_sub(jiff::Span::new().days(phase))
            }
            Self::Monthly => {
                #[expect(
                    clippy::expect_used,
                    reason = "year and month are taken from an existing valid Date; day=1 is always valid"
                )]
                let d = Date::new(date.year(), date.month(), 1)
                    .expect("year and month from existing date are always valid");
                d
            }
            Self::Quarterly => {
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "month is 1–12; the arithmetic maps it to the quarter start 1, 4, 7, or 10"
                )]
                #[expect(
                    clippy::integer_division,
                    reason = "integer division by 3 is intentional: maps months to quarters"
                )]
                #[expect(
                    clippy::integer_division_remainder_used,
                    reason = "integer division by 3 is intentional: maps months to quarters"
                )]
                let q_month = ((i16::from(date.month()) - 1) / 3) * 3 + 1;
                #[expect(
                    clippy::cast_possible_truncation,
                    reason = "q_month is always 1, 4, 7, or 10 — fits in i8"
                )]
                #[expect(
                    clippy::as_conversions,
                    reason = "q_month is bounded to 1, 4, 7, or 10 by construction; cast is safe"
                )]
                let q_month_i8 = q_month as i8;
                #[expect(
                    clippy::expect_used,
                    reason = "quarter start is always 1, 4, 7, or 10 with day=1; always a valid date"
                )]
                let d = Date::new(date.year(), q_month_i8, 1)
                    .expect("quarter start month is always valid");
                d
            }
            Self::FinancialYear {
                start_month,
                start_day,
            } => {
                #[expect(
                    clippy::cast_possible_wrap,
                    reason = "start_month is validated to 1–12 at construction; fits in i8"
                )]
                #[expect(
                    clippy::as_conversions,
                    reason = "start_month is validated to 1–12 at construction; cast is safe"
                )]
                let sm = *start_month as i8;
                #[expect(
                    clippy::cast_possible_wrap,
                    reason = "start_day is validated to 1–28 at construction; fits in i8"
                )]
                #[expect(
                    clippy::as_conversions,
                    reason = "start_day is validated to 1–28 at construction; cast is safe"
                )]
                let sd = *start_day as i8;
                #[expect(
                    clippy::expect_used,
                    reason = "month/day were validated at construction of FinancialYear"
                )]
                let this_year = Date::new(date.year(), sm, sd)
                    .expect("FinancialYear was validated at construction");
                if date >= this_year {
                    this_year
                } else {
                    #[expect(
                        clippy::arithmetic_side_effects,
                        reason = "subtracting 1 from a valid i16 year is safe for any realistic year"
                    )]
                    #[expect(
                        clippy::expect_used,
                        reason = "prior year with validated month/day is always a valid date"
                    )]
                    Date::new(date.year() - 1, sm, sd).expect("prior year FY start is valid")
                }
            }
            Self::CalendarYear => {
                #[expect(
                    clippy::expect_used,
                    reason = "January 1 of any year is always a valid date"
                )]
                let d = Date::new(date.year(), 1, 1).expect("Jan 1 is always valid");
                d
            }
            Self::Custom { .. } => {
                // Custom has no anchor; treat date as-is (period start = date itself)
                date
            }
        }
    }

    /// Advances a period start date by one period length.
    fn advance(&self, date: Date) -> Date {
        match self {
            Self::Weekly => date.saturating_add(jiff::Span::new().weeks(1)),
            Self::Fortnightly { .. } => date.saturating_add(jiff::Span::new().weeks(2)),
            Self::Monthly => date.saturating_add(jiff::Span::new().months(1)),
            Self::Quarterly => date.saturating_add(jiff::Span::new().months(3)),
            Self::FinancialYear { .. } | Self::CalendarYear => {
                date.saturating_add(jiff::Span::new().years(1))
            }
            Self::Custom {
                days,
                weeks,
                months,
            } => date.saturating_add(
                jiff::Span::new()
                    .days(i64::from(days.unwrap_or(0)))
                    .weeks(i64::from(weeks.unwrap_or(0)))
                    .months(i64::from(months.unwrap_or(0))),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn fortnightly_requires_anchor() {
        use jiff::civil::date;
        let p = Period::Fortnightly {
            anchor: date(2026, 1, 1),
        };
        assert!(matches!(p, Period::Fortnightly { .. }));
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "unwrap/unwrap_err in tests is acceptable to assert the expected Ok/Err state"
    )]
    fn financial_year_constructor_validates_month() {
        _ = Period::financial_year(13, 1).unwrap_err();
        _ = Period::financial_year(0, 1).unwrap_err();
        _ = Period::financial_year(7, 1).unwrap();
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "unwrap/unwrap_err in tests is acceptable to assert the expected Ok/Err state"
    )]
    fn financial_year_constructor_validates_day() {
        _ = Period::financial_year(7, 0).unwrap_err();
        _ = Period::financial_year(7, 29).unwrap_err();
        _ = Period::financial_year(7, 28).unwrap();
    }

    #[test]
    #[expect(
        clippy::unwrap_used,
        reason = "unwrap/unwrap_err in tests is acceptable to assert the expected Ok/Err state"
    )]
    fn custom_constructor_rejects_all_none() {
        _ = Period::custom(None, None, None).unwrap_err();
        _ = Period::custom(Some(30), None, None).unwrap();
    }

    #[test]
    fn range_containing_weekly() {
        use jiff::civil::date;
        // 2026-03-23 is a Monday; weekly range should start on Monday
        let (start, end) = Period::Weekly.range_containing(date(2026, 3, 23));
        assert!(start <= date(2026, 3, 23));
        assert!(date(2026, 3, 23) < end);
    }

    #[test]
    fn iter_from_yields_sequential_months() {
        use jiff::civil::date;
        let periods: Vec<_> = Period::Monthly
            .iter_from(date(2026, 1, 1))
            .take(3)
            .collect();
        #[expect(
            clippy::indexing_slicing,
            reason = "we just collected exactly 3 elements with .take(3); indices 0, 1, 2 are valid"
        )]
        {
            assert_eq!(periods[0], date(2026, 1, 1));
            assert_eq!(periods[1], date(2026, 2, 1));
            assert_eq!(periods[2], date(2026, 3, 1));
        }
    }

    #[test]
    fn period_variants_exist() {
        _ = (
            Period::Weekly,
            Period::Monthly,
            Period::Quarterly,
            Period::CalendarYear,
        );
    }

    #[test]
    fn custom_period_days() {
        let p = Period::Custom {
            days: Some(30),
            weeks: None,
            months: None,
        };
        assert!(matches!(p, Period::Custom { days: Some(30), .. }));
    }
}
