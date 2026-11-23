//! BorrowChecker desktop GUI — Tauri host.
//!
//! One Tauri command, `rpc`, forwards every call to
//! [`bc_service::dispatch`]; the frontend's transport sends `{cmd, args}`.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use tauri::AppHandle;
use tauri::Manager as _;
use tauri::Url;
use tauri::WebviewUrl;
use tauri::WebviewWindow;
use tauri::WebviewWindowBuilder;
use tauri::utils::config::WindowConfig;
use tauri::webview::NewWindowResponse;

/// Label of the window `Tauri.toml` declares.
const MAIN_WINDOW: &str = "main";

/// Label prefix of the windows a `_blank` link opens.
/// `capabilities/default.toml` grants them the main window's permissions.
const LINK_WINDOW_PREFIX: &str = "link-";

/// Numbers each link window, so every label is unique.
static LINK_WINDOWS: AtomicUsize = AtomicUsize::new(0);

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
        .setup(|app| {
            let config = window_config(app.handle(), MAIN_WINDOW)?;
            build_window(app.handle(), &config)?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running borrow-checker");
}

/// Looks up a window's declared configuration.
///
/// # Arguments
///
/// * `app` - The running app.
/// * `label` - The window's label in `Tauri.toml`.
///
/// # Returns
///
/// A copy of the window's configuration.
///
/// # Errors
///
/// Returns [`tauri::Error::WindowNotFound`] when `Tauri.toml` declares no
/// window with that label.
fn window_config(app: &AppHandle, label: &str) -> tauri::Result<WindowConfig> {
    app.config()
        .app
        .windows
        .iter()
        .find(|window| window.label == label)
        .cloned()
        .ok_or(tauri::Error::WindowNotFound)
}

/// Builds a window whose `_blank` links open in a new app window.
///
/// The webview drops a `_blank` request unless a handler answers it, so a
/// link that keeps its page's unsaved state, such as the account picker's
/// "manage accounts ↗", would otherwise do nothing.
///
/// # Arguments
///
/// * `app` - The running app.
/// * `config` - The window to build.
///
/// # Returns
///
/// The built window.
///
/// # Errors
///
/// Returns the error Tauri raises when the window cannot be created.
fn build_window(app: &AppHandle, config: &WindowConfig) -> tauri::Result<WebviewWindow> {
    let handle = app.clone();
    let opener = config.label.clone();
    WebviewWindowBuilder::from_config(app, config)?
        .on_new_window(move |url, _features| {
            open_link_window(&handle, &opener, &url);
            NewWindowResponse::Deny
        })
        .build()
}

/// Opens `url` in a new app window when it points into the app.
///
/// The new window loads the app afresh at the link's route and shares
/// nothing with its opener, as `rel="noopener"` asks. A link to another
/// origin is dropped.
///
/// # Arguments
///
/// * `app` - The running app.
/// * `opener` - Label of the window the link was followed in.
/// * `url` - The link's resolved URL.
fn open_link_window(app: &AppHandle, opener: &str, url: &Url) {
    let Some(current) = app
        .get_webview_window(opener)
        .and_then(|window| window.url().ok())
    else {
        return;
    };
    let Some(route) = app_route(&current, url) else {
        tracing::warn!(%url, "dropped a new-window request outside the app");
        return;
    };
    let Ok(mut config) = window_config(app, MAIN_WINDOW) else {
        return;
    };
    let number = LINK_WINDOWS.fetch_add(1, Ordering::Relaxed);
    config.label = format!("{LINK_WINDOW_PREFIX}{number}");
    config.url = WebviewUrl::App(route.into());
    // The handler runs inside the webview's own callback; building the
    // window there can deadlock on Windows, so it is built on the runtime.
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = build_window(&handle, &config) {
            tracing::warn!(%error, "could not open a link window");
        }
    });
}

/// The route a link names within the app, when it stays in the app.
///
/// # Arguments
///
/// * `current` - The URL of the page the link was followed from.
/// * `link` - The link's resolved URL.
///
/// # Returns
///
/// The link's path and query, or `None` when its scheme, host or port
/// differs from the page's. `Url::origin` cannot decide this: the bundled
/// app's `tauri://` scheme has an opaque origin, which equals nothing.
fn app_route(current: &Url, link: &Url) -> Option<String> {
    let same_site = link.scheme() == current.scheme()
        && link.host_str() == current.host_str()
        && link.port_or_known_default() == current.port_or_known_default();
    if !same_site {
        return None;
    }
    let mut route = link.path().to_owned();
    if let Some(query) = link.query() {
        route.push('?');
        route.push_str(query);
    }
    Some(route)
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use tauri::Url;

    use crate::app_route;

    #[rstest]
    #[case::bundled(
        "tauri://localhost/accounts/1",
        "tauri://localhost/settings?section=accounts&new=asset",
        Some("/settings?section=accounts&new=asset")
    )]
    #[case::dev_server(
        "http://localhost:1420/accounts/1",
        "http://localhost:1420/settings?section=accounts",
        Some("/settings?section=accounts")
    )]
    #[case::no_query("tauri://localhost/", "tauri://localhost/settings", Some("/settings"))]
    #[case::other_host("tauri://localhost/", "https://example.com/settings", None)]
    #[case::other_port("http://localhost:1420/", "http://localhost:1421/settings", None)]
    #[case::other_scheme("tauri://localhost/", "http://localhost/settings", None)]
    fn a_link_stays_in_the_app_only_on_the_same_origin(
        #[case] current: &str,
        #[case] link: &str,
        #[case] expected: Option<&str>,
    ) {
        let current_url = Url::parse(current).expect("valid current URL");
        let link_url = Url::parse(link).expect("valid link URL");
        assert_eq!(app_route(&current_url, &link_url).as_deref(), expected);
    }
}
