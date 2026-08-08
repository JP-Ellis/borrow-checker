//! Shared CLI period selection: the `--period` flag and its resolution into a
//! [`bc_models::Period`].

use jiff::civil::Date;

use crate::error::CliError;
use crate::error::CliResult;

// MARK: Flag

/// CLI period selector, shared by every command that takes `--period`.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum PeriodArg {
    /// Every 7 days (Monday–Sunday).
    Weekly,
    /// Every 14 days, phased from the configured `fortnightly_anchor`.
    Fortnightly,
    /// Calendar month.
    Monthly,
    /// Calendar quarter (Jan/Apr/Jul/Oct).
    Quarterly,
    /// Financial year, starting on the configured month and day.
    #[value(name = "financial-year")]
    FinancialYear,
    /// Financial quarter, aligned to the configured financial year start.
    #[value(name = "financial-quarter")]
    FinancialQuarter,
    /// Full calendar year (1 January – 31 December).
    #[value(name = "calendar-year")]
    CalendarYear,
    /// Arbitrary duration; requires at least one `--duration-*` component.
    Custom,
}

// MARK: Resolution

/// Everything [`resolve`] needs beyond the selector itself.
///
/// The financial-year start is a plain `u8` pair rather than an `Option`: the
/// caller supplies the configured value, so no command has to demand a flag
/// for a setting that already has a validated default.
#[non_exhaustive]
#[derive(Debug, Clone, Copy)]
pub struct PeriodInputs {
    /// Phase for [`PeriodArg::Fortnightly`]; `None` if unconfigured.
    pub fortnightly_anchor: Option<Date>,
    /// Days component of a [`PeriodArg::Custom`] duration.
    pub duration_days: Option<u32>,
    /// Weeks component of a [`PeriodArg::Custom`] duration.
    pub duration_weeks: Option<u32>,
    /// Months component of a [`PeriodArg::Custom`] duration.
    pub duration_months: Option<u32>,
    /// 1-based month the financial year starts in.
    pub fy_start_month: u8,
    /// 1-based day of `fy_start_month` the financial year starts on.
    pub fy_start_day: u8,
}

/// Converts a CLI period selector into a [`bc_models::Period`].
///
/// # Arguments
///
/// * `arg` - The selector supplied on the command line.
/// * `inputs` - Configured anchors and any `--duration-*` components.
///
/// # Returns
///
/// The equivalent domain period.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if [`PeriodArg::Fortnightly`] is selected with no
/// configured anchor, if [`PeriodArg::Custom`] is selected with no positive
/// duration, or if the configured financial-year start is out of range.
#[inline]
pub fn resolve(arg: PeriodArg, inputs: &PeriodInputs) -> CliResult<bc_models::Period> {
    match arg {
        PeriodArg::Weekly => Ok(bc_models::Period::Weekly),
        PeriodArg::Monthly => Ok(bc_models::Period::Monthly),
        PeriodArg::Quarterly => Ok(bc_models::Period::Quarterly),
        PeriodArg::CalendarYear => Ok(bc_models::Period::CalendarYear),
        PeriodArg::Fortnightly => {
            let anchor = inputs.fortnightly_anchor.ok_or_else(|| {
                CliError::Arg(
                    "fortnightly period requires `fortnightly_anchor` to be set in config"
                        .to_owned(),
                )
            })?;
            Ok(bc_models::Period::Fortnightly { anchor })
        }
        PeriodArg::FinancialYear => {
            bc_models::Period::financial_year(inputs.fy_start_month, inputs.fy_start_day)
                .map_err(|e| CliError::Arg(format!("invalid financial year: {e}")))
        }
        PeriodArg::FinancialQuarter => {
            bc_models::Period::financial_quarter(inputs.fy_start_month, inputs.fy_start_day)
                .map_err(|e| CliError::Arg(format!("invalid financial quarter: {e}")))
        }
        PeriodArg::Custom => bc_models::Period::custom(
            inputs.duration_days,
            inputs.duration_weeks,
            inputs.duration_months,
        )
        .map_err(|e| CliError::Arg(format!("invalid custom period: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use bc_models::Period;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    use super::PeriodArg;
    use super::PeriodInputs;
    use super::resolve;

    /// Inputs with the Australian defaults and no optional components set.
    fn au_inputs() -> PeriodInputs {
        PeriodInputs {
            fortnightly_anchor: None,
            duration_days: None,
            duration_weeks: None,
            duration_months: None,
            fy_start_month: 7,
            fy_start_day: 1,
        }
    }

    #[test]
    fn financial_year_uses_supplied_start_without_a_flag() {
        let period = resolve(PeriodArg::FinancialYear, &au_inputs()).expect("resolve");
        assert_eq!(
            period.range_containing(date(2025, 9, 15)),
            (date(2025, 7, 1), date(2026, 7, 1))
        );
    }

    #[test]
    fn financial_quarter_uses_supplied_start_without_a_flag() {
        let period = resolve(PeriodArg::FinancialQuarter, &au_inputs()).expect("resolve");
        assert_eq!(
            period.range_containing(date(2025, 9, 15)),
            (date(2025, 7, 1), date(2025, 10, 1))
        );
    }

    #[test]
    fn fortnightly_without_an_anchor_is_an_error() {
        let err = resolve(PeriodArg::Fortnightly, &au_inputs()).expect_err("no anchor");
        assert!(err.to_string().contains("fortnightly_anchor"));
    }

    #[test]
    fn fortnightly_with_an_anchor_resolves() {
        let inputs = PeriodInputs {
            fortnightly_anchor: Some(date(2026, 3, 3)),
            ..au_inputs()
        };
        assert!(matches!(
            resolve(PeriodArg::Fortnightly, &inputs).expect("resolve"),
            Period::Fortnightly { .. }
        ));
    }

    #[test]
    fn custom_without_any_duration_is_an_error() {
        let _unused = resolve(PeriodArg::Custom, &au_inputs()).expect_err("no duration");
    }

    #[test]
    fn custom_with_days_resolves() {
        let inputs = PeriodInputs {
            duration_days: Some(30),
            ..au_inputs()
        };
        assert!(matches!(
            resolve(PeriodArg::Custom, &inputs).expect("resolve"),
            Period::Custom { days: Some(30), .. }
        ));
    }

    #[test]
    fn fixed_anchor_variants_resolve() {
        let inputs = au_inputs();
        assert!(matches!(
            resolve(PeriodArg::Weekly, &inputs).expect("weekly"),
            Period::Weekly
        ));
        assert!(matches!(
            resolve(PeriodArg::Monthly, &inputs).expect("monthly"),
            Period::Monthly
        ));
        assert!(matches!(
            resolve(PeriodArg::Quarterly, &inputs).expect("quarterly"),
            Period::Quarterly
        ));
        assert!(matches!(
            resolve(PeriodArg::CalendarYear, &inputs).expect("calendar"),
            Period::CalendarYear
        ));
    }
}
