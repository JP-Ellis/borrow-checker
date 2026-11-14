//! Settings-derived defaults for building a [`bc_ipc::Period`] from a name.

use jiff::civil::Date;

/// The configured values a period needs but its name alone does not carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeriodDefaults {
    /// Financial year start month (1–12).
    pub fy_start_month: u8,
    /// Financial year start day (1–28).
    pub fy_start_day: u8,
    /// Fortnightly cycle anchor, when one is configured.
    pub fortnightly_anchor: Option<Date>,
}

impl PeriodDefaults {
    /// Mirrors `bc_config::Settings::default()`: 1 July, no anchor.
    pub const FALLBACK: Self = Self {
        fy_start_month: 7,
        fy_start_day: 1,
        fortnightly_anchor: None,
    };

    /// Reads the defaults from the settings the backend reports.
    ///
    /// An anchor that does not parse as `YYYY-MM-DD` reads as unconfigured.
    #[must_use]
    #[cfg_attr(
        all(target_arch = "wasm32", not(test)),
        expect(
            dead_code,
            reason = "first consumer is the settings context added later"
        )
    )]
    pub fn from_settings(info: &bc_ipc::SettingsInfo) -> Self {
        Self {
            fy_start_month: info.financial_year_start_month,
            fy_start_day: info.financial_year_start_day,
            fortnightly_anchor: info
                .fortnightly_anchor
                .as_deref()
                .and_then(|text| text.parse::<Date>().ok()),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::valid(Some("2026-03-02"), Some(date(2026, 3, 2)))]
    #[case::garbage(Some("not a date"), None)]
    #[case::unset(None, None)]
    fn from_settings_parses_the_anchor(#[case] raw: Option<&str>, #[case] want: Option<Date>) {
        let info = bc_ipc::SettingsInfo::new(
            1,
            1,
            raw.map(str::to_owned),
            "AUD",
            "/tmp/ledger.db",
            vec![],
            None,
        );
        let got = PeriodDefaults::from_settings(&info);
        assert_eq!(got.fortnightly_anchor, want);
        assert_eq!((got.fy_start_month, got.fy_start_day), (1, 1));
    }
}
