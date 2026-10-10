//! Command handlers for plugin operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use crate::AppState;

// MARK: Command handlers

/// List all installed plugins.
///
/// Reads the registry [`AppState`] loaded at startup. Plugins are not
/// hot-reloaded, so the list is fixed for the life of the process.
///
/// # Errors
///
/// This command does not perform I/O at call time and will not fail under
/// normal operation. The `Result` wrapper matches every other command.
pub fn list_plugins(state: &AppState) -> Result<Vec<bc_ipc::PluginInfo>, bc_ipc::BcError> {
    Ok(collect_plugin_info(state.plugin_registry.as_ref()))
}

// MARK: Startup helpers

/// Loads the plugins in the configured search paths, each preopening
/// `import.documents-root`.
///
/// # Arguments
///
/// * `settings` - The settings the host loaded at startup.
///
/// # Returns
///
/// The registry, or `None` if the Wasmtime engine cannot be created; the
/// failure is logged and the host starts without plugins.
pub(crate) fn load_plugin_registry(
    settings: &bc_config::Settings,
) -> Option<bc_plugins::PluginRegistry> {
    bc_plugins::PluginRegistry::load(settings.plugin_paths(), settings.documents_root())
        .inspect_err(|e| {
            tracing::warn!(
                error = %e,
                "plugin registry failed to initialise; no plugins will be available"
            );
        })
        .ok()
}

/// Describes every plugin in `registry`.
///
/// # Arguments
///
/// * `registry` - The registry loaded at startup, if it loaded.
///
/// # Returns
///
/// One [`bc_ipc::PluginInfo`] per loaded plugin, empty without a registry.
pub(crate) fn collect_plugin_info(
    registry: Option<&bc_plugins::PluginRegistry>,
) -> Vec<bc_ipc::PluginInfo> {
    registry.map_or_else(Vec::new, |loaded| {
        loaded
            .plugins()
            .map(|p| bc_ipc::PluginInfo::from(p.as_ref()))
            .collect()
    })
}
