//! Integration tests for the `profile` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]
#![expect(clippy::expect_used, reason = "tests panic on setup failure")]

use crate::cmd_snapshot;
use crate::common::TestContext;

/// A minimal, obviously-fake CSV importer config used across these tests.
const BANK_CONFIG: &str = "account = \"Assets:Bank:Checking\"\n\
                           source_dir = \"Assets/Bank/Checking\"\n\
                           date_column = \"Date\"\n";

/// Creates an isolated context that already contains a `bank` profile.
///
/// The seeding command's own output is asserted but not snapshotted, so that
/// each test snapshots exactly one command.
fn ctx_with_bank_profile() -> TestContext {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.toml");
    std::fs::write(&cfg, BANK_CONFIG).expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    let output = ctx.run(&mut cmd);
    assert!(
        output.contains("success: true"),
        "seeding profile create failed:\n{output}"
    );

    ctx
}

#[test]
fn profile_create_succeeds() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.toml");
    std::fs::write(&cfg, BANK_CONFIG).expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_accepts_json_config() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.json");
    std::fs::write(
        &cfg,
        r#"{"account": "Assets:Bank:Checking", "source_dir": "Assets/Bank/Checking"}"#,
    )
    .expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_a_duplicate_name() {
    let ctx = ctx_with_bank_profile();
    let cfg = ctx.home_dir.path().join("bank.toml");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "ofx",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_a_malformed_config() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("broken.toml");
    std::fs::write(&cfg, "this is not = = toml\n").expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_a_non_table_config() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("array.json");
    std::fs::write(&cfg, "[1, 2, 3]\n").expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_json_output() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.toml");
    std::fs::write(&cfg, BANK_CONFIG).expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg)
    .arg("--json");
    cmd_snapshot!(ctx, &mut cmd);
}
