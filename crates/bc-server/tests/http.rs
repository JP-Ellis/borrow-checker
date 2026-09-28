//! The router over a real database on an ephemeral port.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]
#![expect(clippy::expect_used, reason = "tests panic on setup failure")]

use std::path::Path;

use axum::http::StatusCode;
use bc_ipc::BcError;
use pretty_assertions::assert_eq;
use rstest::rstest;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::watch;

/// A server bound to an ephemeral port over a database in a tempdir.
struct Running {
    /// `http://127.0.0.1:<port>`.
    base: String,
    /// Sees the restart flag that a restore sets.
    restart: watch::Receiver<bool>,
    /// Holds the database, backups and config; removed on drop.
    dir: TempDir,
}

impl Running {
    /// The backup directory the server opened with.
    fn backups(&self) -> std::path::PathBuf {
        self.dir.path().join("backups")
    }
}

/// Opens a fresh database in a tempdir and serves the router over it.
async fn start() -> Running {
    let dir = tempfile::tempdir().expect("tempdir");
    // SAFETY: nextest runs each test in its own process, and this runs before
    // the test opens the database or spawns any thread that reads the
    // environment.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config")) }
    let mut settings = bc_config::Settings::default();
    settings.set_db_path(dir.path().join("ledger.db"));
    settings.set_backup_dir(dir.path().join("backups"));
    std::fs::create_dir_all(dir.path().join("backups")).expect("mkdir backups");
    let app = bc_service::AppState::open(&settings).await.expect("open");
    let (shared, restart) = bc_server::Shared::new(app);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move {
        axum::serve(listener, bc_server::router(shared))
            .await
            .expect("serve");
    });
    Running { base, restart, dir }
}

/// POSTs `body` to `/rpc/{cmd}` and returns the status and body text.
async fn post(base: &str, cmd: &str, body: &str) -> (u16, String) {
    let r = reqwest::Client::new()
        .post(format!("{base}/rpc/{cmd}"))
        .header("content-type", "application/json")
        .body(body.to_owned())
        .send()
        .await
        .expect("send");
    (r.status().as_u16(), r.text().await.expect("text"))
}

/// Asks the server to restore `path`.
async fn restore(base: &str, path: &Path) -> (u16, String) {
    let body = json!({ "path": path.to_string_lossy() }).to_string();
    post(base, "restore_database", &body).await
}

/// Takes a manual backup through the server and returns its path.
async fn take_backup(base: &str) -> std::path::PathBuf {
    let (status, body) = post(base, "backup_database", "{}").await;
    assert_eq!(status, 200, "{body}");
    let info: Value = serde_json::from_str(&body).expect("json");
    info.get("path")
        .and_then(Value::as_str)
        .expect("path")
        .into()
}

#[tokio::test]
async fn a_command_round_trips() {
    let s = start().await;
    let (status, body) = post(&s.base, "list_tags", "{}").await;
    assert_eq!(status, 200);
    assert_eq!(body, "[]");
}

#[tokio::test]
async fn an_empty_body_counts_as_no_arguments() {
    let s = start().await;
    let (status, _) = post(&s.base, "list_tags", "").await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn a_body_that_is_not_json_is_422() {
    let s = start().await;
    let (status, body) = post(&s.base, "list_tags", "{not json").await;
    assert_eq!(status, 422);
    assert!(body.contains("not JSON"), "{body}");
}

#[tokio::test]
async fn an_unknown_command_is_404_with_a_bc_error() {
    let s = start().await;
    let (status, body) = post(&s.base, "drop_everything", "{}").await;
    assert_eq!(status, 404);
    let err: BcError = serde_json::from_str(&body).expect("a BcError body");
    assert!(matches!(err, BcError::NotFound(_)), "{err:?}");
}

#[tokio::test]
async fn bad_arguments_are_422() {
    let s = start().await;
    let (status, _) = post(&s.base, "reverse_transaction", r#"{"id":7}"#).await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn a_restore_outside_the_backup_directory_is_refused() {
    let s = start().await;
    let outside = s.dir.path().join("elsewhere.db");
    std::fs::write(&outside, b"x").expect("write");

    let (status, body) = restore(&s.base, &outside).await;

    assert_eq!(status, 422);
    assert!(body.contains("not in the backup directory"), "{body}");
    assert!(!*s.restart.borrow(), "a refused restore must not restart");
}

#[tokio::test]
async fn a_restore_with_malformed_arguments_is_422() {
    let s = start().await;
    let (status, body) = post(&s.base, "restore_database", r#"{"path":7}"#).await;
    assert_eq!(status, 422);
    assert!(body.contains("invalid arguments"), "{body}");
}

#[tokio::test]
async fn a_restore_of_the_backup_directory_itself_is_refused() {
    let s = start().await;
    let (status, body) = restore(&s.base, &s.backups()).await;
    assert_eq!(status, 422);
    assert!(body.contains("not a regular file"), "{body}");
}

#[tokio::test]
async fn moving_the_backup_directory_does_not_move_the_restore_root() {
    let s = start().await;
    let backup = take_backup(&s.base).await;
    let moved = s.dir.path().join("moved");
    std::fs::create_dir_all(&moved).expect("mkdir");
    let outside = moved.join("copied.db");
    std::fs::copy(&backup, &outside).expect("copy");
    let settings = json!({
        "settings": {
            "dir": moved.to_string_lossy(),
            "retain_count": null,
            "retain_days": null,
            "auto_pre_migration": false,
        }
    });
    let (moved_status, moved_body) =
        post(&s.base, "update_backup_settings", &settings.to_string()).await;
    assert_eq!(moved_status, 200, "{moved_body}");

    let (status, body) = restore(&s.base, &outside).await;

    assert_eq!(status, 422);
    assert!(body.contains("not in the backup directory"), "{body}");
}

#[tokio::test]
async fn a_restore_inside_the_backup_directory_flags_a_restart() {
    let s = start().await;
    let backup = take_backup(&s.base).await;

    let (status, body) = restore(&s.base, &backup).await;

    assert_eq!(status, 200, "{body}");
    assert!(*s.restart.borrow(), "a restore must flag a restart");
}

#[tokio::test]
async fn get_on_rpc_is_405() {
    let s = start().await;
    let r = reqwest::get(format!("{}/rpc/list_tags", s.base))
        .await
        .expect("get");
    assert_eq!(r.status().as_u16(), 405);
}

#[tokio::test]
async fn a_deep_link_serves_the_app_shell_and_a_missing_asset_is_404() {
    let s = start().await;
    let deep = reqwest::get(format!("{}/accounts/abc", s.base))
        .await
        .expect("get");
    let asset = reqwest::get(format!("{}/missing.js", s.base))
        .await
        .expect("get");
    // Without a built frontend the shell answers 503 with build instructions;
    // with one it is the index page. Either way it is not a 404.
    assert!(
        matches!(deep.status().as_u16(), 200 | 503),
        "{}",
        deep.status()
    );
    assert_eq!(asset.status().as_u16(), 404);
}

#[rstest]
#[case::not_found(BcError::NotFound("x".to_owned()), StatusCode::NOT_FOUND)]
#[case::validation(BcError::Validation("x".to_owned()), StatusCode::UNPROCESSABLE_ENTITY)]
#[case::conflict(BcError::Conflict("x".to_owned()), StatusCode::CONFLICT)]
#[case::internal(BcError::Internal("x".to_owned()), StatusCode::INTERNAL_SERVER_ERROR)]
fn each_error_maps_to_its_status(#[case] err: BcError, #[case] expected: StatusCode) {
    assert_eq!(bc_server::status_for(&err), expected);
}
