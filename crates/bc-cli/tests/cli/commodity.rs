//! Integration tests for the `commodity` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::cmd_snapshot;
use crate::common::TestContext;

/// Runs a command whose output is setup for the assertion that follows.
///
/// `Command::output` only fails when the process cannot be spawned, so a
/// non-zero exit would otherwise pass silently and leave the test asserting
/// against a registry the setup never modified.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn setup(ctx: &TestContext, args: &[&str]) {
    let output = ctx.command().args(args).output().expect("spawn");
    assert!(
        output.status.success(),
        "setup command {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

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

#[test]
fn create_registers_a_new_commodity() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &[
            "commodity",
            "create",
            "SOL",
            "--name",
            "Solana",
            "--decimals",
            "9",
            "--no-iso",
        ],
    );

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_defaults_to_two_decimals_and_iso() {
    let ctx = TestContext::new();
    setup(&ctx, &["commodity", "create", "TND"]);

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_removes_an_unreferenced_commodity() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &["commodity", "create", "DOGE", "--decimals", "8", "--no-iso"],
    );
    setup(&ctx, &["commodity", "delete", "DOGE"]);

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_matches_a_code_case_insensitively() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &["commodity", "create", "DOGE", "--decimals", "8", "--no-iso"],
    );

    let mut cmd = ctx.command();
    cmd.args(["commodity", "delete", "doge"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_matches_an_alias_exactly() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["commodity", "delete", "AU$"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_reports_an_unknown_marker() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["commodity", "delete", "NOPE"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_rejects_a_conflicting_marker() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["commodity", "create", "btc", "--decimals", "8", "--no-iso"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_accepts_repeated_aliases() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &[
            "commodity",
            "create",
            "XMR",
            "--decimals",
            "12",
            "--no-iso",
            "--symbol",
            "ɱ",
            "--symbol-after",
            "--alias",
            "Monero",
            "--alias",
            "monero-xmr",
        ],
    );

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_changes_only_the_flags_given() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &[
            "commodity",
            "create",
            "SOL",
            "--name",
            "Solana",
            "--decimals",
            "9",
            "--no-iso",
        ],
    );
    setup(&ctx, &["commodity", "update", "SOL", "--symbol", "◎"]);

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_adds_and_removes_aliases() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &[
            "commodity",
            "update",
            "USD",
            "--add-alias",
            "dollar",
            "--remove-alias",
            "US$",
        ],
    );

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_rejects_removing_an_absent_alias() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["commodity", "update", "USD", "--remove-alias", "nope"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_can_turn_a_boolean_back_off() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &["commodity", "update", "ETH", "--iso", "--no-symbol-after"],
    );

    // The table form has no `symbol_after` column, so only the JSON form can
    // show that `--no-symbol-after` took effect. ETH seeds with it set.
    let mut cmd = ctx.command();
    cmd.args(["--json", "commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_can_turn_iso_back_off() {
    let ctx = TestContext::new();
    setup(&ctx, &["commodity", "update", "AUD", "--no-iso"]);

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_clears_a_field_with_an_empty_value() {
    let ctx = TestContext::new();
    setup(&ctx, &["commodity", "update", "AUD", "--symbol", ""]);

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_clears_an_active_date_with_an_empty_value() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &[
            "commodity",
            "create",
            "XPF",
            "--active-from",
            "2020-01-01",
            "--active-until",
            "2030-12-31",
        ],
    );
    setup(&ctx, &["commodity", "update", "XPF", "--active-until", ""]);

    let mut cmd = ctx.command();
    cmd.args(["--json", "commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_json_reports_the_edited_commodity() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "commodity", "update", "AUD", "--symbol", "$A"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_json_reports_the_removed_commodity() {
    let ctx = TestContext::new();
    setup(
        &ctx,
        &["commodity", "create", "DOGE", "--decimals", "8", "--no-iso"],
    );

    let mut cmd = ctx.command();
    cmd.args(["--json", "commodity", "delete", "DOGE"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_rejects_the_same_alias_twice() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "commodity",
        "update",
        "USD",
        "--add-alias",
        "buck",
        "--add-alias",
        "buck",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
