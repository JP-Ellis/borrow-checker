//! Tauri command handlers for plugin operations.
//!
//! The `#[tauri::command]` macro generates wrapper code that triggers a few lints
//! on the `State<'_, AppState>` parameter; these are suppressed module-wide since
//! item-level `#[expect]` cannot reach macro-generated spans.
#![expect(
    clippy::module_name_repetitions,
    reason = "Tauri IPC command names must match bc-ipc contract; renaming is not an option"
)]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::State;

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
/// normal operation. The `Result` wrapper satisfies the Tauri command protocol.
#[expect(
    private_interfaces,
    reason = "Tauri command functions must be pub, but AppState is intentionally crate-private"
)]
#[tauri::command(rename_all = "snake_case")]
pub async fn list_plugins(
    state: State<'_, AppState>,
) -> Result<Vec<bc_ipc::PluginInfo>, bc_ipc::BcError> {
    Ok(state.plugins.clone())
}

// MARK: Startup helpers

/// Loads plugin metadata from configured search paths.
///
/// Builds a [`bc_plugins::PluginRegistry`] using paths from [`bc_config::Settings`],
/// then immediately converts the loaded plugins into plain [`bc_ipc::PluginInfo`]
/// values. This allows the metadata to be stored in [`AppState`] and cloned
/// cheaply, avoiding the need to store the non-`Clone` registry itself.
///
/// # Returns
///
/// A `Vec` of [`bc_ipc::PluginInfo`] for all successfully loaded plugins.
/// Returns an empty `Vec` if no plugins are found or the registry fails to
/// initialise.
pub(crate) fn collect_plugin_info() -> Vec<bc_ipc::PluginInfo> {
    use bc_core::Importer as _;

    let settings = bc_config::Settings::load().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to load settings; using defaults");
        bc_config::Settings::default()
    });
    let paths = settings.plugin_paths().to_owned();

    bc_plugins::PluginRegistry::load(&paths).map_or_else(
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
                .map(|p| {
                    let file_name = p
                        .source_path()
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_else(|| {
                            // A plugin path without a filename component is
                            // effectively impossible (the registry only loads
                            // *.wasm files from directories), but fall back
                            // gracefully rather than panicking.
                            tracing::warn!(
                                path = %p.source_path().display(),
                                "plugin source path has no valid filename component"
                            );
                            ""
                        })
                        .to_owned();
                    let is_deprecated = p.is_deprecated();
                    bc_ipc::PluginInfo::new(
                        p.name().to_owned(),
                        p.sdk_abi(),
                        file_name,
                        is_deprecated,
                    )
                })
                .collect()
        },
    )
}
