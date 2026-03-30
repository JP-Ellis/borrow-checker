//! Integration tests for the `transaction` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn list_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn list_empty_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Parses an account ID string from a JSON output buffer.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn parse_account_id(stdout: &[u8]) -> String {
    let json: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON");
    json.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned()
}

/// Creates two accounts and returns their IDs.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn setup_accounts(ctx: &TestContext) -> (String, String) {
    let checking_out = ctx
        .command()
        .args([
            "--json", "account", "create", "--name", "Checking", "--type", "asset",
        ])
        .output()
        .expect("create checking");
    let checking_id = parse_account_id(&checking_out.stdout);

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
        .expect("create expenses");
    let expenses_id = parse_account_id(&expenses_out.stdout);

    (checking_id, expenses_id)
}

#[test]
fn add_transaction() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        &format!("{checking_id}:-50.00:AUD"),
        "--posting",
        &format!("{expenses_id}:50.00:AUD"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_transaction_json() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        &format!("{checking_id}:-50.00:AUD"),
        "--posting",
        &format!("{expenses_id}:50.00:AUD"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_unbalanced_transaction_fails() {
    let ctx = TestContext::new();
    let (checking_id, _) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Unbalanced",
        "--posting",
        &format!("{checking_id}:-50.00:AUD"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn void_existing_transaction() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);

    let void_out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "To void",
            "--posting",
            &format!("{checking_id}:-10.00:AUD"),
            "--posting",
            &format!("{expenses_id}:10.00:AUD"),
        ])
        .output()
        .expect("add");
    let void_json: serde_json::Value = serde_json::from_slice(&void_out.stdout).expect("json");
    let tx_id = void_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned();

    let mut cmd = ctx.command();
    cmd.args(["transaction", "void", &tx_id]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn void_nonexistent_transaction_returns_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["transaction", "void", "transaction_notavalidid000000000"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn amend_description() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);

    let amend_out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Original desc",
            "--posting",
            &format!("{checking_id}:-20.00:AUD"),
            "--posting",
            &format!("{expenses_id}:20.00:AUD"),
        ])
        .output()
        .expect("add");
    let amend_json: serde_json::Value = serde_json::from_slice(&amend_out.stdout).expect("json");
    let tx_id = amend_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned();

    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "amend",
        &tx_id,
        "--description",
        "Amended desc",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
