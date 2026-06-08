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
