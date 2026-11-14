// Draft state for the period editor: the kind dropdown plus the text of each
// parameter field. Leptos-free so it runs under a native `cargo nextest`; see
// `components_tests` in `main.rs`.

use bc_ipc::Period;
use jiff::civil::Date;

use crate::period_defaults::PeriodDefaults;

/// Message shown when the Fortnightly anchor is empty or not a date.
const ANCHOR_MESSAGE: &str = "Choose an anchor date.";
/// Message shown when a numeric field holds anything but a whole number.
const NUMBER_MESSAGE: &str = "Enter a whole number.";

/// The period kinds the editor's dropdown offers, one per [`Period`] variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PeriodKind {
    /// [`Period::Daily`].
    Daily,
    /// [`Period::Weekly`].
    Weekly,
    /// [`Period::Fortnightly`].
    Fortnightly,
    /// [`Period::Monthly`].
    Monthly,
    /// [`Period::Quarterly`].
    Quarterly,
    /// [`Period::CalendarYear`].
    CalendarYear,
    /// [`Period::FinancialYear`].
    FinancialYear,
    /// [`Period::FinancialQuarter`].
    FinancialQuarter,
    /// [`Period::Custom`].
    Custom,
}

impl PeriodKind {
    /// Every kind, in dropdown order.
    pub(crate) const ALL: [Self; 9] = [
        Self::Daily,
        Self::Weekly,
        Self::Fortnightly,
        Self::Monthly,
        Self::Quarterly,
        Self::CalendarYear,
        Self::FinancialYear,
        Self::FinancialQuarter,
        Self::Custom,
    ];

    /// The dropdown option value, matching the serde tag of the variant.
    #[must_use]
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Fortnightly => "fortnightly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::CalendarYear => "calendar_year",
            Self::FinancialYear => "financial_year",
            Self::FinancialQuarter => "financial_quarter",
            Self::Custom => "custom",
        }
    }

    /// The kind whose [`key`](Self::key) is `key`, if any.
    #[must_use]
    pub(crate) fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.key() == key)
    }

    /// The dropdown option text.
    #[must_use]
    pub(crate) const fn text(self) -> &'static str {
        match self {
            Self::Daily => "daily",
            Self::Weekly => "weekly",
            Self::Fortnightly => "fortnightly",
            Self::Monthly => "monthly",
            Self::Quarterly => "quarterly",
            Self::CalendarYear => "calendar year",
            Self::FinancialYear => "financial year",
            Self::FinancialQuarter => "financial quarter",
            Self::Custom => "custom",
        }
    }

    /// The kind of `period`; `None` for a variant this build does not know.
    #[must_use]
    pub(crate) const fn of(period: &Period) -> Option<Self> {
        Some(match period {
            Period::Daily => Self::Daily,
            Period::Weekly => Self::Weekly,
            Period::Fortnightly { .. } => Self::Fortnightly,
            Period::Monthly => Self::Monthly,
            Period::Quarterly => Self::Quarterly,
            Period::CalendarYear => Self::CalendarYear,
            Period::FinancialYear { .. } => Self::FinancialYear,
            Period::FinancialQuarter { .. } => Self::FinancialQuarter,
            Period::Custom { .. } => Self::Custom,
            _ => return None,
        })
    }
}

/// The editor's state: the selected kind and the text of every parameter
/// field, including fields the selected kind does not show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PeriodDraft {
    /// The selected kind.
    pub kind: PeriodKind,
    /// Fortnightly anchor, as `YYYY-MM-DD`.
    pub anchor: String,
    /// Financial year or quarter start month.
    pub start_month: String,
    /// Financial year or quarter start day.
    pub start_day: String,
    /// Custom days component; empty means none.
    pub days: String,
    /// Custom weeks component; empty means none.
    pub weeks: String,
    /// Custom months component; empty means none.
    pub months: String,
}

