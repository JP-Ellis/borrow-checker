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

/// A hostname the test servers list in `allowed-hosts`.
const ALLOWED_HOST: &str = "ledger.example.ts.net";

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
    let (shared, restart) = bc_server::Shared::new(app, vec![ALLOWED_HOST.to_owned()]);
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

/// POSTs `body` to `/rpc/{cmd}` as JSON and returns the status and body text.
async fn post(base: &str, cmd: &str, body: &str) -> (u16, String) {
    post_with(base, cmd, body, &[("content-type", "application/json")]).await
}

/// POSTs `body` to `/rpc/{cmd}` with exactly `headers` added, and returns the
/// status and body text.
async fn post_with(base: &str, cmd: &str, body: &str, headers: &[(&str, &str)]) -> (u16, String) {
    let mut req = reqwest::Client::new()
        .post(format!("{base}/rpc/{cmd}"))
        .body(body.to_owned());
    for &(name, value) in headers {
        req = req.header(name, value);
    }
    let r = req.send().await.expect("send");
    (r.status().as_u16(), r.text().await.expect("text"))
}

/// GETs `/` with `host` as the `Host` header and returns the status.
async fn get_with_host(base: &str, host: &str) -> u16 {
    reqwest::Client::new()
        .get(format!("{base}/"))
        .header("host", host)
        .send()
        .await
        .expect("send")
        .status()
        .as_u16()
}

/// The `host:port` the server listens on.
fn authority(base: &str) -> &str {
    base.strip_prefix("http://").expect("http base")
}

/// The body of an `update_backup_settings` call that keeps one backup.
fn retain_one() -> String {
    json!({
        "settings": {
            "dir": null,
            "retain_count": 1,
            "retain_days": null,
            "auto_pre_migration": true,
        }
    })
    .to_string()
}

/// The backup settings' `retain_count` as the server reports it.
async fn retain_count(base: &str) -> Value {
    let (status, body) = post(base, "get_backup_settings", "{}").await;
    assert_eq!(status, 200, "{body}");
    let settings: Value = serde_json::from_str(&body).expect("json");
    settings.get("retain_count").cloned().expect("retain_count")
}

/// Asserts `status` is the app shell's: 200 with a built frontend, 503
/// without one.
fn assert_shell(status: u16) {
    assert!(matches!(status, 200 | 503), "{status}");
}

