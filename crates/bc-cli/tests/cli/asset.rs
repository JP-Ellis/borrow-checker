//! Integration tests for the `asset` subcommand.

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
    json.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned()
}

/// Creates a `ManualAsset` account and returns its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn create_manual_asset(ctx: &TestContext, name: &str) -> String {
    let out = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            name,
            "--type",
            "asset",
            "--kind",
            "manual-asset",
        ])
        .output()
        .expect("create ManualAsset");
    parse_account_id(&out.stdout)
}

/// Creates a `Receivable` account and returns its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn create_receivable(ctx: &TestContext, name: &str) -> String {
    let out = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            name,
            "--type",
            "asset",
            "--kind",
            "receivable",
        ])
        .output()
        .expect("create Receivable");
    parse_account_id(&out.stdout)
}

/// Creates a standard deposit account and returns its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn create_deposit_account(ctx: &TestContext, name: &str) -> String {
    let out = ctx
        .command()
        .args([
            "--json", "account", "create", "--name", name, "--type", "asset",
        ])
        .output()
        .expect("create DepositAccount");
    parse_account_id(&out.stdout)
}

#[test]
fn record_valuation_happy_path() {
    let ctx = TestContext::new();
    let account_id = create_manual_asset(&ctx, "My House");

    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "asset",
        "record-valuation",
        "--account",
        &account_id,
        "--amount",
        "650000.00",
        "--commodity",
        "AUD",
        "--source",
        "manual-estimate",
        "--date",
        "2026-03-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn record_valuation_wrong_account_kind_fails() {
    let ctx = TestContext::new();
    let deposit_id = create_deposit_account(&ctx, "Savings");

    let mut cmd = ctx.command();
    cmd.args([
        "asset",
        "record-valuation",
        "--account",
        &deposit_id,
        "--amount",
        "10000.00",
        "--commodity",
        "AUD",
        "--source",
        "manual-estimate",
        "--date",
        "2026-03-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn set_loan_terms_happy_path() {
    let ctx = TestContext::new();
    let account_id = create_receivable(&ctx, "Loan to Friend");

    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "asset",
        "set-loan-terms",
        "--account",
        &account_id,
        "--principal",
        "20000.00",
        "--rate",
        "0.05",
        "--start",
        "2026-01-01",
        "--term-months",
        "24",
        "--frequency",
        "monthly",
        "--commodity",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn set_loan_terms_wrong_account_kind_fails() {
    let ctx = TestContext::new();
    let deposit_id = create_deposit_account(&ctx, "Not A Receivable");

    let mut cmd = ctx.command();
    cmd.args([
        "asset",
        "set-loan-terms",
        "--account",
        &deposit_id,
        "--principal",
        "5000.00",
        "--rate",
        "0.05",
        "--start",
        "2026-01-01",
        "--term-months",
        "12",
        "--frequency",
        "monthly",
        "--commodity",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn amortization_shows_schedule() {
    let ctx = TestContext::new();
    let account_id = create_receivable(&ctx, "Loan with Schedule");

    // First set loan terms.
    ctx.command()
        .args([
            "asset",
            "set-loan-terms",
            "--account",
            &account_id,
            "--principal",
            "12000.00",
            "--rate",
            "0.06",
            "--start",
            "2026-01-01",
            "--term-months",
            "12",
            "--frequency",
            "monthly",
            "--commodity",
            "AUD",
        ])
        .output()
        .expect("set loan terms");

    // Then display the amortization schedule.
    let mut cmd = ctx.command();
    cmd.args(["--json", "asset", "amortization", "--account", &account_id]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn depreciate_no_policy_returns_error() {
    let ctx = TestContext::new();
    // Create a ManualAsset with no depreciation policy by going through a
    // DepositAccount — or rather, create a manual asset and try depreciation
    // which requires a policy. The account create command does not yet expose
    // depreciation policy flags, so we use an account that cannot be depreciated
    // (a DepositAccount) to force an error from the asset kind check.
    let deposit_id = create_deposit_account(&ctx, "Not A ManualAsset");

    // Also create an expense account to satisfy the CLI args.
    let expense_id = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
            "Depreciation Expense",
            "--type",
            "expense",
        ])
        .output()
        .expect("create expense account");
    let expense_id = parse_account_id(&expense_id.stdout);

    let mut cmd = ctx.command();
    cmd.args([
        "asset",
        "depreciate",
        "--account",
        &deposit_id,
        "--commodity",
        "AUD",
        "--date",
        "2026-03-31",
        "--expense-account",
        &expense_id,
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
