//! Integration tests for the `account` subcommand.

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn list_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["account", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn list_empty_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "account", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_asset_account() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "create",
        "--name",
        "CommBank Savings",
        "--type",
        "asset",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_account_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "account",
        "create",
        "--name",
        "CommBank Savings",
        "--type",
        "asset",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_then_list() {
    let ctx = TestContext::new();

    ctx.command()
        .args(["account", "create", "--name", "Savings", "--type", "asset"])
        .output()
        .expect("create");

    let mut cmd = ctx.command();
    cmd.args(["account", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn archive_existing_account() {
    let ctx = TestContext::new();

    let output = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            "Old Account",
            "--type",
            "asset",
        ])
        .output()
        .expect("create");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let id = json["id"].as_str().expect("id field");

    let mut cmd = ctx.command();
    cmd.args(["account", "archive", id]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn archive_nonexistent_returns_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["account", "archive", "account_notavalidid0000000000"]);
    cmd_snapshot!(ctx, &mut cmd);
}
