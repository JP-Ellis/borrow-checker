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
fn budget_stub() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "budget"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn net_worth_includes_manual_asset_at_market_value() {
    let ctx = TestContext::new();

    // Create a ManualAsset account.
    let out = ctx
        .command()
        .args([
            "--json",
            "account",
            "create",
            "--name",
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
        .get("id")
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
