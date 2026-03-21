//! Budget period types.

/// A recurring budget period.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Period {
    /// Every 7 days, anchored to a day of the week.
    Weekly,
    /// Every 14 days, anchored to a specific calendar date.
    Fortnightly,
    /// Calendar month.
    Monthly,
    /// Calendar quarter (Jan/Apr/Jul/Oct by default).
    Quarterly,
    /// Financial year; start determined by [`crate::settings::GlobalSettings`].
    FinancialYear,
    /// Calendar year (1 January).
    CalendarYear,
    /// Arbitrary duration; at least one of the fields must be `Some`.
    Custom {
        /// Number of days, if specified.
        days: Option<u32>,
        /// Number of weeks, if specified.
        weeks: Option<u32>,
        /// Number of months, if specified.
        months: Option<u32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_variants_exist() {
        let _w = Period::Weekly;
        let _f = Period::Fortnightly;
        let _m = Period::Monthly;
        let _q = Period::Quarterly;
        let _fy = Period::FinancialYear;
        let _cy = Period::CalendarYear;
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
