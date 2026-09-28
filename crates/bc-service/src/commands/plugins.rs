//! Command handlers for plugin operations.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use crate::AppState;

// MARK: Command handlers

/// List all installed plugins.
///
/// Returns the plugin metadata collected at application startup. The list is
/// static for the lifetime of the process (plugins are not hot-reloaded).
///
/// # Errors
///
/// This command does not perform I/O at call time and will not fail under
/// normal operation. The `Result` wrapper matches every other command.
pub fn list_plugins(state: &AppState) -> Result<Vec<bc_ipc::PluginInfo>, bc_ipc::BcError> {
    Ok(state.plugins.clone())
}

// MARK: Startup helpers

/// Loads plugin metadata from configured search paths.
///
/// Builds a [`bc_plugins::PluginRegistry`] using paths from `settings`, then
/// immediately converts the loaded plugins into plain [`bc_ipc::PluginInfo`]
/// values. This allows the metadata to be stored in [`AppState`] and cloned
/// cheaply, avoiding the need to store the non-`Clone` registry itself.
///
/// # Arguments
///
/// * `settings` - The settings the app loaded at startup.
///
/// # Returns
///
/// A `Vec` of [`bc_ipc::PluginInfo`] for all successfully loaded plugins.
/// Returns an empty `Vec` if no plugins are found or the registry fails to
/// initialise.
pub(crate) fn collect_plugin_info(settings: &bc_config::Settings) -> Vec<bc_ipc::PluginInfo> {
    let paths = settings.plugin_paths().to_owned();

    bc_plugins::PluginRegistry::load(&paths, settings.documents_root()).map_or_else(
        |e| {
            tracing::warn!(
                error = %e,
                "plugin registry failed to initialise; no plugins will be available"
            );
            Vec::new()
        },
        |registry| {
            registry
                .plugins()
                .map(|p| bc_ipc::PluginInfo::from(p.as_ref()))
                .collect()
        },
    )
}
