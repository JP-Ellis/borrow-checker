//! E2E: `borrow-checker backup`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use assert_cmd::Command;
use pretty_assertions::assert_eq;

#[test]
fn backup_writes_file_to_output_path() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let out = dir.path().join("snapshot.sqlite");

    // Seed the DB by running any command that opens it.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args(["--db-path", db.to_str().expect("utf8"), "account", "list"])
        .assert()
        .success();

    Command::cargo_bin("borrow-checker")
        .expect("bin")
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
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let out = dir.path().join("snapshot.sqlite");

    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args(["--db-path", db.to_str().expect("utf8"), "account", "list"])
        .assert()
        .success();

    let assert = Command::cargo_bin("borrow-checker")
        .expect("bin")
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
