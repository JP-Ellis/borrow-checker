//! E2E: `borrow-checker backup`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

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
