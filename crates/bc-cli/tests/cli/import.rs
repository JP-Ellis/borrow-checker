//! Integration tests for the `import` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use assert_fs::fixture::FileWriteStr as _;
use assert_fs::fixture::PathChild as _;

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn import_missing_profile_returns_error() {
    let ctx = TestContext::new();
    let file = ctx.home_dir.child("test.csv");
    file.write_str("date,amount,description\n2026-01-01,-50.00,Test\n")
        .expect("write fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "import",
        "--profile",
        "nonexistent",
        "--counterpart",
        "account_00000000000000000000000000",
        file.path().to_str().expect("utf8 path"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
