//! E2E: `borrow-checker backup`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::path::Path;

use assert_cmd::Command;
use pretty_assertions::assert_eq;

use crate::common::TestContext;

#[test]
fn backup_writes_file_to_output_path() {
    let ctx = TestContext::new();
    let dir = &ctx.home_dir;
    let db = dir.path().join("db.sqlite");
    let out = dir.path().join("snapshot.sqlite");

    // Seed the DB by running any command that opens it.
    ctx.bare_command()
        .args(["--db-path", db.to_str().expect("utf8"), "account", "list"])
        .assert()
        .success();

    ctx.bare_command()
        .args([
            "--db-path",
            db.to_str().expect("utf8"),
            "backup",
            "--output",
            out.to_str().expect("utf8"),
        ])
        .assert()
        .success();

    assert!(out.exists(), "backup --output should create the file");
}

#[test]
fn backup_json_emits_full_record() {
    let ctx = TestContext::new();
    let dir = &ctx.home_dir;
    let db = dir.path().join("db.sqlite");
    let out = dir.path().join("snapshot.sqlite");

    ctx.bare_command()
        .args(["--db-path", db.to_str().expect("utf8"), "account", "list"])
        .assert()
        .success();

    let assert = ctx
        .bare_command()
        .args([
            "--json",
            "--db-path",
            db.to_str().expect("utf8"),
            "backup",
            "--output",
            out.to_str().expect("utf8"),
        ])
        .assert()
        .success();

    assert!(out.exists(), "backup --output should create the file");

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).expect("utf8 stdout");
    let json: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");

    assert_eq!(json.get("kind"), Some(&serde_json::Value::from("manual")));
    assert!(
        json.get("created_at")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|s| !s.is_empty())
    );
    assert!(
        json.get("size_bytes")
            .and_then(serde_json::Value::as_u64)
            .expect("size_bytes is u64")
            > 0,
        "size_bytes should be greater than zero"
    );
}

/// A command against `db` with managed backups isolated under `bk`.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn bc(ctx: &TestContext, db: &Path, bk: &Path) -> Command {
    let mut cmd = ctx.bare_command();
    cmd.env("BC_BACKUP__DIR", bk)
        .args(["--db-path", db.to_str().expect("utf8")]);
    cmd
}

/// Runs `backup list --json` and returns the parsed array.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn list(ctx: &TestContext, db: &Path, bk: &Path) -> Vec<serde_json::Value> {
    let out = bc(ctx, db, bk)
        .args(["--json", "backup", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("json array")
}

#[test]
fn list_shows_only_this_ledgers_backups() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let a = ctx.home_dir.path().join("a.sqlite");
    let b = ctx.home_dir.path().join("b.sqlite");
    bc(&ctx, &a, &bk).arg("backup").assert().success();
    bc(&ctx, &b, &bk).arg("backup").assert().success();
    bc(&ctx, &b, &bk).arg("backup").assert().success();

    assert_eq!(list(&ctx, &a, &bk).len(), 1);
    assert_eq!(list(&ctx, &b, &bk).len(), 2);
}

#[test]
fn list_json_shape() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");
    bc(&ctx, &db, &bk).arg("backup").assert().success();

    let rows = list(&ctx, &db, &bk);
    let row = rows.first().expect("one row");
    let keys: Vec<&str> = row
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();

    insta::assert_debug_snapshot!(keys);
    assert_eq!(row.get("kind"), Some(&serde_json::Value::from("manual")));
}

#[test]
fn delete_removes_a_listed_backup() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");
    bc(&ctx, &db, &bk).arg("backup").assert().success();
    let name = list(&ctx, &db, &bk)
        .first()
        .and_then(|r| r.get("file_name"))
        .and_then(serde_json::Value::as_str)
        .expect("file_name")
        .to_owned();

    bc(&ctx, &db, &bk)
        .args(["backup", "delete", &name])
        .assert()
        .success();

    assert_eq!(list(&ctx, &db, &bk).len(), 0);
}

#[test]
fn delete_refuses_a_traversal_name() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");

    let out = bc(&ctx, &db, &bk)
        .args(["backup", "delete", "../20260101-000000000.manual.sqlite"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&out);
    assert!(stderr.contains("not a backup file name"), "{stderr}");
}

#[test]
fn bare_backup_table_prints_the_pool_path() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");

    let out = bc(&ctx, &db, &bk)
        .arg("backup")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&out);

    assert!(stdout.contains("Path"), "{stdout}");
    assert!(stdout.contains(bk.to_str().expect("utf8")), "{stdout}");
}

#[test]
fn rekey_starts_a_new_pool() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");
    bc(&ctx, &db, &bk).arg("backup").assert().success();

    let out = bc(&ctx, &db, &bk)
        .args(["--json", "backup", "rekey"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let ids: serde_json::Value = serde_json::from_slice(&out).expect("json");

    pretty_assertions::assert_ne!(ids.get("old"), ids.get("new"));
    assert!(list(&ctx, &db, &bk).is_empty(), "the new pool starts empty");
    let old = ids
        .get("old")
        .and_then(serde_json::Value::as_str)
        .expect("old");
    assert!(bk.join(old).is_dir(), "the old pool stays on disk");
}

#[test]
fn rekey_refuses_while_the_database_is_locked() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");
    bc(&ctx, &db, &bk)
        .args(["account", "list"])
        .assert()
        .success();
    let _held = bc_core::DbLock::acquire(&db).expect("hold lock");

    let out = bc(&ctx, &db, &bk)
        .args(["backup", "rekey"])
        .assert()
        .failure()
        .get_output()
        .stderr
        .clone();
    let stderr = String::from_utf8_lossy(&out);
    assert!(stderr.contains("in use"), "{stderr}");
}

#[test]
fn restoring_an_own_backup_keeps_the_pool() {
    let ctx = TestContext::new();
    let bk = ctx.home_dir.path().join("backups");
    let db = ctx.home_dir.path().join("db.sqlite");
    bc(&ctx, &db, &bk).arg("backup").assert().success();
    let first = list(&ctx, &db, &bk)
        .first()
        .and_then(|r| r.get("path"))
        .and_then(serde_json::Value::as_str)
        .expect("path")
        .to_owned();

    bc(&ctx, &db, &bk)
        .args(["restore", &first])
        .assert()
        .success();

    let kinds: Vec<String> = list(&ctx, &db, &bk)
        .iter()
        .filter_map(|r| {
            r.get("kind")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    assert!(kinds.contains(&"manual".to_owned()), "{kinds:?}");
    assert!(kinds.contains(&"pre-restore".to_owned()), "{kinds:?}");
}
