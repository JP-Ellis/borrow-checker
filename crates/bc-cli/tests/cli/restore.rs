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

/// Restoring must swap in the *content* of the backup, not stale WAL frames.
///
/// The database is opened in WAL mode, so a leftover `{db}-wal`/`{db}-shm`
/// sidecar from the database being replaced would be replayed by SQLite
/// recovery on the next open. This test writes distinguishable data, backs it
/// up, mutates the live database, restores, then REOPENS and reads: it fails if
/// the sidecars are not cleared (the mutated row would resurrect, or the file
/// would be corrupt and unreadable).
#[test]
fn restore_reopens_with_backup_content_not_stale_wal() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let snap = dir.path().join("snap.sqlite");
    let db_arg = db.to_str().expect("utf8");

    // Original, distinguishable state: a single account.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args([
            "--db-path",
            db_arg,
            "account",
            "create",
            "--name",
            "AlphaMarker",
            "--type",
            "asset",
        ])
        .assert()
        .success();

    // Snapshot the original state to a standalone backup file.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args([
            "--db-path",
            db_arg,
            "backup",
            "--output",
            snap.to_str().expect("utf8"),
        ])
        .assert()
        .success();

    // Mutate the live database: add a second account absent from the backup.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args([
            "--db-path",
            db_arg,
            "account",
            "create",
            "--name",
            "BetaMutation",
            "--type",
            "asset",
        ])
        .assert()
        .success();

    // Restore from the pre-mutation snapshot.
    Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args(["--db-path", db_arg, "restore", snap.to_str().expect("utf8")])
        .assert()
        .success();

    // Reopen (fresh process) and read the accounts. The restored content must be
    // live: AlphaMarker present, BetaMutation gone. If stale WAL frames were
    // replayed, BetaMutation would resurrect or the DB would be unreadable.
    let out = Command::cargo_bin("borrow-checker")
        .expect("bin")
        .args(["--db-path", db_arg, "account", "list"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let listing = String::from_utf8(out).expect("utf8 stdout");

    assert!(
        listing.contains("AlphaMarker"),
        "restored account must be present after reopen; got:\n{listing}"
    );
    assert!(
        !listing.contains("BetaMutation"),
        "post-backup mutation must NOT resurrect via stale WAL; got:\n{listing}"
    );
}
