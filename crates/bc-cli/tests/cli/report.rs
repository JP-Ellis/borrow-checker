//! Integration tests for the `report` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn net_worth_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "net-worth"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn net_worth_empty_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "report", "net-worth"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn summary_monthly_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "report",
        "summary",
        "--period",
        "monthly",
        "--date",
        "2026-03-15",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn summary_calendar_year_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "report",
        "summary",
        "--period",
        "calendar-year",
        "--date",
        "2026-06-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn net_worth_includes_manual_asset_at_market_value() {
    let ctx = TestContext::new();

    // Create a ManualAsset account.
    let out = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "Family Home",
            "--type",
            "asset",
            "--kind",
            "manual-asset",
        ])
        .output()
        .expect("create ManualAsset");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let account_id = json
        .get("account")
        .and_then(|account| account.get("id"))
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned();

    // Record a valuation.
    ctx.command()
        .args([
            "asset",
            "record-valuation",
            "--account",
            &account_id,
            "--amount",
            "750000.00",
            "--commodity",
            "AUD",
            "--source",
            "professional-appraisal",
            "--date",
            "2026-01-15",
        ])
        .output()
        .expect("record valuation");

    // Net-worth report should include the ManualAsset at its recorded market value.
    let mut cmd = ctx.command();
    cmd.args(["--json", "report", "net-worth"]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Creates a Checking (asset) and Interest (income) account, and returns
/// their IDs.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn setup_accounts(ctx: &TestContext) -> (String, String) {
    let checking_out = ctx
        .command()
        .args([
            "--json", "account", "create", "--name", "Checking", "--type", "asset",
        ])
        .output()
        .expect("create checking");
    let checking_json: serde_json::Value =
        serde_json::from_slice(&checking_out.stdout).expect("valid JSON");
    let checking_id = checking_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned();

    let interest_out = ctx
        .command()
        .args([
            "--json", "account", "create", "--name", "Interest", "--type", "income",
        ])
        .output()
        .expect("create interest");
    let interest_json: serde_json::Value =
        serde_json::from_slice(&interest_out.stdout).expect("valid JSON");
    let interest_id = interest_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned();

    (checking_id, interest_id)
}

#[test]
fn summary_fy_includes_a_transaction_in_the_financial_year() {
    let ctx = TestContext::new();
    let (checking_id, interest_id) = setup_accounts(&ctx);

    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2025-08-01",
            "--description",
            "Interest payment",
            "--posting",
            &format!("{checking_id}:100.00:AUD"),
            "--posting",
            &format!("{interest_id}:-100.00:AUD"),
        ])
        .output()
        .expect("add transaction");

    let mut cmd = ctx.command();
    cmd.args(["report", "summary", "--fy", "2026"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn categories_fy_totals_a_transaction_in_the_financial_year() {
    let ctx = TestContext::new();
    let (checking_id, interest_id) = setup_accounts(&ctx);

    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2025-08-01",
            "--description",
            "Interest payment",
            "--posting",
            &format!("{checking_id}:100.00:AUD"),
            "--posting",
            &format!("{interest_id}:-100.00:AUD"),
        ])
        .output()
        .expect("add transaction");

    let mut cmd = ctx.command();
    cmd.args(["report", "categories", "--fy", "2026"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn categories_custom_period_is_rejected() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "categories", "--period", "custom"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn categories_unresolvable_account_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "categories", "--account", "No:Such:Account"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn categories_unresolvable_account_errors_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "report",
        "categories",
        "--account",
        "No:Such:Account",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn categories_unresolvable_tag_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "categories", "--tag", "no-such-tag"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn categories_unresolvable_tag_errors_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "report", "categories", "--tag", "no-such-tag"]);
    cmd_snapshot!(ctx, &mut cmd);
}
