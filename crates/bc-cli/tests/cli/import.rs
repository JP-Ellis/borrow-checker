//! Integration tests for the `import` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn import_missing_profile_returns_error() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args(["import", "--profile", "nonexistent"]);
    cmd_snapshot!(ctx, &mut cmd);
}
