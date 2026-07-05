//! Integration tests for the transfer resolution subcommands.
//!
//! The merge/unmerge/suggest logic itself is exercised exhaustively by the
//! `bc-core` unit tests; these cover the CLI wiring, argument parsing, and the
//! empty-suggestion branch. A full happy-path (import two legs, suggest, merge)
//! requires the import-profile setup and belongs to the end-to-end suite.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn suggest_transfers_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["suggest-transfers"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn suggest_transfers_empty_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "suggest-transfers"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn merge_invalid_ids_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["merge", "not-an-id", "also-not-an-id"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn unmerge_invalid_id_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["unmerge", "not-an-id"]);
    cmd_snapshot!(ctx, &mut cmd);
}
