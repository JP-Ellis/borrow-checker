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
    /// Ordered list of plugin search directory paths as UTF-8 strings.
    pub plugin_paths: Vec<String>,
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
    /// * `plugin_paths` - Ordered list of plugin directory paths.
    #[inline]
    #[must_use]
    pub fn new(
        financial_year_start_month: u8,
        financial_year_start_day: u8,
        fortnightly_anchor: Option<String>,
        display_commodity: impl Into<String>,
        db_path: impl Into<String>,
        plugin_paths: Vec<String>,
    ) -> Self {
        Self {
            financial_year_start_month,
            financial_year_start_day,
            fortnightly_anchor,
            display_commodity: display_commodity.into(),
            db_path: db_path.into(),
            plugin_paths,
        }
    }
}
