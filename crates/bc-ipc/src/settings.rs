//! Settings IPC type — read-only view of `bc_config::Settings`.

use serde::Deserialize;
use serde::Serialize;

/// Read-only snapshot of application settings for the frontend.
///
/// Produced by the native `get_settings` Tauri command and consumed by
/// the WASM frontend to render the settings page.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct SettingsInfo {
    /// Financial year start month (1-based, 1–12).
    pub financial_year_start_month: u8,
    /// Financial year start day (1-based, 1–28).
    pub financial_year_start_day: u8,
    /// Fortnightly anchor date as `"YYYY-MM-DD"`, or `None` if not configured.
    pub fortnightly_anchor: Option<String>,
    /// Display commodity code (e.g. `"AUD"`).
    pub display_commodity: String,
    /// Resolved database file path as a UTF-8 string.
    pub db_path: String,
    /// Filename component of `db_path` (e.g. `"db.sqlite"`).
    ///
    /// Extracted on the native side so the WASM component stays purely
    /// presentational and avoids platform-specific path splitting.
    pub db_filename: String,
    /// Ordered list of plugin search directory paths as UTF-8 strings.
    pub plugin_paths: Vec<String>,
    /// First candidate config file path on this platform.
    ///
    /// Used by the UI to show the user where to find the config file without
    /// hardcoding platform-specific paths.
    pub config_file_path: Option<String>,
}

impl SettingsInfo {
    /// Creates a new [`SettingsInfo`] with all fields.
    ///
    /// # Arguments
    ///
    /// * `financial_year_start_month` - Financial year start month (1–12).
    /// * `financial_year_start_day` - Financial year start day (1–28).
    /// * `fortnightly_anchor` - Optional anchor date as `"YYYY-MM-DD"`.
    /// * `display_commodity` - Display commodity code string.
    /// * `db_path` - Resolved database file path as a string.
    /// * `db_filename` - Filename component of the database path.
    /// * `plugin_paths` - Ordered list of plugin directory paths.
    /// * `config_file_path` - First candidate config file path, if determinable.
    #[expect(
        clippy::too_many_arguments,
        reason = "All fields are required; a builder would be over-engineering for a read-only IPC snapshot"
    )]
    #[inline]
    #[must_use]
    pub fn new(
        financial_year_start_month: u8,
        financial_year_start_day: u8,
        fortnightly_anchor: Option<String>,
        display_commodity: impl Into<String>,
        db_path: impl Into<String>,
        db_filename: impl Into<String>,
        plugin_paths: Vec<String>,
        config_file_path: Option<String>,
    ) -> Self {
        Self {
            financial_year_start_month,
            financial_year_start_day,
            fortnightly_anchor,
            display_commodity: display_commodity.into(),
            db_path: db_path.into(),
            db_filename: db_filename.into(),
            plugin_paths,
            config_file_path,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn serde_roundtrip_with_anchor_and_plugins() {
        let info = SettingsInfo::new(
            7,
            1,
            Some("2026-01-15".to_owned()),
            "AUD",
            "/home/alice/.local/share/borrow-checker/db.sqlite",
            "db.sqlite",
            vec!["/home/alice/.local/share/borrow-checker/plugins".to_owned()],
            Some("/home/alice/.config/borrow-checker/config.toml".to_owned()),
        );
        let json = serde_json::to_string(&info).expect("serialises");
        let info2: SettingsInfo = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(info, info2);
    }

    #[test]
    fn serde_roundtrip_no_anchor_empty_plugins() {
        let info = SettingsInfo::new(
            1,
            1,
            None,
            "USD",
            "/data/db.sqlite",
            "db.sqlite",
            vec![],
            None,
        );
        let json = serde_json::to_string(&info).expect("serialises");
        let info2: SettingsInfo = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(info, info2);
    }
}
