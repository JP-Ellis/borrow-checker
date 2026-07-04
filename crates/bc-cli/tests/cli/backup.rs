//! E2E: `borrow-checker backup`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use assert_cmd::Command;

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
