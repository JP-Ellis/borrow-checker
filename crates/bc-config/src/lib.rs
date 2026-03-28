//! Application configuration for BorrowChecker.
//!
//! Settings are loaded from a hierarchy: built-in defaults → user config
//! file(s) → local project file → environment variables (`BC_` prefix).

use std::path::PathBuf;

use bc_models::CommodityCode;
use jiff::civil::Date;

/// Error returned when loading or validating configuration.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConfigError {
    /// A configuration source could not be read or parsed.
    #[error("configuration error: {0}")]
    Load(#[from] config::ConfigError),
    /// A field value is out of range.
    #[error("invalid configuration: {0}")]
    Validation(String),
}

/// Raw deserialized settings before validation.
#[derive(Debug, Clone, serde::Deserialize)]
struct RawSettings {
    /// Financial year start month (1-based, 1–12).
    financial_year_start_month: u8,
    /// Financial year start day (1-based, 1–28).
    ///
    /// Capped at 28 to ensure the start day exists in every calendar month,
    /// including February (which has at minimum 28 days). Use 1 for the
    /// safest cross-month anchor.
    financial_year_start_day: u8,
    /// Fortnightly anchor date string, if set.
    fortnightly_anchor: Option<String>,
    /// Display commodity code string.
    display_commodity: String,
}

/// Validated application-wide settings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Settings {
    /// Financial year start month (1-based, 1–12).
    financial_year_start_month: u8,
    /// Financial year start day (1-based, 1–28).
    ///
    /// Capped at 28 to ensure the start day exists in every calendar month,
    /// including February (which has at minimum 28 days). Use 1 for the
    /// safest cross-month anchor.
    ///
    /// # Example
    ///
    /// A value of `1` is always safe; `28` is the maximum accepted value.
    /// Values of 29, 30, or 31 are rejected during validation because they
    /// do not exist in every month.
    financial_year_start_day: u8,
    /// Fortnightly anchor date, if configured.
    fortnightly_anchor: Option<Date>,
    /// Display commodity code.
    display_commodity: CommodityCode,
}

impl Settings {
    /// Loads settings from the configuration hierarchy.
    ///
    /// Sources (lowest to highest priority):
    /// 1. Built-in defaults
    /// 2. `$XDG_CONFIG_HOME/borrow-checker/config.toml` (or `~/.config/…`
    ///    when `XDG_CONFIG_HOME` is unset)
    /// 3. Platform-native config directory (e.g.
    ///    `~/Library/Application Support/borrow-checker/config.toml` on macOS)
    /// 4. `./borrow-checker.toml`
    /// 5. Environment variables prefixed `BC_`
    ///
    /// Steps 2 and 3 are deduplicated when they resolve to the same path
    /// (common on Linux).  All file sources are optional.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if any source fails to parse or a value is
    /// out of range.
    #[inline]
    pub fn load() -> Result<Self, ConfigError> {
        let mut builder = config::Config::builder()
            .set_default("financial_year_start_month", 7_i64)?
            .set_default("financial_year_start_day", 1_i64)?
            .set_default("fortnightly_anchor", Option::<String>::None)?
            .set_default("display_commodity", "AUD")?;

        for path in user_config_paths() {
            builder = builder.add_source(config::File::from(path).required(false));
        }
        builder = builder
            .add_source(config::File::with_name("borrow-checker").required(false))
            .add_source(config::Environment::with_prefix("BC").separator("_"));

        let raw: RawSettings = builder.build()?.try_deserialize()?;
        Self::validate(raw)
    }

    /// Validates raw settings and returns a [`Settings`] instance.
    fn validate(raw: RawSettings) -> Result<Self, ConfigError> {
        if !(1..=12).contains(&raw.financial_year_start_month) {
            return Err(ConfigError::Validation(format!(
                "financial_year_start_month {} is out of range 1–12",
                raw.financial_year_start_month
            )));
        }
        if !(1..=28).contains(&raw.financial_year_start_day) {
            return Err(ConfigError::Validation(format!(
                "financial_year_start_day must be between 1 and 28 (capped at 28 so the day \
                 exists in every month, including February); got {}",
                raw.financial_year_start_day
            )));
        }
        let fortnightly_anchor = raw
            .fortnightly_anchor
            .map(|s| {
                s.parse::<Date>().map_err(|e| {
                    ConfigError::Validation(format!("invalid fortnightly_anchor '{s}': {e}"))
                })
            })
            .transpose()?;

        if raw.display_commodity.is_empty() {
            return Err(ConfigError::Validation(
                "display_commodity must not be empty".into(),
            ));
        }

        Ok(Self {
            financial_year_start_month: raw.financial_year_start_month,
            financial_year_start_day: raw.financial_year_start_day,
            fortnightly_anchor,
            display_commodity: CommodityCode::new(raw.display_commodity),
        })
    }

    /// Returns the financial year start month (1-based).
    #[inline]
    #[must_use]
    pub fn financial_year_start_month(&self) -> u8 {
        self.financial_year_start_month
    }

    /// Returns the financial year start day (1-based).
    #[inline]
    #[must_use]
    pub fn financial_year_start_day(&self) -> u8 {
        self.financial_year_start_day
    }

    /// Returns the fortnightly anchor date, if configured.
    #[inline]
    #[must_use]
    pub fn fortnightly_anchor(&self) -> Option<Date> {
        self.fortnightly_anchor
    }

    /// Returns the display commodity code.
    #[inline]
    #[must_use]
    pub fn display_commodity(&self) -> &CommodityCode {
        &self.display_commodity
    }
}

