//! Settings-derived defaults for building a [`bc_ipc::Period`] from a name,
//! and the app-level settings context the shell provides.

use bc_ipc::SettingsInfo;
use jiff::civil::Date;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

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
    pub fn from_settings(info: &SettingsInfo) -> Self {
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

/// Settings the shell runs on when `get_settings` fails.
///
/// The period fields come from [`PeriodDefaults::FALLBACK`]. The rest mirror
/// `bc_config::Settings::default()`. Its database path depends on platform
/// directories the browser cannot see, so this takes the name `bc_config`
/// falls back to when no platform directory resolves.
///
/// # Returns
///
/// A [`SettingsInfo`] built from the fallback values.
#[must_use]
pub fn fallback_settings() -> SettingsInfo {
    let defaults = PeriodDefaults::FALLBACK;
    SettingsInfo::new(
        defaults.fy_start_month,
        defaults.fy_start_day,
        defaults.fortnightly_anchor.map(|anchor| anchor.to_string()),
        "AUD",
        "borrow-checker.db",
        Vec::new(),
        None,
    )
}

// MARK: Context

/// Reactive handle to the loaded settings, provided once by the shell.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct SettingsStore(pub ReadSignal<SettingsInfo>);

/// Provides the settings and the [`PeriodDefaults`] derived from them. Call
/// once, at the shell root, before anything reads the URL.
///
/// # Arguments
///
/// * `info` - The loaded settings, or [`fallback_settings`] on a load error.
///
/// # Returns
///
/// The derived [`PeriodDefaults`].
#[cfg(target_arch = "wasm32")]
pub fn provide_settings(info: SettingsInfo) -> PeriodDefaults {
    let defaults = PeriodDefaults::from_settings(&info);
    if let Some(raw) = info.fortnightly_anchor.as_deref()
        && defaults.fortnightly_anchor.is_none()
    {
        leptos::logging::warn!("fortnightly anchor {raw:?} is not a YYYY-MM-DD date; ignoring it");
    }
    provide_context(SettingsStore(RwSignal::new(info).read_only()));
    provide_context(defaults);
    defaults
}

/// Reads the configured [`PeriodDefaults`]; outside the shell,
/// [`PeriodDefaults::FALLBACK`].
///
/// # Returns
///
/// The defaults from context, or the fallback.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn use_period_defaults() -> PeriodDefaults {
    use_context::<PeriodDefaults>().unwrap_or(PeriodDefaults::FALLBACK)
}

/// Reads the loaded settings; `None` outside the shell.
///
/// # Returns
///
/// A snapshot of the settings from context.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn use_settings() -> Option<SettingsInfo> {
    use_context::<SettingsStore>().map(|store| store.0.get_untracked())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[test]
    fn from_settings_maps_every_field() {
        let info = SettingsInfo::new(
            1,
            1,
            Some("2026-03-02".to_owned()),
            "AUD",
            "/tmp/ledger.db",
            vec![],
            None,
        );
        assert_eq!(
            PeriodDefaults::from_settings(&info),
            PeriodDefaults {
                fy_start_month: 1,
                fy_start_day: 1,
                fortnightly_anchor: Some(date(2026, 3, 2)),
            }
        );
    }

    #[test]
    fn fallback_settings_carry_the_fallback_defaults() {
        let info = fallback_settings();
        assert_eq!(
            PeriodDefaults::from_settings(&info),
            PeriodDefaults::FALLBACK
        );
        assert_eq!(info.display_commodity, "AUD");
        assert_eq!(info.plugin_paths, Vec::<String>::new());
    }

    #[rstest]
    #[case::valid(Some("2026-03-02"), Some(date(2026, 3, 2)))]
    #[case::garbage(Some("not-a-date"), None)]
    #[case::unset(None, None)]
    fn from_settings_parses_the_anchor(#[case] raw: Option<&str>, #[case] want: Option<Date>) {
        let info = SettingsInfo::new(
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

    /// The database path is left out: `bc_config` resolves it from platform
    /// directories, which differ per machine.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn fallback_settings_match_the_config_defaults() {
        let config = bc_config::Settings::default();
        let info = fallback_settings();
        assert_eq!(
            (
                info.financial_year_start_month,
                info.financial_year_start_day,
                info.fortnightly_anchor,
                info.display_commodity,
            ),
            (
                config.financial_year_start_month(),
                config.financial_year_start_day(),
                config.fortnightly_anchor().map(|anchor| anchor.to_string()),
                config.display_commodity().to_string(),
            )
        );
    }
}
