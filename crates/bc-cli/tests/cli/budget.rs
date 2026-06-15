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
    let acc_id = create_expense_account(&ctx);
    let _budget_id = create_budget(&ctx, &acc_id);

    let mut cmd = ctx.command();
    cmd.args(["budget", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Helper: create an expense account and return its ID string.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn create_expense_account(ctx: &TestContext) -> String {
    let out = ctx
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
        .expect("account create executed");
    assert!(
        out.status.success(),
        "account create should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    json.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("account id")
        .to_owned()
}

/// Helper: create a budget with a target and return its ID string.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn create_budget(ctx: &TestContext, acc_id: &str) -> String {
    let out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "create",
            "--account",
            acc_id,
            "--name",
            "Groceries Budget",
            "--target",
            "500",
            "--commodity",
            "AUD",
            "--period",
            "monthly",
            "--rollover",
            "reset-to-zero",
        ])
        .output()
        .expect("budget create executed");
    assert!(
        out.status.success(),
        "budget create should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    json.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("budget id")
        .to_owned()
}

#[test]
fn archive_budget() {
    let ctx = TestContext::new();
    let acc_id = create_expense_account(&ctx);
    let budget_id = create_budget(&ctx, &acc_id);

    let mut archive_cmd = ctx.command();
    archive_cmd.args(["budget", "archive", &budget_id]);
    cmd_snapshot!(ctx, &mut archive_cmd);

    let mut list_cmd = ctx.command();
    list_cmd.args(["budget", "list"]);
    cmd_snapshot!(ctx, &mut list_cmd);
}

#[test]
fn archive_nonexistent_budget() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["budget", "archive", "budget_00000000000000000000000000"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn allocate_to_budget() {
    let ctx = TestContext::new();
    let acc_id = create_expense_account(&ctx);
    let budget_id = create_budget(&ctx, &acc_id);

    let mut cmd = ctx.command();
    cmd.args([
        "budget",
        "allocate",
        "--budget",
        &budget_id,
        "--amount",
        "500",
        "--commodity",
        "AUD",
        "--period-start",
        "2030-01-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn reallocate_updates_amount() {
    let ctx = TestContext::new();
    let acc_id = create_expense_account(&ctx);
    let budget_id = create_budget(&ctx, &acc_id);

    let first_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "allocate",
            "--budget",
            &budget_id,
            "--amount",
            "500",
            "--commodity",
            "AUD",
            "--period-start",
            "2030-01-01",
        ])
        .output()
        .expect("first allocate executed");
    assert!(
        first_out.status.success(),
        "first allocate should succeed: {}",
        String::from_utf8_lossy(&first_out.stderr)
    );

    let mut second_cmd = ctx.command();
    second_cmd.args([
        "budget",
        "allocate",
        "--budget",
        &budget_id,
        "--amount",
        "300",
        "--commodity",
        "AUD",
        "--period-start",
        "2030-01-01",
    ]);
    cmd_snapshot!(ctx, &mut second_cmd);

    // Verify the allocation is overwritten (not accumulated).
    let third_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "allocate",
            "--budget",
            &budget_id,
            "--amount",
            "200",
            "--commodity",
            "AUD",
            "--period-start",
            "2030-01-01",
        ])
        .output()
        .expect("third allocate executed");
    assert!(
        third_out.status.success(),
        "third allocate should succeed: {}",
        String::from_utf8_lossy(&third_out.stderr)
    );
    let alloc_json: serde_json::Value =
        serde_json::from_slice(&third_out.stdout).expect("valid JSON");
    let amount_value = alloc_json
        .get("amount")
        .and_then(|a| a.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("amount.value");
    assert_eq!(amount_value, "200");
}

#[test]
fn update_budget_name() {
    let ctx = TestContext::new();
    let acc_id = create_expense_account(&ctx);
    let budget_id = create_budget(&ctx, &acc_id);

    let mut cmd = ctx.command();
    cmd.args(["budget", "update", "--id", &budget_id, "--name", "New Name"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_budget_rollover_and_target() {
    let ctx = TestContext::new();
    let acc_id = create_expense_account(&ctx);
    let budget_id = create_budget(&ctx, &acc_id);

    let mut cmd = ctx.command();
    cmd.args([
        "budget",
        "update",
        "--id",
        &budget_id,
        "--target",
        "200",
        "--commodity",
        "AUD",
        "--rollover",
        "carry-forward",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn budget_status_with_allocation() {
    let ctx = TestContext::new();
    let acc_id = create_expense_account(&ctx);
    let budget_id = create_budget(&ctx, &acc_id);

    let alloc_out = ctx
        .command()
        .args([
            "--json",
            "budget",
            "allocate",
            "--budget",
            &budget_id,
            "--amount",
            "500",
            "--commodity",
            "AUD",
            "--period-start",
            "2030-01-01",
        ])
        .output()
        .expect("allocate executed");
    assert!(
        alloc_out.status.success(),
        "allocate should succeed: {}",
        String::from_utf8_lossy(&alloc_out.stderr)
    );

    let mut cmd = ctx.command();
    cmd.args(["budget", "status", "--as-of", "2030-01-15"]);
    cmd_snapshot!(ctx, &mut cmd);
}
