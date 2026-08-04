//! Integration tests for the `commodity` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn list_shows_the_seeded_commodities() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn list_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}
