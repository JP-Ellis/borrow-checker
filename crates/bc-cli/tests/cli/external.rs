//! E2E: unknown subcommands run `borrow-checker-<name>`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use pretty_assertions::assert_eq;

#[test]
fn server_subcommand_runs_the_server_binary() {
    let server = assert_cmd::cargo::cargo_bin("borrow-checker-server");
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
