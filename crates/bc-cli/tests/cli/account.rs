//! Integration tests for the `account` subcommand.

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
        "Bank Savings",
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
        "Bank Savings",
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
    let id = json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field");

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

/// Creates an account of the given type and returns its ID (via `--json`).
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn create_account(ctx: &TestContext, name: &str, account_type: &str) -> String {
    let output = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            name,
            "--type",
            account_type,
        ])
        .output()
        .expect("create");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    json.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned()
}

/// Records a balancing two-posting transaction between two accounts so that
/// [`bc_core::BalanceEngine::default_balances`] reports non-zero balances.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn add_balancing_transaction(
    ctx: &TestContext,
    debit: &str,
    credit: &str,
    amount: &str,
    commodity: &str,
) {
    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2025-01-15",
            "--description",
            "Groceries",
            "--posting",
            &format!("{debit}:{amount}:{commodity}"),
            "--posting",
            &format!("{credit}:-{amount}:{commodity}"),
        ])
        .output()
        .expect("transaction add");
}

#[test]
fn balance_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["account", "balance"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_empty_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "account", "balance"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_lists_sorted_by_type_then_name() {
    let ctx = TestContext::new();
    // Create out of sort order to prove the sort: an expense before an asset,
    // and two assets whose names sort non-alphabetically at creation time.
    let expense = create_account(&ctx, "Groceries", "expense");
    let savings = create_account(&ctx, "Savings", "asset");
    let checking = create_account(&ctx, "Checking", "asset");
    add_balancing_transaction(&ctx, &savings, &expense, "100", "AUD");
    add_balancing_transaction(&ctx, &checking, &expense, "40", "AUD");

    let mut cmd = ctx.command();
    cmd.args(["account", "balance"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_json() {
    let ctx = TestContext::new();
    let asset = create_account(&ctx, "Savings", "asset");
    let expense = create_account(&ctx, "Groceries", "expense");
    add_balancing_transaction(&ctx, &asset, &expense, "100", "AUD");

    let mut cmd = ctx.command();
    cmd.args(["--json", "account", "balance"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_filter_by_account() {
    let ctx = TestContext::new();
    let asset = create_account(&ctx, "Savings", "asset");
    let expense = create_account(&ctx, "Groceries", "expense");
    add_balancing_transaction(&ctx, &asset, &expense, "100", "AUD");

    let mut cmd = ctx.command();
    cmd.args(["--json", "account", "balance", &asset]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_filter_by_commodity() {
    let ctx = TestContext::new();
    let asset = create_account(&ctx, "Savings", "asset");
    let expense = create_account(&ctx, "Groceries", "expense");
    add_balancing_transaction(&ctx, &asset, &expense, "100", "AUD");

    // A commodity nobody holds yields an empty result.
    let mut cmd = ctx.command();
    cmd.args(["--json", "account", "balance", "--commodity", "USD"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_filter_by_commodity_matches() {
    let ctx = TestContext::new();
    // Two assets holding different commodities, each with its own expense
    // counterparty so every account's default commodity is unambiguous.
    let savings = create_account(&ctx, "Savings", "asset");
    let wallet = create_account(&ctx, "Wallet", "asset");
    let groceries = create_account(&ctx, "Groceries", "expense");
    let travel = create_account(&ctx, "Travel", "expense");
    add_balancing_transaction(&ctx, &savings, &groceries, "100", "AUD");
    add_balancing_transaction(&ctx, &wallet, &travel, "50", "USD");

    // Only the AUD-denominated accounts should appear.
    let mut cmd = ctx.command();
    cmd.args(["--json", "account", "balance", "--commodity", "AUD"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn balance_invalid_account_id_returns_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["account", "balance", "not-a-valid-id"]);
    cmd_snapshot!(ctx, &mut cmd);
}
