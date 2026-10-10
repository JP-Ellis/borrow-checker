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
    cmd.args(["import", "run", "--profile", "nonexistent"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn rejected_on_an_empty_ledger() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["import", "rejected"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn rejected_on_an_empty_ledger_as_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "import", "rejected"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn rejected_for_an_unknown_account_is_an_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["import", "rejected", "--account", "Assets:Nowhere"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn release_of_an_unknown_group_is_an_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "import",
        "rejected",
        "release",
        "transaction_01h455vb4pex5vsknk084sn02q",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn release_of_a_malformed_id_is_an_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["import", "rejected", "release", "nonsense"]);
    cmd_snapshot!(ctx, &mut cmd);
}
