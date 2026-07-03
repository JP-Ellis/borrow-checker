//! Conversion from `bc_config::Settings` into `bc_ipc::SettingsInfo`.
//!
//! This module is gated behind the `ipc` feature. Because the source type
//! (`Settings`) is local to this crate, the orphan rule permits `impl From`
//! even though the destination DTO lives in `bc-ipc`.

use crate::Settings;
use crate::config_file_paths;

/// Converts application [`Settings`] into the IPC [`bc_ipc::SettingsInfo`]
/// snapshot consumed by the WASM frontend.
///
/// `config_file_path` is resolved internally by scanning
/// [`config_file_paths`] for the first path that exists, falling back to the
/// first candidate path if none has been written yet.
impl From<&Settings> for bc_ipc::SettingsInfo {
    fn from(settings: &Settings) -> Self {
        let config_file_paths: Vec<_> = config_file_paths().collect();
        let config_file_path = config_file_paths
            .iter()
            .find(|p| p.exists())
            .or_else(|| config_file_paths.first())
            .map(|p| p.to_string_lossy().into_owned());

        bc_ipc::SettingsInfo::new(
            settings.financial_year_start_month(),
            settings.financial_year_start_day(),
            settings.fortnightly_anchor().map(|d| d.to_string()),
            settings.display_commodity().to_string(),
            settings.db_path().to_string_lossy(),
            settings
                .plugin_paths()
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect(),
            config_file_path,
        )
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn converts_default_settings() {
        let settings = Settings::default();
        let info = bc_ipc::SettingsInfo::from(&settings);

        assert_eq!(
            info.financial_year_start_month,
            settings.financial_year_start_month()
        );
        assert_eq!(
            info.display_commodity,
            settings.display_commodity().to_string()
        );
    }
}
