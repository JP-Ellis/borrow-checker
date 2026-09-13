//! Integration tests for the `sync` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn sync_missing_profile_returns_error() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args(["sync", "--profile", "nonexistent"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn sync_all_over_no_profiles_reports_zero_and_exits_zero() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args(["sync", "--all"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn sync_all_dry_run_over_no_profiles_is_json_clean() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args(["--json", "sync", "--all", "--dry-run"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn sync_requires_profile_or_all() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args(["sync"]);
    cmd_snapshot!(ctx, &mut cmd);
}