impl PeriodDraft {
    /// The draft showing `period`; `None` for a variant this build does not know.
    ///
    /// # Arguments
    ///
    /// * `period` - The loaded period.
    /// * `defaults` - Seeds the fields of the kinds `period` is not.
    #[must_use]
    pub(crate) fn from_period(period: &Period, defaults: PeriodDefaults) -> Option<Self> {
        let blank = Self {
            kind: PeriodKind::Monthly,
            anchor: String::new(),
            start_month: String::new(),
            start_day: String::new(),
            days: String::new(),
            weeks: String::new(),
            months: String::new(),
        };
        Some(blank.switch_to(PeriodKind::of(period)?, period, defaults))
    }

    /// The draft after the user picks `kind`.
    ///
    /// The fields of `kind` come from `original` when it is that kind, so a
    /// flip away and back loses nothing. Otherwise they come from `defaults`,
    /// and Custom starts empty. Fields of other kinds carry over.
    ///
    /// # Arguments
    ///
    /// * `kind` - The newly selected kind.
    /// * `original` - The period the editor opened with.
    /// * `defaults` - The configured FY start and fortnightly anchor.
    #[must_use]
    pub(crate) fn switch_to(
        &self,
        kind: PeriodKind,
        original: &Period,
        defaults: PeriodDefaults,
    ) -> Self {
        let mut next = self.clone();
        next.kind = kind;
        match (kind, original) {
            (PeriodKind::Fortnightly, Period::Fortnightly { anchor }) => {
                next.anchor = anchor.to_string();
            }
            (PeriodKind::Fortnightly, _) => {
                next.anchor = defaults
                    .fortnightly_anchor
                    .map_or_else(String::new, |anchor| anchor.to_string());
            }
            (
                PeriodKind::FinancialYear,
                Period::FinancialYear {
                    start_month,
                    start_day,
                },
            )
            | (
                PeriodKind::FinancialQuarter,
                Period::FinancialQuarter {
                    start_month,
                    start_day,
                },
            ) => {
                next.start_month = start_month.to_string();
                next.start_day = start_day.to_string();
            }
            (PeriodKind::FinancialYear | PeriodKind::FinancialQuarter, _) => {
                next.start_month = defaults.fy_start_month.to_string();
                next.start_day = defaults.fy_start_day.to_string();
            }
            (
                PeriodKind::Custom,
                Period::Custom {
                    days,
                    weeks,
                    months,
                },
            ) => {
                next.days = optional_text(*days);
                next.weeks = optional_text(*weeks);
                next.months = optional_text(*months);
            }
            (PeriodKind::Custom, _) => {
                next.days = String::new();
                next.weeks = String::new();
                next.months = String::new();
            }
            (
                PeriodKind::Daily
                | PeriodKind::Weekly
                | PeriodKind::Monthly
                | PeriodKind::Quarterly
                | PeriodKind::CalendarYear,
                _,
            ) => {}
        }
        next
    }

    /// The period the draft describes.
    ///
    /// # Errors
    ///
    /// Returns the message to show under the fields: a missing anchor, a
    /// field that is not a whole number, or the `BuildError` the server would
    /// give for the parsed values.
    pub(crate) fn to_period(&self) -> Result<Period, String> {
        match self.kind {
            PeriodKind::Daily => Ok(Period::Daily),
            PeriodKind::Weekly => Ok(Period::Weekly),
            PeriodKind::Monthly => Ok(Period::Monthly),
            PeriodKind::Quarterly => Ok(Period::Quarterly),
            PeriodKind::CalendarYear => Ok(Period::CalendarYear),
            PeriodKind::Fortnightly => self
                .anchor
                .trim()
                .parse::<Date>()
                .map(|anchor| Period::Fortnightly { anchor })
                .map_err(|_unparsable| ANCHOR_MESSAGE.to_owned()),
            PeriodKind::FinancialYear => {
                Period::financial_year(whole(&self.start_month)?, whole(&self.start_day)?)
                    .map_err(|e| e.to_string())
            }
            PeriodKind::FinancialQuarter => {
                Period::financial_quarter(whole(&self.start_month)?, whole(&self.start_day)?)
                    .map_err(|e| e.to_string())
            }
            PeriodKind::Custom => Period::custom(
                optional_whole(&self.days)?,
                optional_whole(&self.weeks)?,
                optional_whole(&self.months)?,
            )
            .map_err(|e| e.to_string()),
        }
    }
}