/// Asserts `body` decodes as a `BcError::Validation`.
fn assert_validation_error(body: &str) {
    let err: BcError = serde_json::from_str(body).expect("a BcError body");
    assert!(matches!(err, BcError::Validation(_)), "{err:?}");
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
    assert!(
        body.contains("restart the server to restore from a new backup directory"),
        "{body}"
    );
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
#[case::text_plain("text/plain")]
#[case::form("application/x-www-form-urlencoded")]
#[case::multipart("multipart/form-data; boundary=x")]
#[tokio::test]
async fn a_non_json_rpc_is_415_and_does_not_run(#[case] content_type: &str) {
    let s = start().await;
    let before = retain_count(&s.base).await;

    let (status, body) = post_with(
        &s.base,
        "update_backup_settings",
        &retain_one(),
        &[("content-type", content_type)],
    )
    .await;

    assert_eq!(status, 415, "{body}");
    assert_validation_error(&body);
    assert_eq!(retain_count(&s.base).await, before);
}

#[tokio::test]
async fn an_rpc_without_a_content_type_is_415() {
    let s = start().await;
    let (status, _) = post_with(&s.base, "list_tags", "{}", &[]).await;
    assert_eq!(status, 415);
}

#[tokio::test]
async fn a_json_content_type_with_a_charset_is_accepted() {
    let s = start().await;
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[("content-type", "Application/JSON; charset=utf-8")],
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[rstest]
#[case::other_site("http://evil.example")]
#[case::other_port("http://127.0.0.1:1")]
#[case::opaque("null")]
#[tokio::test]
async fn a_foreign_origin_is_403_and_does_not_run(#[case] origin: &str) {
    let s = start().await;
    let before = retain_count(&s.base).await;

    let (status, body) = post_with(
        &s.base,
        "update_backup_settings",
        &retain_one(),
        &[("content-type", "application/json"), ("origin", origin)],
    )
    .await;

    assert_eq!(status, 403, "{body}");
    assert_validation_error(&body);
    assert_eq!(retain_count(&s.base).await, before);
}

#[tokio::test]
async fn a_cross_site_fetch_is_403() {
    let s = start().await;
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[
            ("content-type", "application/json"),
            ("sec-fetch-site", "cross-site"),
        ],
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_validation_error(&body);
}

#[tokio::test]
async fn a_same_origin_json_rpc_is_200() {
    let s = start().await;
    let origin = s.base.clone();

    let (status, body) = post_with(
        &s.base,
        "update_backup_settings",
        &retain_one(),
        &[
            ("content-type", "application/json"),
            ("origin", &origin),
            ("sec-fetch-site", "same-origin"),
        ],
    )
    .await;

    assert_eq!(status, 200, "{body}");
    assert_eq!(retain_count(&s.base).await, json!(1_u32));
}

#[tokio::test]
async fn an_origin_matching_the_forwarded_host_is_200() {
    // `trunk serve --proxy-backend` sends this shape: `Host` without the
    // dev server's port, and the browser's host in `X-Forwarded-Host`.
    let s = start().await;
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[
            ("content-type", "application/json"),
            ("host", "127.0.0.1"),
            ("x-forwarded-host", "127.0.0.1:1421"),
            ("origin", "http://127.0.0.1:1421"),
            ("sec-fetch-site", "same-origin"),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[tokio::test]
async fn a_rebound_name_behind_the_dev_proxy_is_403() {
    // A rebinding page on `trunk serve`'s port: the proxy puts its own
    // backend in `Host`, and the page's name in `X-Forwarded-Host`.
    let s = start().await;
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[
            ("content-type", "application/json"),
            ("host", "127.0.0.1:7171"),
            ("x-forwarded-host", "evil.example:1421"),
            ("origin", "http://evil.example:1421"),
            ("sec-fetch-site", "same-origin"),
        ],
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_validation_error(&body);
}

#[tokio::test]
async fn an_allowed_name_behind_tailscale_serve_is_200() {
    // Tailscale Serve terminates TLS on 443 and keeps the browser's `Host`,
    // which names no port.
    let s = start().await;
    let origin = format!("https://{ALLOWED_HOST}");
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[
            ("content-type", "application/json"),
            ("host", ALLOWED_HOST),
            ("x-forwarded-host", ALLOWED_HOST),
            ("origin", &origin),
            ("sec-fetch-site", "same-origin"),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[rstest]
#[case::rebound_name("evil.example")]
#[case::rebound_name_with_port("evil.example:7171")]
#[case::ip_prefixed_name("127.0.0.1.evil.example")]
#[tokio::test]
async fn an_unknown_host_is_403(#[case] host: &str) {
    let s = start().await;
    assert_eq!(get_with_host(&s.base, host).await, 403);
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[("content-type", "application/json"), ("host", host)],
    )
    .await;
    assert_eq!(status, 403, "{body}");
    assert_validation_error(&body);
}

#[rstest]
#[case::localhost("localhost:7171")]
#[case::v6("[::1]:7171")]
#[case::allowed(ALLOWED_HOST)]
#[case::allowed_with_port_and_case("Ledger.Example.TS.net:7171")]
#[tokio::test]
async fn an_ip_localhost_or_allowed_host_is_served(#[case] host: &str) {
    let s = start().await;
    assert_shell(get_with_host(&s.base, host).await);
}

#[tokio::test]
async fn the_listening_address_is_served() {
    let s = start().await;
    assert_shell(get_with_host(&s.base, authority(&s.base)).await);
    let (status, body) = post_with(
        &s.base,
        "list_tags",
        "{}",
        &[
            ("content-type", "application/json"),
            ("host", authority(&s.base)),
        ],
    )
    .await;
    assert_eq!(status, 200, "{body}");
}

#[rstest]
#[case::not_found(BcError::NotFound("x".to_owned()), StatusCode::NOT_FOUND)]
#[case::validation(BcError::Validation("x".to_owned()), StatusCode::UNPROCESSABLE_ENTITY)]
#[case::conflict(BcError::Conflict("x".to_owned()), StatusCode::CONFLICT)]
#[case::internal(BcError::Internal("x".to_owned()), StatusCode::INTERNAL_SERVER_ERROR)]
fn each_error_maps_to_its_status(#[case] err: BcError, #[case] expected: StatusCode) {
    assert_eq!(bc_server::status_for(&err), expected);
}
