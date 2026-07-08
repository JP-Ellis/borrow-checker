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

    let account_id = bc_models::AccountId::new().to_string();
    let mut cmd = ctx.command();
    cmd.args([
        "import",
        "--profile",
        "nonexistent",
        "--account",
        &account_id,
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
