//! BorrowChecker web server: the Leptos UI and `POST /rpc/{cmd}`.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod assets;

use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::Path;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse as _;
use axum::response::Response;
use axum::routing::post;
use bc_ipc::BcError;
use bc_ipc::commands;
use tokio::sync::watch;

/// State shared by every request.
#[non_exhaustive]
pub struct Shared {
    /// Services over the open database.
    pub app: bc_service::AppState,
    /// Set to `true` after a restore; `main` then exits for a restart.
    pub restart: watch::Sender<bool>,
}

impl Shared {
    /// Wraps `app` for the router, with a restart flag starting at `false`.
    ///
    /// # Arguments
    ///
    /// * `app` - Services over the open database.
    ///
    /// # Returns
    ///
    /// The shared state and a receiver that sees the flag turn `true`.
    #[inline]
    #[must_use]
    pub fn new(app: bc_service::AppState) -> (Arc<Self>, watch::Receiver<bool>) {
        let (restart, rx) = watch::channel(false);
        (Arc::new(Self { app, restart }), rx)
    }
}

/// Builds the router: the RPC route plus the embedded frontend.
///
/// # Arguments
///
/// * `shared` - Request state.
///
/// # Returns
///
/// The router.
#[inline]
pub fn router(shared: Arc<Shared>) -> Router {
    Router::new()
        .route("/rpc/{cmd}", post(rpc))
        .fallback(assets::serve)
        .with_state(shared)
}

/// Maps a [`BcError`] to its HTTP status.
///
/// # Arguments
///
/// * `e` - The error.
///
/// # Returns
///
/// `404`, `409`, `422` or `500`.
#[inline]
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "BcError is #[non_exhaustive]; a future variant is a server error until mapped"
)]
pub fn status_for(e: &BcError) -> StatusCode {
    match e {
        BcError::NotFound(_) => StatusCode::NOT_FOUND,
        BcError::Validation(_) => StatusCode::UNPROCESSABLE_ENTITY,
        BcError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// `POST /rpc/{cmd}`.
async fn rpc(State(shared): State<Arc<Shared>>, Path(cmd): Path<String>, body: Bytes) -> Response {
    match run(&shared, &cmd, &body).await {
        Ok(value) => (StatusCode::OK, axum::Json(value)).into_response(),
        Err(e) => (status_for(&e), axum::Json(e)).into_response(),
    }
}

/// Parses the body, confines a restore, dispatches, and flags a restart.
async fn run(shared: &Shared, cmd: &str, body: &[u8]) -> Result<serde_json::Value, BcError> {
    let raw: serde_json::Value = if body.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_slice(body)
            .map_err(|e| BcError::Validation(format!("request body is not JSON: {e}")))?
    };
    let is_restore = cmd == commands::RESTORE_DATABASE;
    let args = if is_restore {
        confine_restore(shared, raw)?
    } else {
        raw
    };
    let out = bc_service::dispatch(&shared.app, cmd, args).await?;
    if is_restore {
        if under_systemd() {
            tracing::info!("restore scheduled; exiting for systemd to restart the server");
        } else {
            tracing::info!("restore scheduled; restart the server to apply it");
        }
        shared.restart.send_replace(true);
    }
    Ok(out)
}

/// Rewrites a restore's `path` to its canonical form inside the backup
/// directory the server opened with.
///
/// The root is frozen at startup, so a network caller cannot widen it by
/// first moving the backup directory through `update_backup_settings`.
fn confine_restore(shared: &Shared, args: serde_json::Value) -> Result<serde_json::Value, BcError> {
    let parsed: commands::RestoreDatabaseArgs = bc_service::parse_args(args)?;
    let confined =
        bc_service::confine_to_dir(std::path::Path::new(&parsed.path), &shared.app.backup_dir())?;
    if !confined.is_file() {
        return Err(BcError::Validation(format!(
            "cannot restore {}: not a regular file",
            parsed.path
        )));
    }
    Ok(serde_json::json!({ "path": confined.to_string_lossy() }))
}

/// Whether systemd started this process, so it restarts it after exit.
fn under_systemd() -> bool {
    std::env::var_os("INVOCATION_ID").is_some()
}
