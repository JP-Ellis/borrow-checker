//! E2E: `borrow-checker restore`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use assert_cmd::Command;

#[test]
fn restore_rejects_garbage_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let junk = dir.path().join("junk.sqlite");
    std::fs::write(&junk, b"not a database").expect("write junk");

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
            "restore",
            junk.to_str().expect("utf8"),
        ])
        .assert()
        .failure();

    // Live database must remain intact and queryable.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args(["--db-path", db.to_str().expect("utf8"), "account", "list"])
        .assert()
        .success();
}

#[test]
fn restore_replaces_database_from_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let snap = dir.path().join("snap.sqlite");

    // Seed + snapshot.
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
            snap.to_str().expect("utf8"),
        ])
        .assert()
        .success();

    // Restore should succeed on a valid snapshot.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args([
            "--db-path",
            db.to_str().expect("utf8"),
            "restore",
            snap.to_str().expect("utf8"),
        ])
        .assert()
        .success();
}
