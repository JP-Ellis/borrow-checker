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
fn monthly_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "monthly", "2026-03"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn annual_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "annual", "2026"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn budget_stub() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["report", "budget"]);
    cmd_snapshot!(ctx, &mut cmd);
}
