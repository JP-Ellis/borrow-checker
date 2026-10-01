//! BorrowChecker desktop GUI — Tauri host.
//!
//! One Tauri command, `rpc`, forwards every call to
//! [`bc_service::dispatch`]; the frontend's transport sends `{cmd, args}`.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

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
/// The database opens before Tauri starts. When it cannot, such as while the
/// web server holds its lock, a native dialog says why and the app exits.
///
/// # Panics
///
/// Panics if Tauri cannot initialise the `WebView` runtime. This is
/// unrecoverable for a desktop GUI.
#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable for a desktop GUI"
)]
#[inline]
pub fn run() {
    let state = open_state().unwrap_or_else(|message| fail_to_start(&message));
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![rpc])
        .manage(state)
        .run(tauri::generate_context!())
        .expect("error while running borrow-checker");
}

/// Loads the settings and opens the shared services.
///
/// # Returns
///
/// The opened state.
///
/// # Errors
///
/// Returns the reason as display text. A config that fails to load could
/// name a different database, so it is an error rather than a fallback to
/// the defaults.
fn open_state() -> Result<bc_service::AppState, String> {
    let settings = bc_config::Settings::load().map_err(|e| e.to_string())?;
    tauri::async_runtime::block_on(bc_service::AppState::open(&settings)).map_err(|e| e.to_string())
}

/// Reports why the app cannot start, on stderr and in a dialog, then exits
/// with status 1.
///
/// # Arguments
///
/// * `message` - The reason, shown to the user.
#[expect(
    clippy::print_stderr,
    clippy::exit,
    reason = "a startup failure ends the process before any window exists"
)]
fn fail_to_start(message: &str) -> ! {
    eprintln!("error: {message}");
    // The dialog's result is only which button closed it.
    drop(
        rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Error)
            .set_title("BorrowChecker cannot start")
            .set_description(message)
            .set_buttons(rfd::MessageButtons::Ok)
            .show(),
    );
    std::process::exit(1);
}
