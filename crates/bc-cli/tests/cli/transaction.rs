//! Integration tests for the `transaction` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use pretty_assertions::assert_eq;

use crate::cmd_snapshot;
use crate::common::TestContext;

#[test]
fn list_empty() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn list_empty_json() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["--json", "transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

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

/// Creates two accounts and returns their IDs.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn setup_accounts(ctx: &TestContext) -> (String, String) {
    let checking_out = ctx
        .command()
        .args(["--json", "account", "create", "Assets:Checking"])
        .output()
        .expect("create checking");
    let checking_id = parse_account_id(&checking_out.stdout);

    let expenses_out = ctx
        .command()
        .args(["--json", "account", "create", "Expenses:Groceries"])
        .output()
        .expect("create expenses");
    let expenses_id = parse_account_id(&expenses_out.stdout);

    (checking_id, expenses_id)
}

#[test]
fn add_transaction() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        &format!("{checking_id}:-50.00:AUD"),
        "--posting",
        &format!("{expenses_id}:50.00:AUD"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_transaction_json() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        &format!("{checking_id}:-50.00:AUD"),
        "--posting",
        &format!("{expenses_id}:50.00:AUD"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_unbalanced_transaction_fails() {
    let ctx = TestContext::new();
    let (checking_id, _) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Unbalanced",
        "--posting",
        &format!("{checking_id}:-50.00:AUD"),
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn reverse_existing_transaction() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);

    let add_out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "To reverse",
            "--posting",
            &format!("{checking_id}:-10.00:AUD"),
            "--posting",
            &format!("{expenses_id}:10.00:AUD"),
        ])
        .output()
        .expect("add");
    let add_json: serde_json::Value = serde_json::from_slice(&add_out.stdout).expect("json");
    let tx_id = add_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned();

    let mut cmd = ctx.command();
    cmd.args(["transaction", "reverse", &tx_id]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn reverse_nonexistent_transaction_returns_error() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["transaction", "reverse", "transaction_notavalidid000000000"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn amend_description() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);

    let amend_out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Original desc",
            "--posting",
            &format!("{checking_id}:-20.00:AUD"),
            "--posting",
            &format!("{expenses_id}:20.00:AUD"),
        ])
        .output()
        .expect("add");
    let amend_json: serde_json::Value = serde_json::from_slice(&amend_out.stdout).expect("json");
    let tx_id = amend_json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned();

    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "amend",
        &tx_id,
        "--description",
        "Amended desc",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn amend_date_only() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);

    let out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Original",
            "--posting",
            &format!("{checking_id}:-10.00:AUD"),
            "--posting",
            &format!("{expenses_id}:10.00:AUD"),
        ])
        .output()
        .expect("add");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let tx_id = json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned();

    let mut cmd = ctx.command();
    cmd.args(["transaction", "amend", &tx_id, "--date", "2026-04-15"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn amend_after_reversal_succeeds() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);

    let out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "To void then amend",
            "--posting",
            &format!("{checking_id}:-10.00:AUD"),
            "--posting",
            &format!("{expenses_id}:10.00:AUD"),
        ])
        .output()
        .expect("add");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let tx_id = json
        .get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned();

    ctx.command()
        .args(["transaction", "reverse", &tx_id])
        .output()
        .expect("reverse");

    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "amend",
        &tx_id,
        "--description",
        "Should succeed after reversal",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Adds a balanced two-leg transaction carrying `extra` arguments, and returns
/// its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn add_with(ctx: &TestContext, checking: &str, expenses: &str, extra: &[&str]) -> String {
    let mut args: Vec<String> = ["--json", "transaction", "add", "--date", "2026-03-01"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    args.push("--description".to_owned());
    args.push("Coffee".to_owned());
    args.extend(extra.iter().map(|s| (*s).to_owned()));
    args.push("--posting".to_owned());
    args.push(format!("{checking}:-5.00:AUD"));
    args.push("--posting".to_owned());
    args.push(format!("{expenses}:5.00:AUD"));

    let out = ctx.command().args(&args).output().expect("add");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    json.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned()
}

/// Reads a transaction's metadata as `(key, value)` pairs in stored order,
/// where each value is its externally-tagged JSON object.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn metadata_of(stdout: &[u8]) -> Vec<(String, serde_json::Value)> {
    let json: serde_json::Value = serde_json::from_slice(stdout).expect("json");
    json.get("metadata")
        .and_then(serde_json::Value::as_array)
        .expect("metadata array")
        .iter()
        .map(|entry| {
            (
                entry
                    .get("key")
                    .and_then(serde_json::Value::as_str)
                    .expect("key")
                    .to_owned(),
                entry.get("value").expect("value").clone(),
            )
        })
        .collect()
}

