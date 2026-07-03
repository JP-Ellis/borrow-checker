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
    let settings =
        bc_config::Settings::load().map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::SettingsInfo::from(&settings))
}