/// Field text for an optional Custom component.
fn optional_text(value: Option<u32>) -> String {
    value.map_or_else(String::new, |n| n.to_string())
}

/// Parses a required whole-number field.
fn whole<T>(text: &str) -> Result<T, String>
where
    T: core::str::FromStr,
{
    text.trim()
        .parse()
        .map_err(|_unparsable| NUMBER_MESSAGE.to_owned())
}

/// Parses an optional whole-number field; empty reads as `None`.
fn optional_whole(text: &str) -> Result<Option<u32>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        Ok(None)
    } else {
        whole(trimmed).map(Some)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Period;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::PeriodDraft;
    use super::PeriodKind;
    use crate::period_defaults::PeriodDefaults;

    const FALLBACK: PeriodDefaults = PeriodDefaults::FALLBACK;

    fn draft_of(period: &Period) -> PeriodDraft {
        PeriodDraft::from_period(period, FALLBACK).expect("every known variant has a draft")
    }

    #[rstest]
    #[case::daily(Period::Daily)]
    #[case::weekly(Period::Weekly)]
    #[case::fortnightly(Period::Fortnightly { anchor: date(2026, 3, 2) })]
    #[case::monthly(Period::Monthly)]
    #[case::quarterly(Period::Quarterly)]
    #[case::calendar_year(Period::CalendarYear)]
    #[case::fy(Period::FinancialYear { start_month: 1, start_day: 1 })]
    #[case::fq(Period::FinancialQuarter { start_month: 4, start_day: 6 })]
    #[case::custom(Period::Custom { days: Some(10), weeks: None, months: Some(1) })]
    fn draft_round_trips(#[case] period: Period) {
        assert_eq!(draft_of(&period).to_period(), Ok(period));
    }

    #[test]
    fn flipping_away_and_back_restores_the_loaded_fy_start() {
        let original = Period::FinancialYear {
            start_month: 1,
            start_day: 1,
        };
        let back = draft_of(&original)
            .switch_to(PeriodKind::Monthly, &original, FALLBACK)
            .switch_to(PeriodKind::FinancialYear, &original, FALLBACK);
        assert_eq!(back.start_month, "1");
        assert_eq!(back.start_day, "1");
        assert_eq!(back.to_period(), Ok(original));
    }

    #[test]
    fn switching_to_fy_seeds_the_configured_start() {
        let original = Period::Monthly;
        let fy = draft_of(&original).switch_to(PeriodKind::FinancialYear, &original, FALLBACK);
        assert_eq!(fy.start_month, "7");
        assert_eq!(fy.start_day, "1");
    }

    #[test]
    fn switching_to_fortnightly_seeds_the_configured_anchor() {
        let original = Period::Monthly;
        let defaults = PeriodDefaults {
            fortnightly_anchor: Some(date(2026, 1, 5)),
            ..FALLBACK
        };
        let draft = PeriodDraft::from_period(&original, defaults)
            .expect("monthly has a draft")
            .switch_to(PeriodKind::Fortnightly, &original, defaults);
        assert_eq!(
            draft.to_period(),
            Ok(Period::Fortnightly {
                anchor: date(2026, 1, 5)
            })
        );
    }

    #[test]
    fn fortnightly_without_an_anchor_asks_for_one() {
        let original = Period::Monthly;
        let draft = draft_of(&original).switch_to(PeriodKind::Fortnightly, &original, FALLBACK);
        assert_eq!(draft.anchor, "");
        assert_eq!(draft.to_period(), Err("Choose an anchor date.".to_owned()));
    }

    #[test]
    fn custom_starts_empty_and_needs_a_duration() {
        let original = Period::Monthly;
        let draft = draft_of(&original).switch_to(PeriodKind::Custom, &original, FALLBACK);
        assert_eq!(
            (
                draft.days.as_str(),
                draft.weeks.as_str(),
                draft.months.as_str()
            ),
            ("", "", "")
        );
        let message = Period::custom(None, None, None)
            .expect_err("an empty custom period is invalid")
            .to_string();
        assert_eq!(draft.to_period(), Err(message));
    }

    #[test]
    fn other_kinds_fields_carry_over() {
        let original = Period::Fortnightly {
            anchor: date(2026, 3, 2),
        };
        let draft = draft_of(&original).switch_to(PeriodKind::Weekly, &original, FALLBACK);
        assert_eq!(draft.kind, PeriodKind::Weekly);
        assert_eq!(draft.anchor, "2026-03-02");
    }

    #[rstest]
    #[case::day_out_of_range(PeriodKind::FinancialYear, "7", "29", "invalid day 29: must be 1–28")]
    #[case::month_out_of_range(
        PeriodKind::FinancialQuarter,
        "13",
        "1",
        "invalid month 13: must be 1–12"
    )]
    #[case::empty_month(PeriodKind::FinancialYear, "", "1", "Enter a whole number.")]
    #[case::fractional_day(PeriodKind::FinancialQuarter, "7", "1.5", "Enter a whole number.")]
    fn fy_fields_report_errors(
        #[case] kind: PeriodKind,
        #[case] month: &str,
        #[case] day: &str,
        #[case] message: &str,
    ) {
        let draft = PeriodDraft {
            kind,
            start_month: month.to_owned(),
            start_day: day.to_owned(),
            ..draft_of(&Period::Monthly)
        };
        assert_eq!(draft.to_period(), Err(message.to_owned()));
    }

    #[rstest]
    #[case::bad_days("x", "", "", Err("Enter a whole number.".to_owned()))]
    #[case::bad_weeks("", "-1", "", Err("Enter a whole number.".to_owned()))]
    #[case::bad_months("", "", "1 month", Err("Enter a whole number.".to_owned()))]
    #[case::days_only(" 10 ", "", "", Ok(Period::Custom { days: Some(10), weeks: None, months: None }))]
    #[case::weeks_and_months("", "2", "1", Ok(Period::Custom { days: None, weeks: Some(2), months: Some(1) }))]
    fn custom_fields_parse(
        #[case] days: &str,
        #[case] weeks: &str,
        #[case] months: &str,
        #[case] want: Result<Period, String>,
    ) {
        let draft = PeriodDraft {
            kind: PeriodKind::Custom,
            days: days.to_owned(),
            weeks: weeks.to_owned(),
            months: months.to_owned(),
            ..draft_of(&Period::Monthly)
        };
        assert_eq!(draft.to_period(), want);
    }

    #[test]
    fn an_unparsable_anchor_asks_for_one() {
        let draft = PeriodDraft {
            kind: PeriodKind::Fortnightly,
            anchor: "2026-02-30".to_owned(),
            ..draft_of(&Period::Monthly)
        };
        assert_eq!(draft.to_period(), Err("Choose an anchor date.".to_owned()));
    }

    #[test]
    fn kind_keys_round_trip() {
        for kind in PeriodKind::ALL {
            assert_eq!(PeriodKind::from_key(kind.key()), Some(kind));
        }
        assert_eq!(PeriodKind::from_key("biweekly"), None);
    }

    #[rstest]
    #[case(PeriodKind::Daily, "daily")]
    #[case(PeriodKind::Weekly, "weekly")]
    #[case(PeriodKind::Fortnightly, "fortnightly")]
    #[case(PeriodKind::Monthly, "monthly")]
    #[case(PeriodKind::Quarterly, "quarterly")]
    #[case(PeriodKind::CalendarYear, "calendar year")]
    #[case(PeriodKind::FinancialYear, "financial year")]
    #[case(PeriodKind::FinancialQuarter, "financial quarter")]
    #[case(PeriodKind::Custom, "custom")]
    fn kind_text_names_the_kind(#[case] kind: PeriodKind, #[case] text: &str) {
        assert_eq!(kind.text(), text);
    }

    #[test]
    fn kind_of_matches_the_variant() {
        for kind in PeriodKind::ALL {
            let draft = PeriodDraft {
                kind,
                anchor: "2026-03-02".to_owned(),
                start_month: "7".to_owned(),
                start_day: "1".to_owned(),
                days: "10".to_owned(),
                ..draft_of(&Period::Monthly)
            };
            let period = draft.to_period().expect("filled draft builds");
            assert_eq!(PeriodKind::of(&period), Some(kind));
        }
    }
}
