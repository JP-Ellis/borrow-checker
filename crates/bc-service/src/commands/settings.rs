//! Command handler for settings.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
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
pub fn get_settings() -> Result<bc_ipc::SettingsInfo, bc_ipc::BcError> {
    let settings =
        bc_config::Settings::load().map_err(|e| bc_ipc::BcError::Internal(e.to_string()))?;

    Ok(bc_ipc::SettingsInfo::from(&settings))
}
