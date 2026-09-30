//! E2E: unknown subcommands run `borrow-checker-<name>`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::env::consts::EXE_SUFFIX;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use pretty_assertions::assert_eq;

#[test]
fn server_subcommand_runs_the_server_binary() {
    // `cargo_bin` panics on a missing binary, so derive the sibling path.
    let server = assert_cmd::cargo::cargo_bin("borrow-checker")
        .with_file_name(format!("borrow-checker-server{EXE_SUFFIX}"));
    if !server.exists() {
        // `cargo nextest run -p bc-cli` does not build other packages' bins.
        eprintln!("skipping: borrow-checker-server not built");
        return;
    }
    let dir = assert_fs::TempDir::new().expect("tempdir");
    std::fs::copy(&server, dir.path().join("borrow-checker-server")).expect("copy");
    std::fs::copy(
        assert_cmd::cargo::cargo_bin("borrow-checker"),
        dir.path().join("borrow-checker"),
    )
    .expect("copy");

    let o = std::process::Command::new(dir.path().join("borrow-checker"))
        .args(["server", "--help"])
        .output()
        .expect("run");
    assert!(
        o.status.success(),
        "status: {}, stderr: {}",
        o.status,
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(String::from_utf8_lossy(&o.stdout).contains("borrow-checker-server"));
}

#[cfg(unix)]
#[test]
fn db_path_reaches_the_child_as_bc_db_path() {
    let dir = assert_fs::TempDir::new().expect("tempdir");
    let stub = dir.path().join("borrow-checker-envdump");
    std::fs::write(&stub, "#!/bin/sh\nprintf '%s' \"$BC_DB__PATH\"\n").expect("write");
    std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    std::fs::copy(
        assert_cmd::cargo::cargo_bin("borrow-checker"),
        dir.path().join("borrow-checker"),
    )
    .expect("copy");

    let o = std::process::Command::new(dir.path().join("borrow-checker"))
        .env_remove("BC_DB__PATH")
        .args(["--db-path", "/ledger/test.db", "envdump"])
        .output()
        .expect("run");

    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&o.stdout), "/ledger/test.db");
}

#[test]
fn an_unknown_subcommand_keeps_the_usage_error() {
    let out = assert_cmd::Command::cargo_bin("borrow-checker")
        .expect("bin")
        .arg("frobnicate")
        .output()
        .expect("run");
    assert_eq!(out.status.code(), Some(2_i32));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("unrecognized subcommand 'frobnicate'"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[cfg(unix)]
#[test]
fn help_lists_external_subcommands_on_path() {
    let bin = assert_fs::TempDir::new().expect("tempdir");
    let demo = bin.path().join("borrow-checker-demo");
    std::fs::write(&demo, b"#!/bin/sh\n").expect("write");
    std::fs::set_permissions(&demo, std::fs::Permissions::from_mode(0o755)).expect("chmod");

    let out = assert_cmd::Command::cargo_bin("borrow-checker")
        .expect("bin")
        .env("PATH", bin.path())
        .arg("--help")
        .output()
        .expect("run");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("External subcommands:\n"), "{stdout}");
    assert!(stdout.contains("\n  demo\n"), "{stdout}");
}
