//! Integration tests for the `meta` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use pretty_assertions::assert_eq;

use crate::cmd_snapshot;
use crate::common::TestContext;

/// Parses an account ID string from a JSON output buffer.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn parse_account_id(stdout: &[u8]) -> String {
    let json: serde_json::Value = serde_json::from_slice(stdout).expect("valid JSON");
    json.get("account")
        .and_then(|account| account.get("id"))
        .and_then(serde_json::Value::as_str)
        .expect("id field")
        .to_owned()
}

/// Registers `key` by writing one transaction carrying `value` under it.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn register_key(ctx: &TestContext, spec: &str) {
    let checking = parse_account_id(
        &ctx.command()
            .args(["--json", "account", "create", "Assets:Checking"])
            .output()
            .expect("create checking")
            .stdout,
    );
    let expenses = parse_account_id(
        &ctx.command()
            .args(["--json", "account", "create", "Expenses:Groceries"])
            .output()
            .expect("create expenses")
            .stdout,
    );
    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Coffee",
            "--meta",
            spec,
            "--posting",
            &format!("{checking}:-5.00:AUD"),
            "--posting",
            &format!("{expenses}:5.00:AUD"),
        ])
        .output()
        .expect("add");
}

/// Reads `meta list --json` as `(key, type)` pairs.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn registry(ctx: &TestContext) -> Vec<(String, String)> {
    let out = ctx
        .command()
        .args(["--json", "meta", "list"])
        .output()
        .expect("meta list");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    json.as_array()
        .expect("array")
        .iter()
        .map(|def| {
            (
                def.get("key")
                    .and_then(serde_json::Value::as_str)
                    .expect("key")
                    .to_owned(),
                def.get("ty")
                    .and_then(serde_json::Value::as_str)
                    .expect("ty")
                    .to_owned(),
            )
        })
        .collect()
}

#[test]
fn list_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["meta", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn a_written_key_appears_in_the_registry() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");
    assert_eq!(
        registry(&ctx),
        vec![("invoice".to_owned(), "number".to_owned())],
        "a key enters the registry on first write, with the type its value read as"
    );
}

#[test]
fn retype_reports_the_type_it_replaced() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    let mut cmd = ctx.command();
    cmd.args(["meta", "retype", "invoice", "text"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn retype_changes_the_registered_type() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    ctx.command()
        .args(["meta", "retype", "invoice", "text"])
        .output()
        .expect("retype");

    assert_eq!(
        registry(&ctx),
        vec![("invoice".to_owned(), "text".to_owned())]
    );
}

#[test]
fn retyping_a_key_to_what_it_already_holds_says_so() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    let mut cmd = ctx.command();
    cmd.args(["meta", "retype", "invoice", "number"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn retyping_an_unregistered_key_fails() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["meta", "retype", "invoice", "text"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn retype_rejects_a_type_it_does_not_know() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["meta", "retype", "invoice", "colour"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn rename_moves_the_key() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    let mut cmd = ctx.command();
    cmd.args(["meta", "rename", "invoice", "bill"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn a_renamed_key_keeps_its_type_and_its_entries() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    ctx.command()
        .args(["meta", "rename", "invoice", "bill"])
        .output()
        .expect("rename");

    assert_eq!(
        registry(&ctx),
        vec![("bill".to_owned(), "number".to_owned())],
        "a rename is the same key under a new name, not a fresh registration"
    );

    let out = ctx
        .command()
        .args(["--json", "transaction", "list"])
        .output()
        .expect("list");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let keys: Vec<String> = json
        .as_array()
        .expect("array")
        .iter()
        .flat_map(|tx| {
            tx.get("metadata")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|entry| {
            entry
                .get("key")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect();
    assert_eq!(keys, vec!["bill".to_owned()], "entries follow the key");
}

#[test]
fn renaming_onto_a_registered_key_fails() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");
    register_key(&ctx, "note=paid in cash");

    let mut cmd = ctx.command();
    cmd.args(["meta", "rename", "invoice", "note"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn a_key_the_registry_never_saw_cannot_be_renamed() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["meta", "rename", "invoice", "bill"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn an_invalid_key_is_rejected_before_the_registry_is_touched() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["meta", "rename", "invoice", "1bill"]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Filters that collapse a table's column padding and its separator rule.
///
/// A registration timestamp renders with as many fractional-second digits as
/// it has, and the table sizes the `REGISTERED` column to that width, so the
/// padding and the rule shift from run to run. `TestContext` already rewrites
/// the timestamp itself; these leave only the cells and the pipes.
const TABLE_FILTERS: [(&str, &str); 2] = [(" {2,}", " "), ("={2,}", "=")];

#[test]
fn list_usage_counts_a_written_key() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    let mut cmd = ctx.command();
    cmd.args(["meta", "list", "--usage"]);
    insta::with_settings!({ filters => TABLE_FILTERS.to_vec() }, {
        cmd_snapshot!(ctx, &mut cmd);
    });
}

#[test]
fn list_usage_json_carries_the_counts() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    let out = ctx
        .command()
        .args(["--json", "meta", "list", "--usage"])
        .output()
        .expect("meta list --usage");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    let first = json.get(0).expect("one key");

    assert_eq!(
        first.get("key").and_then(serde_json::Value::as_str),
        Some("invoice"),
        "the registration's fields are flattened alongside the counts"
    );
    assert_eq!(
        first
            .get("transactions")
            .and_then(serde_json::Value::as_u64),
        Some(1),
        "one entry was written on a transaction"
    );
    assert_eq!(
        first.get("mismatched").and_then(serde_json::Value::as_u64),
        Some(0),
        "1502 reads as the number the key registered as"
    );
}

#[test]
fn list_without_usage_keeps_its_three_columns() {
    let ctx = TestContext::new();
    register_key(&ctx, "invoice=1502");

    let mut cmd = ctx.command();
    cmd.args(["meta", "list"]);
    insta::with_settings!({ filters => TABLE_FILTERS.to_vec() }, {
        cmd_snapshot!(ctx, &mut cmd);
    });
}
