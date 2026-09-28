//! BorrowChecker desktop GUI — Tauri host.
//!
//! One Tauri command, [`rpc`], forwards every call to
//! [`bc_service::dispatch`]; the frontend's transport sends `{cmd, args}`.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![expect(
    clippy::let_underscore_must_use,
    reason = "tauri::command macro generates must-use bindings that cannot be suppressed per-item"
)]

use tauri::Manager as _;

/// Runs command `cmd` through the shared dispatcher.
///
/// A successful restore relaunches the app so `AppState::open` swaps the
/// candidate in before any connection opens.
///
/// # Arguments
///
/// * `cmd` - A name from [`bc_ipc::commands`].
/// * `args` - The command's argument object.
/// * `app` - Handle used to relaunch after a restore.
/// * `state` - The shared services.
///
/// # Returns
///
/// The command's serialised result.
///
/// # Errors
///
/// Returns the command's [`bc_ipc::BcError`].
#[tauri::command(rename_all = "snake_case")]
async fn rpc(
    cmd: String,
    args: serde_json::Value,
    app: tauri::AppHandle,
    state: tauri::State<'_, bc_service::AppState>,
) -> Result<serde_json::Value, bc_ipc::BcError> {
    let out = bc_service::dispatch(&state, &cmd, args).await?;
    if cmd == bc_ipc::commands::RESTORE_DATABASE {
        // `restart()` exits without dropping managed state, so the pool is
        // closed here to checkpoint the WAL before the restore swap runs.
        state.close().await;
        app.restart();
    }
    Ok(out)
}

/// Initialise and run the Tauri application.
///
/// # Panics
///
/// Panics if Tauri cannot initialise the `WebView` runtime. This is
/// unrecoverable for a desktop GUI.
#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable for a desktop GUI"
)]
#[expect(
    clippy::exit,
    reason = "tauri::generate_context!() macro internally calls process::exit"
)]
#[inline]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rpc])
        .setup(|app| {
            // A config that fails to load could name a different database, so
            // the app refuses to start instead of opening the default one.
            let settings = bc_config::Settings::load()?;
            let state = tauri::async_runtime::block_on(bc_service::AppState::open(&settings))?;
            app.manage(state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running borrow-checker");
}
