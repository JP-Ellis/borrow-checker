//! Integration tests for the `budget` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use pretty_assertions::assert_eq;

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn list_budgets_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["budget", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_budget_monthly_and_list() {
    let ctx = TestContext::new();

    // Create an expense account to anchor the budget to.
    let acc_out = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            "Groceries",
            "--type",
            "expense",
            "--kind",
            "deposit-account",
        ])
        .output()
        .expect("command executed");
    assert!(
        acc_out.status.success(),
        "account create should succeed: {}",
        String::from_utf8_lossy(&acc_out.stderr)
    );
    let acc_json: serde_json::Value = serde_json::from_slice(&acc_out.stdout).expect("valid JSON");
    let acc_id = acc_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("account id");

    // Create a budget anchored to that account.
    let budget_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "create",
            "--account",
            acc_id,
            "--name",
            "Groceries Budget",
            "--period",
            "monthly",
            "--rollover",
            "reset-to-zero",
        ])
        .output()
        .expect("command executed");
    assert!(
        budget_out.status.success(),
        "budget create should succeed: {}",
        String::from_utf8_lossy(&budget_out.stderr)
    );
    let budget_json: serde_json::Value =
        serde_json::from_slice(&budget_out.stdout).expect("valid JSON");
    assert_eq!(
        budget_json.get("name").and_then(serde_json::Value::as_str),
        Some("Groceries Budget"),
        "name should be persisted"
    );
}
