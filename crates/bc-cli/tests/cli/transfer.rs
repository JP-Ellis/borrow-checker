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

use pretty_assertions::assert_eq;

use crate::cmd_snapshot;
use crate::common::TestContext;

/// Runs `cmd` and parses its stdout as JSON.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn json_of(cmd: &mut assert_cmd::Command) -> serde_json::Value {
    let output = cmd.output().expect("command runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON")
}

/// Creates a single-posting transaction, `ACCOUNT` for `amount` AUD, by
/// adding a balancing offset posting into a scratch account and then
/// removing it — `transaction add` requires at least two postings, so a
/// one-leg transaction is built via `add` then `edit --remove-posting`.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn one_posting_transaction(ctx: &TestContext, account: &str, amount: i64) -> String {
    ctx.command()
        .args(["account", "create", account])
        .output()
        .expect("create account");
    let scratch = format!("Equity:Scratch:{account}");
    ctx.command()
        .args(["account", "create", &scratch])
        .output()
        .expect("create scratch account");

    let negated = amount.checked_neg().expect("amount does not overflow");
    let added = json_of(ctx.command().args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "one-leg row",
        "--posting",
        account,
        &amount.to_string(),
        "AUD",
        "--posting",
        &scratch,
        &negated.to_string(),
        "AUD",
    ]));
    let id = added
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned();

    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &id,
        "--remove-posting",
        &scratch,
    ]));
    edited
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned()
}

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

#[test]
fn merge_unequal_legs_warns() {
    let ctx = TestContext::new();
    let debit = one_posting_transaction(&ctx, "Assets:Checking", -100);
    let credit = one_posting_transaction(&ctx, "Assets:Mortgage", 90);
    let mut cmd = ctx.command();
    cmd.args(["merge", &debit, &credit]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// The `warnings` array's entries, as strings.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn warning_strings(merged: &serde_json::Value) -> Vec<&str> {
    merged
        .get("warnings")
        .and_then(serde_json::Value::as_array)
        .expect("warnings array")
        .iter()
        .map(|w| w.as_str().expect("warning is a string"))
        .collect()
}

#[test]
fn merge_unequal_legs_json_warnings() {
    let ctx = TestContext::new();
    let debit = one_posting_transaction(&ctx, "Assets:Checking", -100);
    let credit = one_posting_transaction(&ctx, "Assets:Mortgage", 90);
    let merged = json_of(ctx.command().args(["--json", "merge", &debit, &credit]));
    let warnings = warning_strings(&merged);
    assert_eq!(warnings.len(), 1, "{merged:?}");
    let warning = warnings.first().unwrap_or(&"");
    assert!(
        warning.contains("residual"),
        "warning should name the residual: {warning}"
    );
}
