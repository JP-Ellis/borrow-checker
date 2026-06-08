//! Tauri command handler for settings.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers
//! `clippy::module_name_repetitions`; it is suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]

// MARK: Command handlers

/// Returns the current application settings as a serialisable snapshot.
///
/// Loads settings fresh from the config hierarchy on every call. This is a
/// cheap read (no I/O beyond the config file) and settings rarely change at
/// runtime, so no caching is required.
///
/// # Errors
///
/// Returns [`bc_ipc::BcError::Internal`] if the configuration cannot be
/// loaded (e.g. malformed config file, out-of-range field values).
#[tauri::command(rename_all = "snake_case")]
pub async fn get_settings() -> Result<bc_ipc::SettingsInfo, bc_ipc::BcError> {
    let config_file_paths = bc_config::Settings::config_file_paths();
    let config_file_path = config_file_paths
        .iter()
        .find(|p| p.exists())
        .or_else(|| config_file_paths.first())
        .map(|p| p.to_string_lossy().into_owned());

    let settings =
        bc_config::Settings::load().map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::SettingsInfo::new(
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
    ))
}
