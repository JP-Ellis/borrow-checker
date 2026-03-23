//! Application configuration for BorrowChecker.
//!
//! Settings are loaded from a hierarchy: built-in defaults → user config
//! file → local project file → environment variables (`BC_` prefix).

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
    /// Financial year start month (1-based).
    financial_year_start_month: u8,
    /// Financial year start day (1-based).
    financial_year_start_day: u8,
    /// Fortnightly anchor date string, if set.
    fortnightly_anchor: Option<String>,
    /// Display commodity code string.
    display_commodity: String,
}

/// Validated application-wide settings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Settings {
    /// Financial year start month (1-based).
    financial_year_start_month: u8,
    /// Financial year start day (1-based).
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
    /// 2. `~/.config/borrow-checker/config.toml`
    /// 3. `./borrow-checker.toml`
    /// 4. Environment variables prefixed `BC_`
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError`] if any source fails to parse or a value is
    /// out of range.
    #[inline]
    pub fn load() -> Result<Self, ConfigError> {
        let home_config =
            home_dir().map(|h| h.join(".config").join("borrow-checker").join("config.toml"));

        let mut builder = config::Config::builder()
            .set_default("financial_year_start_month", 7_i64)?
            .set_default("financial_year_start_day", 1_i64)?
            .set_default("fortnightly_anchor", Option::<String>::None)?
            .set_default("display_commodity", "AUD")?;

        if let Some(path) = home_config {
            builder = builder.add_source(config::File::from(path).required(false));
        }
        builder = builder
            .add_source(config::File::with_name("borrow-checker").required(false))
            .add_source(config::Environment::with_prefix("BC").separator("__"));

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
                "financial_year_start_day {} is out of range 1–28",
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

/// Returns the user home directory by reading the `HOME` environment variable.
fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME").map(std::path::PathBuf::from)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

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
}