/// Re-reads one transaction out of `transaction list`, as JSON.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn reload(ctx: &TestContext, tx_id: &str) -> Vec<u8> {
    let out = ctx
        .command()
        .args(["--json", "transaction", "list"])
        .output()
        .expect("list");
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    let found = json
        .as_array()
        .expect("array")
        .iter()
        .find(|tx| tx.get("id").and_then(serde_json::Value::as_str) == Some(tx_id))
        .expect("the transaction just written")
        .clone();
    serde_json::to_vec(&found).expect("re-serialise")
}

#[test]
fn add_takes_a_new_keys_type_from_its_value() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &[
            "--meta",
            "payee=Generic Grocer",
            "--meta",
            "invoice=1502",
            "--meta",
            "reimbursed=false",
            "--meta",
            "due=2026-04-15",
        ],
    );

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![
            (
                "payee".to_owned(),
                serde_json::json!({ "text": "Generic Grocer" })
            ),
            ("invoice".to_owned(), serde_json::json!({ "number": "1502" })),
            (
                "reimbursed".to_owned(),
                serde_json::json!({ "boolean": false })
            ),
            ("due".to_owned(), serde_json::json!({ "date": "2026-04-15" })),
        ],
        "a key the registry has not seen takes the type its value reads as"
    );
}

#[test]
fn add_takes_a_registered_keys_type_over_its_value() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    // Registers `invoice` as a number.
    add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "invoice=1502"],
    );
    let second = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "invoice=1600"],
    );

    assert_eq!(
        metadata_of(&reload(&ctx, &second)),
        vec![("invoice".to_owned(), serde_json::json!({ "number": "1600" }))],
        "the registry decides the type, so the second write is a number too"
    );
}

#[test]
fn a_value_the_registered_type_cannot_read_is_stored_flagged() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "invoice=1502"],
    );
    let second = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "invoice=awaiting the paperwork"],
    );

    let json: serde_json::Value =
        serde_json::from_slice(&reload(&ctx, &second)).expect("valid JSON");
    let entry = json
        .get("metadata")
        .and_then(serde_json::Value::as_array)
        .and_then(|entries| entries.first())
        .expect("one entry");
    assert_eq!(
        entry.get("value"),
        Some(&serde_json::json!({ "text": "awaiting the paperwork" })),
        "nothing is lost: the text is kept verbatim"
    );
    assert_eq!(
        entry.get("mismatched"),
        Some(&serde_json::Value::Bool(true)),
        "and the store flags what it could not fit"
    );
}

#[test]
fn a_meta_value_may_contain_an_equals_sign() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "query=type=expense"],
    );

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![(
            "query".to_owned(),
            serde_json::json!({ "text": "type=expense" })
        )],
        "the split is on the first '=' only"
    );
}

#[test]
fn one_key_may_carry_several_entries() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "note=first", "--meta", "note=second"],
    );

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![
            ("note".to_owned(), serde_json::json!({ "text": "first" })),
            ("note".to_owned(), serde_json::json!({ "text": "second" })),
        ],
        "repeated keys are legal and there is nothing to collapse"
    );
}

#[test]
fn amend_replaces_one_key_in_place_and_leaves_the_others() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &[
            "--meta",
            "payee=Generic Grocer",
            "--meta",
            "note=weekly shop",
        ],
    );

    ctx.command()
        .args([
            "transaction",
            "amend",
            &tx_id,
            "--meta",
            "payee=Other Grocer",
        ])
        .output()
        .expect("amend");

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![
            (
                "payee".to_owned(),
                serde_json::json!({ "text": "Other Grocer" })
            ),
            (
                "note".to_owned(),
                serde_json::json!({ "text": "weekly shop" })
            ),
        ],
        "position is the display order, so a replaced key keeps the place it held"
    );
}

#[test]
fn amend_clear_meta_removes_every_entry_under_the_key() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &[
            "--meta",
            "note=first",
            "--meta",
            "payee=Generic Grocer",
            "--meta",
            "note=second",
        ],
    );

    ctx.command()
        .args(["transaction", "amend", &tx_id, "--clear-meta", "note"])
        .output()
        .expect("amend");

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![(
            "payee".to_owned(),
            serde_json::json!({ "text": "Generic Grocer" })
        )],
        "clearing a key removes every entry under it, not just the first"
    );
}

#[test]
fn list_renders_metadata_and_marks_what_did_not_fit() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    // Registers `invoice` as a number, then writes text under it.
    add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "invoice=1502"],
    );
    add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &[
            "--meta",
            "payee=Generic Grocer",
            "--meta",
            "invoice=awaiting the paperwork",
        ],
    );

    let mut cmd = ctx.command();
    cmd.args(["transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn amend_rejects_setting_and_clearing_one_key() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "note=first"],
    );

    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "amend",
        &tx_id,
        "--meta",
        "note=second",
        "--clear-meta",
        "note",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
