//! Integration tests for the account lifecycle sub-commands: `close`,
//! `reopen`, `set-opened-on`, `archive --cascade`, and the `--opened-on`
//! flag on `create` (including its reuse-conflict handling when `create` is
//! run again against an existing path).

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

/// Parses an account ID string from a JSON output buffer.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn parse_account_id(stdout: &[u8]) -> String {
    let json: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON");
    json.get("account")
        .and_then(|account| account.get("id"))
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned()
}

/// Creates an account at `path` under `--type asset` and returns its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn create_asset(ctx: &TestContext, path: &str) -> String {
    let out = ctx
        .command()
        .args(["--json", "account", "create", path, "--type", "asset"])
        .output()
        .expect("create account");
    parse_account_id(&out.stdout)
}

#[test]
fn create_with_opened_on() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "create",
        "Assets:BankA:Checking",
        "--type",
        "asset",
        "--opened-on",
        "2024-01-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_with_opened_on_shows_in_list() {
    let ctx = TestContext::new();
    ctx.command()
        .args([
            "account",
            "create",
            "Assets:BankA:Checking",
            "--type",
            "asset",
            "--opened-on",
            "2024-01-01",
        ])
        .output()
        .expect("create");

    let mut cmd = ctx.command();
    cmd.args(["account", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn recreate_with_same_opened_on_is_a_noop() {
    let ctx = TestContext::new();
    ctx.command()
        .args([
            "account",
            "create",
            "Assets:BankA:Checking",
            "--type",
            "asset",
            "--opened-on",
            "2024-01-01",
        ])
        .output()
        .expect("create");

    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "create",
        "Assets:BankA:Checking",
        "--type",
        "asset",
        "--opened-on",
        "2024-01-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn recreate_with_different_opened_on_is_rejected() {
    let ctx = TestContext::new();
    ctx.command()
        .args([
            "account",
            "create",
            "Assets:BankA:Checking",
            "--type",
            "asset",
            "--opened-on",
            "2024-01-01",
        ])
        .output()
        .expect("create");

    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "create",
        "Assets:BankA:Checking",
        "--type",
        "asset",
        "--opened-on",
        "2024-02-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn recreate_with_opened_on_when_none_stored_is_rejected() {
    let ctx = TestContext::new();
    create_asset(&ctx, "Assets:BankA:Checking");

    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "create",
        "Assets:BankA:Checking",
        "--type",
        "asset",
        "--opened-on",
        "2024-01-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn close_rejects_a_parent_with_an_open_child() {
    let ctx = TestContext::new();
    create_asset(&ctx, "Assets:BankA:Checking");
    let bank_a = create_asset(&ctx, "Assets:BankA");

    let mut cmd = ctx.command();
    cmd.args(["account", "close", &bank_a, "--on", "2024-06-30"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn close_with_cascade_closes_the_subtree() {
    let ctx = TestContext::new();
    create_asset(&ctx, "Assets:BankA:Checking");
    let bank_a = create_asset(&ctx, "Assets:BankA");

    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "close",
        &bank_a,
        "--on",
        "2024-06-30",
        "--cascade",
    ]);
    cmd_snapshot!(ctx, &mut cmd);

    let mut list_cmd = ctx.command();
    list_cmd.args(["account", "list"]);
    cmd_snapshot!(ctx, &mut list_cmd);
}

#[test]
fn close_nonexistent_returns_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "close",
        "account_notavalidid0000000000",
        "--on",
        "2024-06-30",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn reopen_a_closed_account() {
    let ctx = TestContext::new();
    let account_id = create_asset(&ctx, "Assets:BankA:Checking");
    ctx.command()
        .args(["account", "close", &account_id, "--on", "2024-06-30"])
        .output()
        .expect("close");

    let mut cmd = ctx.command();
    cmd.args(["account", "reopen", &account_id]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn reopen_rejects_when_parent_is_closed() {
    let ctx = TestContext::new();
    let child = create_asset(&ctx, "Assets:BankA:Checking");
    let bank_a = create_asset(&ctx, "Assets:BankA");
    ctx.command()
        .args([
            "account",
            "close",
            &bank_a,
            "--on",
            "2024-06-30",
            "--cascade",
        ])
        .output()
        .expect("close with cascade");

    let mut cmd = ctx.command();
    cmd.args(["account", "reopen", &child]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn set_opened_on_updates_the_account() {
    let ctx = TestContext::new();
    let account_id = create_asset(&ctx, "Assets:BankA:Checking");

    let mut cmd = ctx.command();
    cmd.args([
        "account",
        "set-opened-on",
        &account_id,
        "--on",
        "2024-01-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn set_opened_on_clears_when_omitted() {
    let ctx = TestContext::new();
    let account_id = create_asset(&ctx, "Assets:BankA:Checking");
    ctx.command()
        .args([
            "account",
            "set-opened-on",
            &account_id,
            "--on",
            "2024-01-01",
        ])
        .output()
        .expect("set opened_on");

    let mut cmd = ctx.command();
    cmd.args(["account", "set-opened-on", &account_id]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn archive_rejects_a_parent_with_an_active_child() {
    let ctx = TestContext::new();
    create_asset(&ctx, "Assets:BankA:Checking");
    let bank_a = create_asset(&ctx, "Assets:BankA");

    let mut cmd = ctx.command();
    cmd.args(["account", "archive", &bank_a]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn archive_with_cascade_archives_the_subtree() {
    let ctx = TestContext::new();
    create_asset(&ctx, "Assets:BankA:Checking");
    let bank_a = create_asset(&ctx, "Assets:BankA");

    let mut cmd = ctx.command();
    cmd.args(["account", "archive", &bank_a, "--cascade"]);
    cmd_snapshot!(ctx, &mut cmd);

    let mut list_cmd = ctx.command();
    list_cmd.args(["account", "list"]);
    cmd_snapshot!(ctx, &mut list_cmd);
}
