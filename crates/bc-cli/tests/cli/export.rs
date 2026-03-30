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
