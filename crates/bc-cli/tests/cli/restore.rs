//! E2E: `borrow-checker restore`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::path::Path;

use assert_cmd::Command;

/// Builds a `borrow-checker` command bound to `db_arg` with an isolated managed
/// backup directory.
///
/// `restore` writes an automatic safety snapshot to the *managed* backup
/// directory, which defaults to a shared per-user platform path. Under the
/// parallel test runner two tests would then write same-second snapshot
/// filenames into that one directory and race on temp-file/rotation deletion
/// (surfacing as `SQLITE_IOERR_DELETE_NOENT`), besides polluting the real user
/// data directory. Pointing `BC_BACKUP_DIR` at a per-test temp directory keeps
/// each test's managed backups isolated.
#[expect(
    clippy::expect_used,
    reason = "test helper — a missing test binary should fail the test loudly"
)]
fn bc(db_arg: &str, backup_dir: &Path) -> Command {
    let mut cmd = Command::cargo_bin("borrow-checker").expect("bin");
    cmd.env("BC_BACKUP_DIR", backup_dir);
    cmd.args(["--db-path", db_arg]);
    cmd
}

#[test]
fn restore_rejects_garbage_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let db_arg = db.to_str().expect("utf8");
    let bk = dir.path().join("backups");
    let junk = dir.path().join("junk.sqlite");
    std::fs::write(&junk, b"not a database").expect("write junk");

    bc(db_arg, &bk).args(["account", "list"]).assert().success();

    bc(db_arg, &bk)
        .args(["restore", junk.to_str().expect("utf8")])
        .assert()
        .failure();

    // Live database must remain intact and queryable.
    bc(db_arg, &bk).args(["account", "list"]).assert().success();
}

#[test]
fn restore_replaces_database_from_backup() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db = dir.path().join("db.sqlite");
    let db_arg = db.to_str().expect("utf8");
    let bk = dir.path().join("backups");
    let snap = dir.path().join("snap.sqlite");

    // Seed + snapshot.
    bc(db_arg, &bk).args(["account", "list"]).assert().success();
    bc(db_arg, &bk)
        .args(["backup", "--output", snap.to_str().expect("utf8")])
        .assert()
        .success();

    // Restore should succeed on a valid snapshot.
    bc(db_arg, &bk)
        .args(["restore", snap.to_str().expect("utf8")])
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
    let db_arg = db.to_str().expect("utf8");
    let bk = dir.path().join("backups");
    let snap = dir.path().join("snap.sqlite");

    // Original, distinguishable state: a single account.
    bc(db_arg, &bk)
        .args(["account", "create", "AlphaMarker", "--type", "asset"])
        .assert()
        .success();

    // Snapshot the original state to a standalone backup file.
    bc(db_arg, &bk)
        .args(["backup", "--output", snap.to_str().expect("utf8")])
        .assert()
        .success();

    // Mutate the live database: add a second account absent from the backup.
    bc(db_arg, &bk)
        .args(["account", "create", "BetaMutation", "--type", "asset"])
        .assert()
        .success();

    // Restore from the pre-mutation snapshot.
    bc(db_arg, &bk)
        .args(["restore", snap.to_str().expect("utf8")])
        .assert()
        .success();

    // Reopen (fresh process) and read the accounts. The restored content must be
    // live: AlphaMarker present, BetaMutation gone. If stale WAL frames were
    // replayed, BetaMutation would resurrect or the DB would be unreadable.
    let out = bc(db_arg, &bk)
        .args(["account", "list"])
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
