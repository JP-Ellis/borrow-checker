//! `dispatch` routes every command name and rejects bad input cleanly.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use bc_ipc::BcError;
use bc_ipc::commands;
use bc_service::AppState;
use pretty_assertions::assert_eq;
use rstest::rstest;
use serde_json::Value;
use serde_json::json;
use tempfile::TempDir;

/// Settings that keep the database and backups inside `dir`.
fn settings_in(dir: &TempDir) -> bc_config::Settings {
    let mut settings = bc_config::Settings::default();
    settings.set_db_path(dir.path().join("ledger.db"));
    settings.set_backup_dir(dir.path().join("backups"));
    settings
}

/// Points config-file lookup into `dir`, so no command reads the developer's config.
fn isolate_config(dir: &TempDir) {
    // SAFETY: nextest runs each test in its own process, and this runs before
    // the test opens the database or spawns any thread that reads the
    // environment.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config")) }
}

/// Opens a fresh state on a database inside `dir`.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
async fn open_state(dir: &TempDir) -> AppState {
    isolate_config(dir);
    AppState::open(&settings_in(dir)).await.expect("open")
}

#[tokio::test]
async fn every_command_name_is_routed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_state(&dir).await;
    for &cmd in commands::ALL {
        // An empty object is valid for argument-free commands and malformed
        // for the rest; either way the name must be recognised.
        let result = bc_service::dispatch(&state, cmd, json!({})).await;
        assert!(
            !matches!(result, Err(BcError::NotFound(ref m)) if m.starts_with("unknown command")),
            "{cmd} is not routed"
        );
    }
}

#[rstest]
#[case::drop_everything("drop_everything")]
#[case::empty("")]
#[case::other_case("LIST_ACCOUNTS")]
#[case::trailing_space("list_accounts ")]
#[tokio::test]
async fn unknown_command_is_not_found(#[case] cmd: &str) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_state(&dir).await;
    let result = bc_service::dispatch(&state, cmd, json!({})).await;
    assert_eq!(
        result,
        Err(BcError::NotFound(format!("unknown command: {cmd}")))
    );
}

#[rstest]
#[case::null(commands::REVERSE_TRANSACTION, json!(null))]
#[case::array(commands::REVERSE_TRANSACTION, json!([]))]
#[case::wrong_field_type(commands::REVERSE_TRANSACTION, json!({ "id": 7_i32 }))]
#[case::missing_required_field(
    commands::GET_ACCOUNT_STATS,
    json!({
        "account_id": "acct-1",
        "commodity": null,
        "date_from": "2026-01-01",
        "date_until": "2026-02-01",
        "filter": null,
    })
)]
#[tokio::test]
async fn malformed_arguments_are_a_validation_error(#[case] cmd: &str, #[case] args: Value) {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_state(&dir).await;
    let result = bc_service::dispatch(&state, cmd, args).await;
    assert!(
        matches!(result, Err(BcError::Validation(ref m)) if m.starts_with("invalid arguments")),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_second_open_on_one_database_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let _first = open_state(&dir).await;

    let second = AppState::open(&settings_in(&dir)).await;

    let err = second.err();
    assert!(
        matches!(err, Some(BcError::Internal(ref m)) if m.contains("database is in use")),
        "{err:?}"
    );
}

#[tokio::test]
async fn a_settings_update_does_not_move_the_restore_root() {
    let dir = tempfile::tempdir().expect("tempdir");
    let state = open_state(&dir).await;
    let moved = dir.path().join("elsewhere").display().to_string();
    let settings = bc_ipc::BackupSettings::new(Some(moved), Some(3), None, true);

    let result = bc_service::dispatch(
        &state,
        commands::UPDATE_BACKUP_SETTINGS,
        json!({ "settings": settings }),
    )
    .await;

    assert_eq!(result, Ok(serde_json::Value::Null));
    assert_eq!(
        state.backup_dir().parent(),
        Some(dir.path().join("backups").as_path())
    );
}

#[tokio::test]
async fn startup_survives_an_uncreatable_backup_pool() {
    let dir = tempfile::tempdir().expect("tempdir");
    isolate_config(&dir);
    let blocker = dir.path().join("not-a-dir");
    std::fs::write(&blocker, b"").expect("blocker");
    let mut settings = settings_in(&dir);
    settings.set_backup_dir(blocker.join("backups"));

    let state = AppState::open(&settings).await;

    assert!(state.is_ok(), "{:?}", state.err());
}
