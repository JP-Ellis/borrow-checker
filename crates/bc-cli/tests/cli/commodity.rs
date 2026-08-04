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

#[test]
fn create_registers_a_new_commodity() {
    let ctx = TestContext::new();
    ctx.command()
        .args([
            "commodity",
            "create",
            "SOL",
            "--name",
            "Solana",
            "--decimals",
            "9",
            "--no-iso",
        ])
        .output()
        .expect("create SOL");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn create_defaults_to_two_decimals_and_iso() {
    let ctx = TestContext::new();
    ctx.command()
        .args(["commodity", "create", "TND"])
        .output()
        .expect("create TND");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_removes_an_unreferenced_commodity() {
    let ctx = TestContext::new();
    ctx.command()
        .args(["commodity", "create", "DOGE", "--decimals", "8", "--no-iso"])
        .output()
        .expect("create DOGE");
    ctx.command()
        .args(["commodity", "delete", "DOGE"])
        .output()
        .expect("delete DOGE");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn delete_matches_a_code_case_insensitively() {
    let ctx = TestContext::new();
    ctx.command()
        .args(["commodity", "create", "DOGE", "--decimals", "8", "--no-iso"])
        .output()
        .expect("create DOGE");

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
    ctx.command()
        .args([
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
        ])
        .output()
        .expect("create XMR");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_changes_only_the_flags_given() {
    let ctx = TestContext::new();
    ctx.command()
        .args([
            "commodity",
            "create",
            "SOL",
            "--name",
            "Solana",
            "--decimals",
            "9",
            "--no-iso",
        ])
        .output()
        .expect("create SOL");
    ctx.command()
        .args(["commodity", "update", "SOL", "--symbol", "◎"])
        .output()
        .expect("update SOL");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_adds_and_removes_aliases() {
    let ctx = TestContext::new();
    ctx.command()
        .args([
            "commodity",
            "update",
            "USD",
            "--add-alias",
            "dollar",
            "--remove-alias",
            "US$",
        ])
        .output()
        .expect("update USD");

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
    ctx.command()
        .args(["commodity", "update", "ETH", "--iso", "--no-symbol-after"])
        .output()
        .expect("update ETH");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn update_clears_a_field_with_an_empty_value() {
    let ctx = TestContext::new();
    ctx.command()
        .args(["commodity", "update", "AUD", "--symbol", ""])
        .output()
        .expect("update AUD");

    let mut cmd = ctx.command();
    cmd.args(["commodity", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}