impl Default for Settings {
    #[inline]
    fn default() -> Self {
        Self {
            financial_year_start_month: 7,
            financial_year_start_day: 1,
            fortnightly_anchor: None,
            display_commodity: CommodityCode::new("AUD"),
        }
    }
}

/// Returns the ordered list of user config file paths to load.
///
/// Priority (lowest first, so later entries override earlier ones):
/// 1. XDG path: `$XDG_CONFIG_HOME/borrow-checker/config.toml`, falling back
///    to `~/.config/borrow-checker/config.toml` when `XDG_CONFIG_HOME` is
///    unset (the XDG Base Directory default).
/// 2. Platform-native path from the [`directories`] crate (e.g.
///    `~/Library/Application Support/borrow-checker/config.toml` on macOS).
///
/// The two paths are deduplicated when they resolve to the same location,
/// which is the common case on Linux (where `directories` already honours
/// `XDG_CONFIG_HOME`).
fn user_config_paths() -> Vec<PathBuf> {
    // XDG path: $XDG_CONFIG_HOME or fall back to $HOME/.config.
    // Per the XDG Base Directory Specification, XDG_CONFIG_HOME must be an
    // absolute path; non-absolute values are ignored.
    let xdg_base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| directories::BaseDirs::new().map(|b| b.home_dir().join(".config")));
    let xdg_path = xdg_base.map(|b| b.join("borrow-checker").join("config.toml"));

    // Platform-native path via the directories crate
    let native_path = directories::ProjectDirs::from("", "", "borrow-checker")
        .map(|p| p.config_dir().join("config.toml"));

    let mut paths: Vec<PathBuf> = Vec::new();
    if let Some(xdg) = xdg_path {
        paths.push(xdg);
    }
    if let Some(native) = native_path {
        if !paths.contains(&native) {
            paths.push(native);
        }
    }
    paths
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;

    use super::*;

    /// Builds a fully-valid [`RawSettings`] that passes validation so individual
    /// tests can override exactly one field at a time.
    fn valid_raw() -> RawSettings {
        RawSettings {
            financial_year_start_month: 7,
            financial_year_start_day: 1,
            fortnightly_anchor: None,
            display_commodity: "AUD".to_owned(),
        }
    }

    #[test]
    fn default_settings_fy_start_is_july() {
        let s = Settings::default();
        assert_eq!(s.financial_year_start_month(), 7);
        assert_eq!(s.financial_year_start_day(), 1);
    }

    #[test]
    fn load_returns_defaults_with_no_config_files() {
        let s = Settings::load().expect("load should succeed with no files");
        assert_eq!(s.financial_year_start_month(), 7);
    }

    #[test]
    fn user_config_paths_contains_xdg_path_when_env_is_set() {
        // SAFETY: Tests run in isolated processes under nextest; no concurrent
        // threads are reading environment variables.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "/tmp/bc_test_xdg_9f3a") }
        let paths = user_config_paths();
        // SAFETY: Same as above — isolated process, no concurrent env access.
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") }
        assert!(
            paths.contains(&PathBuf::from(
                "/tmp/bc_test_xdg_9f3a/borrow-checker/config.toml"
            )),
            "expected XDG path in list, got: {paths:?}"
        );
    }

    #[test]
    fn user_config_paths_ignores_relative_xdg_config_home() {
        // SAFETY: Tests run in isolated processes under nextest; no concurrent
        // threads are reading environment variables.
        unsafe { std::env::set_var("XDG_CONFIG_HOME", "relative/path") }
        let paths = user_config_paths();
        // SAFETY: Same as above — isolated process, no concurrent env access.
        unsafe { std::env::remove_var("XDG_CONFIG_HOME") }
        assert!(
            !paths.iter().any(|p| p.starts_with("relative/path")),
            "relative XDG_CONFIG_HOME must be ignored per XDG spec; got: {paths:?}"
        );
    }

    #[test]
    fn user_config_paths_has_no_duplicates() {
        let paths = user_config_paths();
        let mut seen = std::collections::HashSet::new();
        for p in &paths {
            assert!(seen.insert(p.clone()), "duplicate path found: {p:?}");
        }
    }

    // --- Validation error paths ---

    #[test]
    fn invalid_fy_start_month_zero() {
        let raw = RawSettings {
            financial_year_start_month: 0,
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "month 0 should fail validation"
        );
    }

    #[test]
    fn invalid_fy_start_month_thirteen() {
        let raw = RawSettings {
            financial_year_start_month: 13,
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "month 13 should fail validation"
        );
    }

    #[test]
    fn invalid_fy_start_day_zero() {
        let raw = RawSettings {
            financial_year_start_day: 0,
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "day 0 should fail validation"
        );
    }

    #[test]
    fn invalid_fy_start_day_twenty_nine() {
        let raw = RawSettings {
            financial_year_start_day: 29,
            ..valid_raw()
        };
        let result = Settings::validate(raw);
        assert!(
            result.is_err(),
            "day 29 should fail validation (capped at 28)"
        );
        let err_msg = result.expect_err("already asserted is_err").to_string();
        assert!(
            err_msg.contains("28"),
            "error message should mention the cap of 28; got: {err_msg}"
        );
    }

    #[test]
    fn invalid_fortnightly_anchor_string() {
        let raw = RawSettings {
            fortnightly_anchor: Some("not-a-date".to_owned()),
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "invalid anchor date string should fail validation"
        );
    }

    #[test]
    fn invalid_empty_display_commodity() {
        let raw = RawSettings {
            display_commodity: String::new(),
            ..valid_raw()
        };
        assert!(
            Settings::validate(raw).is_err(),
            "empty display_commodity should fail validation"
        );
    }
}
