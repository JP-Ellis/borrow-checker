//! Integration tests for the `export` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn export_ledger_empty_db() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["export", "--format", "ledger"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_beancount_empty_db() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["export", "--format", "beancount"]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Seeds a context with one account + one transaction and returns account ids.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn seed_account_and_transaction(ctx: &TestContext) -> (String, String) {
    let parse_id = |stdout: &[u8]| -> String {
        let json: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON");
        json.get("id")
            .and_then(serde_json::Value::as_str)
            .expect("id field")
            .to_owned()
    };

    let checking_out = ctx
        .command()
        .args([
            "--json", "account", "create", "--name", "Checking", "--type", "asset",
        ])
        .output()
        .expect("create checking");
    let checking_id = parse_id(&checking_out.stdout);

    let expenses_out = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            "Groceries",
            "--type",
            "expense",
        ])
        .output()
        .expect("create groceries");
    let expenses_id = parse_id(&expenses_out.stdout);

    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Weekly groceries",
            "--posting",
            &format!("{checking_id}:-80.00:AUD"),
            "--posting",
            &format!("{expenses_id}:80.00:AUD"),
        ])
        .output()
        .expect("add transaction");

    (checking_id, expenses_id)
}

#[test]
fn export_ledger_with_data() {
    let ctx = TestContext::new();
    seed_account_and_transaction(&ctx);
    let mut cmd = ctx.command();
    cmd.args(["export", "--format", "ledger"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_beancount_with_data() {
    let ctx = TestContext::new();
    seed_account_and_transaction(&ctx);
    let mut cmd = ctx.command();
    cmd.args(["export", "--format", "beancount"]);
    cmd_snapshot!(ctx, &mut cmd);
}
