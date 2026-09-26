//! Integration tests for the `transaction` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use core::str::FromStr as _;

use pretty_assertions::assert_eq;
use rust_decimal_macros::dec;

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
        &checking_id,
        "-50.00",
        "AUD",
        "--posting",
        &expenses_id,
        "50.00",
        "AUD",
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
        &checking_id,
        "-50.00",
        "AUD",
        "--posting",
        &expenses_id,
        "50.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn transaction_warning() {
    // Dated before the account's declared opening date: the write succeeds
    // — "warn, don't block" — with the id on stdout and the warning on
    // stderr, so a script piping stdout is unaffected.
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    ctx.command()
        .args([
            "account",
            "set-opened-on",
            &checking_id,
            "--on",
            "2024-01-01",
        ])
        .output()
        .expect("set-opened-on");

    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2020-01-01",
        "--description",
        "Grocery shopping",
        "--posting",
        &checking_id,
        "-50.00",
        "AUD",
        "--posting",
        &expenses_id,
        "50.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Creates a brokerage account and returns its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn setup_brokerage(ctx: &TestContext) -> String {
    let out = ctx
        .command()
        .args(["--json", "account", "create", "Assets:Brokerage"])
        .output()
        .expect("create brokerage");
    parse_account_id(&out.stdout)
}

#[test]
fn add_priced_leg_then_list() {
    // A foreign-currency purchase: the USD leg weighs at its @@ total, so
    // the transaction balances against the AUD leg.
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Online purchase",
            "--posting",
            &checking_id,
            "-6.37",
            "AUD",
            "--posting",
            &expenses_id,
            "4.00",
            "USD",
            "--total-price",
            "6.37",
            "AUD",
        ])
        .output()
        .expect("add");

    let mut cmd = ctx.command();
    cmd.args(["transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_costed_leg_then_list() {
    let ctx = TestContext::new();
    let (checking_id, _) = setup_accounts(&ctx);
    let brokerage_id = setup_brokerage(&ctx);
    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Buy shares",
            "--posting",
            &checking_id,
            "-210",
            "AUD",
            "--posting",
            &brokerage_id,
            "2",
            "AAPL",
            "--cost",
            "105",
            "AUD",
            "--lot-date",
            "2024-03-01",
            "--lot-label",
            "lot-a",
        ])
        .output()
        .expect("add");

    let mut cmd = ctx.command();
    cmd.args(["transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_priced_and_costed_leg_json() {
    let ctx = TestContext::new();
    let (checking_id, _) = setup_accounts(&ctx);
    let brokerage_id = setup_brokerage(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Sell shares",
        // Cost beats price for weighing: the AAPL leg weighs -210 AUD, so
        // 210 AUD balances it and the 150 AUD price is carried as stated.
        "--posting",
        &checking_id,
        "210",
        "AUD",
        "--posting",
        &brokerage_id,
        "-2",
        "AAPL",
        "--cost",
        "105",
        "AUD",
        "--lot-date",
        "2024-03-01",
        "--lot-label",
        "lot-a",
        "--price",
        "150",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_costed_sale_then_list() {
    // The sale leg is negative, and the table shows it anyway: its cost and
    // price are the record of the trade.
    let ctx = TestContext::new();
    let (checking_id, _) = setup_accounts(&ctx);
    let brokerage_id = setup_brokerage(&ctx);
    ctx.command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Sell shares",
            "--posting",
            &checking_id,
            "210",
            "AUD",
            "--posting",
            &brokerage_id,
            "-2",
            "AAPL",
            "--cost",
            "105",
            "AUD",
            "--lot-date",
            "2024-03-01",
            "--lot-label",
            "lot-a",
            "--price",
            "150",
            "AUD",
        ])
        .output()
        .expect("add");

    let mut cmd = ctx.command();
    cmd.args(["transaction", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_leg_quoted_in_its_own_commodity_warns() {
    // A fee stated as N AUD @ P AUD is what Beancount weighs as N × P AUD;
    // the write succeeds and the warning names the account on stderr.
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Fee",
        "--posting",
        &checking_id,
        "-10",
        "AUD",
        "--posting",
        &expenses_id,
        "5",
        "AUD",
        "--price",
        "2",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_rejects_negative_price() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Bad price",
        "--posting",
        &checking_id,
        "-6.37",
        "AUD",
        "--posting",
        &expenses_id,
        "4.00",
        "USD",
        "--total-price",
        "-6.37",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn add_rejects_a_lot_date_without_a_cost() {
    // Naming a lot by its date alone selects a held lot, which needs
    // inventory booking; a new leg's lot date needs its cost beside it.
    let ctx = TestContext::new();
    let (checking_id, _) = setup_accounts(&ctx);
    let brokerage_id = setup_brokerage(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Sell shares",
        "--posting",
        &checking_id,
        "300",
        "AUD",
        "--posting",
        &brokerage_id,
        "-2",
        "AAPL",
        "--lot-date",
        "2024-03-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn transaction_warning_json() {
    // Same write as `transaction_warning`, but under `--json`: the warning
    // is not silently dropped — it rides along in the payload.
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    ctx.command()
        .args([
            "account",
            "set-opened-on",
            &checking_id,
            "--on",
            "2024-01-01",
        ])
        .output()
        .expect("set-opened-on");

    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2020-01-01",
        "--description",
        "Grocery shopping",
        "--posting",
        &checking_id,
        "-50.00",
        "AUD",
        "--posting",
        &expenses_id,
        "50.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_transaction_warning() {
    // Amending a transaction's date to fall outside the account's declared
    // life warns the same way a create does.
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    ctx.command()
        .args([
            "account",
            "set-opened-on",
            &checking_id,
            "--on",
            "2024-01-01",
        ])
        .output()
        .expect("set-opened-on");

    let add_out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2024-03-01",
            "--description",
            "Grocery shopping",
            "--posting",
            &checking_id,
            "-50.00",
            "AUD",
            "--posting",
            &expenses_id,
            "50.00",
            "AUD",
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
    cmd.args(["transaction", "edit", &tx_id, "--date", "2020-01-01"]);
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
        &checking_id,
        "-50.00",
        "AUD",
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
            &checking_id,
            "-10.00",
            "AUD",
            "--posting",
            &expenses_id,
            "10.00",
            "AUD",
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
fn edit_description() {
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
            "Original desc",
            "--posting",
            &checking_id,
            "-20.00",
            "AUD",
            "--posting",
            &expenses_id,
            "20.00",
            "AUD",
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
    cmd.args([
        "transaction",
        "edit",
        &tx_id,
        "--description",
        "Edited desc",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_date_only() {
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
            &checking_id,
            "-10.00",
            "AUD",
            "--posting",
            &expenses_id,
            "10.00",
            "AUD",
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
    cmd.args(["transaction", "edit", &tx_id, "--date", "2026-04-15"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_after_reversal_succeeds() {
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
            "To void then edit",
            "--posting",
            &checking_id,
            "-10.00",
            "AUD",
            "--posting",
            &expenses_id,
            "10.00",
            "AUD",
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
        "edit",
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
    for token in [
        "--posting",
        checking,
        "-5.00",
        "AUD",
        "--posting",
        expenses,
        "5.00",
        "AUD",
    ] {
        args.push(token.to_owned());
    }

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
            (
                "invoice".to_owned(),
                serde_json::json!({ "number": "1502" })
            ),
            (
                "reimbursed".to_owned(),
                serde_json::json!({ "boolean": false })
            ),
            (
                "due".to_owned(),
                serde_json::json!({ "date": "2026-04-15" })
            ),
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
        vec![(
            "invoice".to_owned(),
            serde_json::json!({ "number": "1600" })
        )],
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
fn edit_replaces_one_key_in_place_and_leaves_the_others() {
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
            "edit",
            &tx_id,
            "--meta",
            "payee=Other Grocer",
        ])
        .output()
        .expect("edit");

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
fn edit_clear_meta_removes_every_entry_under_the_key() {
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
        .args(["transaction", "edit", &tx_id, "--clear-meta", "note"])
        .output()
        .expect("edit");

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
fn edit_rejects_setting_and_clearing_one_key() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(&ctx, &checking_id, &expenses_id, &["--meta", "note=first"]);

    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        &tx_id,
        "--meta",
        "note=second",
        "--clear-meta",
        "note",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// Registers `reimburse-to` as an account key and returns a transaction
/// written under it.
///
/// `MetaType::Account` is never inferred, so the only route to an account key
/// is a write followed by `meta retype`.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn account_key_transaction(ctx: &TestContext, checking: &str, expenses: &str) -> String {
    let tx_id = add_with(
        ctx,
        checking,
        expenses,
        &["--meta", "reimburse-to=placeholder"],
    );
    ctx.command()
        .args(["meta", "retype", "reimburse-to", "account"])
        .output()
        .expect("retype");
    tx_id
}

#[test]
fn an_account_key_resolves_a_path_to_the_account_it_names() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = account_key_transaction(&ctx, &checking_id, &expenses_id);

    ctx.command()
        .args([
            "transaction",
            "edit",
            &tx_id,
            "--meta",
            "reimburse-to=Assets:Checking",
        ])
        .output()
        .expect("edit");

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![(
            "reimburse-to".to_owned(),
            serde_json::json!({ "account": checking_id })
        )],
        "coercion holds no database, so the CLI is what turns a path into an id"
    );
}

#[test]
fn an_account_key_flags_a_path_naming_no_account() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = account_key_transaction(&ctx, &checking_id, &expenses_id);

    ctx.command()
        .args([
            "transaction",
            "edit",
            &tx_id,
            "--meta",
            "reimburse-to=Assets:NoSuchAccount",
        ])
        .output()
        .expect("edit");

    let json: serde_json::Value =
        serde_json::from_slice(&reload(&ctx, &tx_id)).expect("valid JSON");
    let entry = json
        .get("metadata")
        .and_then(serde_json::Value::as_array)
        .and_then(|entries| entries.first())
        .expect("one entry");
    assert_eq!(
        entry.get("value"),
        Some(&serde_json::json!({ "text": "Assets:NoSuchAccount" })),
        "a path naming nothing is kept verbatim rather than rejected"
    );
    assert_eq!(
        entry.get("mismatched"),
        Some(&serde_json::Value::Bool(true)),
        "and the store flags it, so the CLI can mark it for repair"
    );
}

#[test]
fn list_renders_an_account_entry_as_its_path() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = account_key_transaction(&ctx, &checking_id, &expenses_id);

    ctx.command()
        .args([
            "transaction",
            "edit",
            &tx_id,
            "--meta",
            "reimburse-to=Assets:Checking",
        ])
        .output()
        .expect("edit");

    let out = ctx
        .command()
        .args(["transaction", "list"])
        .output()
        .expect("list");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("reimburse-to=Assets:Checking"),
        "an account entry reads as the path, not the opaque id, got {stdout}"
    );
    assert!(
        !stdout.contains(&checking_id),
        "the id is what the store holds, not what a person reads, got {stdout}"
    );
}

#[test]
fn edit_appends_a_key_the_transaction_did_not_carry() {
    let ctx = TestContext::new();
    let (checking_id, expenses_id) = setup_accounts(&ctx);
    let tx_id = add_with(
        &ctx,
        &checking_id,
        &expenses_id,
        &["--meta", "payee=Generic Grocer"],
    );

    ctx.command()
        .args(["transaction", "edit", &tx_id, "--meta", "invoice=1502"])
        .output()
        .expect("edit");

    assert_eq!(
        metadata_of(&reload(&ctx, &tx_id)),
        vec![
            (
                "payee".to_owned(),
                serde_json::json!({ "text": "Generic Grocer" })
            ),
            (
                "invoice".to_owned(),
                serde_json::json!({ "number": "1502" })
            ),
        ],
        "a key the transaction did not hold goes after the ones it did"
    );
}

#[test]
fn add_accepts_account_paths() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        "Assets:Checking",
        "-50.00",
        "AUD",
        "--posting",
        "Expenses:Groceries",
        "50.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

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

/// The `field` of each posting in a transaction's JSON.
fn posting_fields(tx: &serde_json::Value, field: &str) -> Vec<String> {
    tx.get("postings")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|p| p.get(field).and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

/// Adds the unbalanced grocery transaction the edit tests start from.
fn add_groceries(ctx: &TestContext) -> serde_json::Value {
    json_of(ctx.command().args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        "Assets:Checking",
        "-50.00",
        "AUD",
        "--posting",
        "Expenses:Groceries",
        "30.00",
        "AUD",
    ]))
}

/// Creates `Expenses:Household` and returns its ID.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn create_household(ctx: &TestContext) -> String {
    let out = ctx
        .command()
        .args(["--json", "account", "create", "Expenses:Household"])
        .output()
        .expect("create household");
    parse_account_id(&out.stdout)
}

#[test]
fn edit_adds_a_posting_by_selector() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    create_household(&ctx);
    add_groceries(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-01",
        "-50",
        "--add",
        "Expenses:Household",
        "20.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);

    let listed = json_of(ctx.command().args(["--json", "transaction", "list"]));
    let first = listed.get(0).cloned().unwrap_or_default();
    assert_eq!(posting_fields(&first, "id").len(), 3);
}

#[test]
fn edit_set_posting_keeps_the_posting_id() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let household = create_household(&ctx);
    let added = add_groceries(&ctx);
    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-01",
        "--set",
        "Expenses:Groceries",
        "--account",
        "Expenses:Household",
        "--amount",
        "50.00",
        "AUD",
    ]));
    assert_eq!(posting_fields(&edited, "id"), posting_fields(&added, "id"));
    assert_eq!(
        posting_fields(&edited, "account_id").get(1),
        Some(&household)
    );
}

#[test]
fn edit_by_transaction_id_removes_a_posting() {
    let ctx = TestContext::new();
    let (_checking, groceries) = setup_accounts(&ctx);
    let household = create_household(&ctx);
    let added = add_groceries(&ctx);
    let id = added
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &id,
        "--add",
        "Expenses:Household",
        "50.00",
        "AUD",
        "--remove",
        "Expenses:Groceries",
    ]));
    let account_ids = posting_fields(&edited, "account_id");
    assert_eq!(posting_fields(&edited, "id").len(), 2);
    assert!(
        !account_ids.contains(&groceries),
        "Groceries was removed: {account_ids:?}"
    );
    assert!(
        account_ids.contains(&household),
        "Household was added: {account_ids:?}"
    );
}

#[test]
fn edit_with_no_matching_transaction() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    add_groceries(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-02",
        "--add",
        "Expenses:Groceries",
        "20.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_with_two_matching_transactions() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    add_groceries(&ctx);
    add_groceries(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-01",
        "--add",
        "Expenses:Groceries",
        "20.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_several_postings_match_error() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let added = json_of(ctx.command().args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        "Expenses:Groceries",
        "10.00",
        "AUD",
        "--posting",
        "Expenses:Groceries",
        "10.00",
        "AUD",
        "--posting",
        "Assets:Checking",
        "-20.00",
        "AUD",
    ]));
    let id = added
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        &id,
        "--remove",
        "Expenses:Groceries",
        "10.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_without_an_operation() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    add_groceries(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn edit_rejects_a_posting_named_twice() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    create_household(&ctx);
    add_groceries(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-01",
        "--set",
        "Expenses:Groceries",
        "--account",
        "Expenses:Household",
        "--remove",
        "Expenses:Groceries",
    ]);
    cmd_snapshot!(ctx, &mut cmd);

    let listed = json_of(ctx.command().args(["--json", "transaction", "list"]));
    let first = listed.get(0).cloned().unwrap_or_default();
    assert_eq!(posting_fields(&first, "id").len(), 2, "nothing was written");
}

// MARK: edit end to end on an imported fixture
//
// `transaction edit` has no CLI surface for import provenance or the event
// log, so these tests seed a source reference and read events back through
// bc-core directly, against the same SQLite file the CLI subprocess just
// wrote. This is the most direct fixture available: no first-party importer
// runs without a pre-built WASM plugin (`bc-plugins`'s
// `wasm32-wasip2`-only, environment-dependent problem noted in the repo's
// `CLAUDE.md`), so a native import is not an option in this test binary.

/// Opens the CLI test's own SQLite file in-process.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
async fn open_pool(ctx: &TestContext) -> sqlx::SqlitePool {
    bc_core::open_db_at(&ctx.db_path)
        .await
        .expect("open the test database")
}

/// Attaches a fake import source reference to `posting_id`, as if an
/// importer had created that leg from a statement row.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
async fn attach_import_source(
    pool: &sqlx::SqlitePool,
    tx_id: &str,
    posting_id: &str,
    account_id: &str,
    date: jiff::civil::Date,
    amount: bc_models::Amount,
) {
    let sources = bc_core::SourceService::new(pool.clone());
    let source_ref = bc_models::SourceRef::builder()
        .id(bc_models::SourceRefId::new())
        .transaction_id(bc_models::TransactionId::from_str(tx_id).expect("valid transaction id"))
        .posting_id(Some(
            bc_models::PostingId::from_str(posting_id).expect("valid posting id"),
        ))
        .account_id(bc_models::AccountId::from_str(account_id).expect("valid account id"))
        .date(date)
        .narration("Woolworths")
        .amount(Some(amount))
        .reference(None)
        .occurrence(0)
        .import_batch_id(None)
        .owns_posting(true)
        .created_at(jiff::Timestamp::now())
        .build();
    sources
        .attach(&source_ref)
        .await
        .expect("attach fixture source reference");
}

/// Loads the event trail for `tx_id`, dropping timestamps: these assertions
/// care about which events fired and their payload, not when.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
async fn events_of(pool: &sqlx::SqlitePool, tx_id: &str) -> Vec<bc_core::Event> {
    let transactions = bc_core::TransactionService::new(pool.clone());
    let id = bc_models::TransactionId::from_str(tx_id).expect("valid transaction id");
    transactions
        .audit_trail(&id)
        .await
        .expect("load audit trail")
        .into_iter()
        .map(|(_, event)| event)
        .collect()
}

/// Loads the source references stored for `tx_id`.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
async fn source_refs_of(pool: &sqlx::SqlitePool, tx_id: &str) -> Vec<bc_models::SourceRef> {
    let sources = bc_core::SourceService::new(pool.clone());
    let id = bc_models::TransactionId::from_str(tx_id).expect("valid transaction id");
    sources
        .list_for_transaction(&id)
        .await
        .expect("list source refs")
}

/// Adds the balanced grocery transaction the imported-fixture tests start
/// from and returns its `--json` payload.
fn add_balanced_groceries(ctx: &TestContext) -> serde_json::Value {
    json_of(ctx.command().args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "Grocery shopping",
        "--posting",
        "Assets:Checking",
        "-50.00",
        "AUD",
        "--posting",
        "Expenses:Groceries",
        "50.00",
        "AUD",
    ]))
}

#[tokio::test]
async fn edit_set_posting_on_an_imported_leg_keeps_id_and_reference() {
    let ctx = TestContext::new();
    let (_checking, groceries) = setup_accounts(&ctx);
    let household = create_household(&ctx);
    let added = add_balanced_groceries(&ctx);
    let tx_id = added
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let groceries_posting = posting_fields(&added, "id")
        .get(1)
        .cloned()
        .unwrap_or_default();

    let pool = open_pool(&ctx).await;
    attach_import_source(
        &pool,
        &tx_id,
        &groceries_posting,
        &groceries,
        jiff::civil::date(2026, 3, 1),
        bc_models::Amount::new(dec!(50.00), bc_models::CommodityCode::new("AUD")),
    )
    .await;

    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &tx_id,
        "--set",
        &groceries_posting,
        "--account",
        "Expenses:Household",
        "--amount",
        "45.00",
        "AUD",
    ]));

    // The posting keeps its ID and moves to Household at its new amount.
    let edited_ids = posting_fields(&edited, "id");
    let edited_accounts = posting_fields(&edited, "account_id");
    let idx = edited_ids
        .iter()
        .position(|id| *id == groceries_posting)
        .expect("posting id survives the edit");
    assert_eq!(
        edited_accounts.get(idx),
        Some(&household),
        "got: {edited_accounts:?}"
    );

    // The event log records both a recategorise and an amount change for
    // this exact posting.
    let posting_id = bc_models::PostingId::from_str(&groceries_posting).expect("valid posting id");
    let events = events_of(&pool, &tx_id).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            bc_core::Event::PostingRecategorised { posting_id: p, .. } if *p == posting_id
        )),
        "expected PostingRecategorised, got: {events:?}"
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            bc_core::Event::PostingAmountChanged { posting_id: p, .. } if *p == posting_id
        )),
        "expected PostingAmountChanged, got: {events:?}"
    );

    // The import reference survives, still pointing at the same posting and
    // still recording the document's own (pre-recategorise) account.
    let refs = source_refs_of(&pool, &tx_id).await;
    let source_ref = refs
        .iter()
        .find(|r| r.posting_id() == Some(&posting_id))
        .expect("source reference survives the edit");
    assert_eq!(
        source_ref.account_id().to_string(),
        groceries,
        "reference keeps the document's own account"
    );
}

#[tokio::test]
async fn edit_add_posting_records_posting_added() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let household = create_household(&ctx);
    let added = add_groceries(&ctx);
    let tx_id = added
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();

    json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &tx_id,
        "--add",
        "Expenses:Household",
        "20.00",
        "AUD",
    ]));

    let pool = open_pool(&ctx).await;
    let household_id = bc_models::AccountId::from_str(&household).expect("valid account id");
    let events = events_of(&pool, &tx_id).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            bc_core::Event::PostingAdded { account, .. } if *account == household_id
        )),
        "expected PostingAdded for Household, got: {events:?}"
    );
}

#[tokio::test]
async fn edit_records_posting_tag_changes_and_whole_new_legs() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let household = create_household(&ctx);
    let added = add_groceries(&ctx);
    let tx_id = id_of(&added);
    let groceries_posting = posting_fields(&added, "id")
        .get(1)
        .cloned()
        .unwrap_or_default();

    json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &tx_id,
        "--set",
        "Expenses:Groceries",
        "--tag",
        "person:a",
    ]));
    json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &tx_id,
        "--add",
        "Expenses:Household",
        "20.00",
        "AUD",
        "--meta",
        "note=shared",
        "--tag",
        "person:b",
    ]));

    let pool = open_pool(&ctx).await;
    let events = events_of(&pool, &tx_id).await;
    let posting_id = bc_models::PostingId::from_str(&groceries_posting).expect("valid posting id");
    let person_a = bc_models::TagId::from_str(&tag_id(&ctx, "person:a")).expect("valid tag id");
    assert!(
        events.iter().any(|e| matches!(
            e,
            bc_core::Event::PostingTagsChanged { posting_id: p, added: tagged, removed, .. }
                if *p == posting_id && *tagged == [person_a.clone()] && removed.is_empty()
        )),
        "expected PostingTagsChanged adding person:a, got: {events:?}"
    );

    let household_id = bc_models::AccountId::from_str(&household).expect("valid account id");
    let person_b = bc_models::TagId::from_str(&tag_id(&ctx, "person:b")).expect("valid tag id");
    let note = bc_models::MetaEntry::new(
        bc_models::MetaKey::new("note").expect("valid key"),
        bc_models::MetaValue::Text("shared".to_owned()),
    );
    assert!(
        events.iter().any(|e| matches!(
            e,
            bc_core::Event::PostingAdded { account, metadata, tag_ids, .. }
                if *account == household_id
                    && metadata.entries() == [note.clone()]
                    && *tag_ids == [person_b.clone()]
        )),
        "expected PostingAdded carrying note=shared and person:b, got: {events:?}"
    );
}

#[rstest::rstest]
#[case::date(&["--date", "2026-03-05"], "2026-03-05", "Grocery shopping")]
#[case::description(&["--description", "Weekly shop"], "2026-03-01", "Weekly shop")]
#[case::both(
    &["--date", "2026-03-05", "--description", "Weekly shop"],
    "2026-03-05",
    "Weekly shop"
)]
fn edit_changes_the_date_and_description(
    #[case] flags: &[&str],
    #[case] date: &str,
    #[case] description: &str,
) {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let tx_id = id_of(&add_groceries(&ctx));

    let edited = json_of(
        ctx.command()
            .args(["--json", "transaction", "edit", &tx_id])
            .args(flags),
    );

    assert_eq!(
        edited.get("date").and_then(serde_json::Value::as_str),
        Some(date)
    );
    assert_eq!(
        edited
            .get("description")
            .and_then(serde_json::Value::as_str),
        Some(description)
    );
}

#[tokio::test]
async fn edit_date_records_transaction_date_changed() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let tx_id = id_of(&add_groceries(&ctx));

    json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &tx_id,
        "--date",
        "2026-03-05",
    ]));

    let pool = open_pool(&ctx).await;
    let events = events_of(&pool, &tx_id).await;
    assert!(
        events.iter().any(|e| matches!(
            e,
            bc_core::Event::TransactionDateChanged { from, to, .. }
                if *from == jiff::civil::date(2026, 3, 1) && *to == jiff::civil::date(2026, 3, 5)
        )),
        "expected TransactionDateChanged 2026-03-01 -> 2026-03-05, got: {events:?}"
    );
}

#[tokio::test]
async fn edit_remove_posting_on_an_imported_leg_warns_and_tombstones() {
    let ctx = TestContext::new();
    let (_checking, groceries) = setup_accounts(&ctx);
    let added = add_balanced_groceries(&ctx);
    let tx_id = added
        .get("id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let groceries_posting = posting_fields(&added, "id")
        .get(1)
        .cloned()
        .unwrap_or_default();

    let pool = open_pool(&ctx).await;
    attach_import_source(
        &pool,
        &tx_id,
        &groceries_posting,
        &groceries,
        jiff::civil::date(2026, 3, 1),
        bc_models::Amount::new(dec!(50.00), bc_models::CommodityCode::new("AUD")),
    )
    .await;

    let output = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "edit",
            &tx_id,
            "--remove",
            &groceries_posting,
        ])
        .output()
        .expect("command runs");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains("came from an import"),
        "expected an import warning, got: {stderr}"
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
    let warnings_count = json
        .get("warnings")
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    assert!(warnings_count > 0, "expected a non-empty warnings array");

    let refs = source_refs_of(&pool, &tx_id).await;
    let source_ref = refs.first().expect("source reference row still present");
    assert!(
        source_ref.posting_id().is_none(),
        "reference is tombstoned, not deleted"
    );
}

/// One `--json edit` payload, snapshotted with `insta`. IDs and timestamps
/// are redacted by `TestContext`'s filters, the same as every other JSON
/// snapshot in this suite.
#[test]
fn edit_json_output_snapshot() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    create_household(&ctx);
    add_groceries(&ctx);
    let mut cmd = ctx.command();
    cmd.args([
        "--json",
        "transaction",
        "edit",
        "--find",
        "Assets:Checking",
        "2026-03-01",
        "--add",
        "Expenses:Household",
        "20.00",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

/// The paths `tag list --json` reports.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn tag_paths(ctx: &TestContext) -> Vec<String> {
    let rows = json_of(ctx.command().args(["--json", "tag", "list"]));
    rows.as_array()
        .expect("rows")
        .iter()
        .filter_map(|row| row.get(1).and_then(serde_json::Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

/// The ID `tag list --json` reports for `path`.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn tag_id(ctx: &TestContext, path: &str) -> String {
    let rows = json_of(ctx.command().args(["--json", "tag", "list"]));
    rows.as_array()
        .expect("rows")
        .iter()
        .find(|row| row.get(1).and_then(serde_json::Value::as_str) == Some(path))
        .and_then(|row| row.get(0))
        .and_then(serde_json::Value::as_str)
        .expect("tag listed")
        .to_owned()
}

#[test]
#[expect(
    clippy::indexing_slicing,
    reason = "indexing JSON yields null for a missing field, which fails the assertion"
)]
fn add_sets_posting_metadata_and_creates_a_tag() {
    let ctx = TestContext::new();
    let (checking, expenses) = setup_accounts(&ctx);
    let out = ctx
        .command()
        .args([
            "--json",
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Groceries",
            "--meta",
            "payee=Example Market",
            "--tag",
            "trip:example",
            "--posting",
            &checking,
            "-50.00",
            "AUD",
            "--posting",
            &expenses,
            "50.00",
            "AUD",
            "--meta",
            "note=Paid by A",
            "--tag",
            "person:a",
        ])
        .output()
        .expect("add");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("warning: created tag 'person:a'"),
        "{stderr}"
    );
    assert!(
        stderr.contains("warning: created tag 'trip:example'"),
        "{stderr}"
    );

    // `from_slice` refuses trailing content, so stdout is one JSON object.
    let tx: serde_json::Value =
        serde_json::from_slice(&out.stdout).expect("stdout is one JSON object");
    let warnings = tx["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str() == Some("created tag 'person:a'")),
        "{warnings:?}"
    );
    assert_eq!(tx["metadata"][0]["key"], "payee");
    assert_eq!(tx["tag_ids"].as_array().map(Vec::len), Some(1));

    let legs = tx["postings"].as_array().expect("postings");
    let grocery = legs
        .iter()
        .find(|p| p["account_id"] == expenses.as_str())
        .expect("grocery leg");
    assert_eq!(grocery["metadata"][0]["key"], "note");
    assert_eq!(grocery["metadata"][0]["value"]["text"], "Paid by A");
    assert_eq!(grocery["tag_ids"].as_array().map(Vec::len), Some(1));
    let checking_leg = legs
        .iter()
        .find(|p| p["account_id"] == checking.as_str())
        .expect("checking leg");
    assert_eq!(checking_leg["metadata"].as_array().map(Vec::len), Some(0));
    assert_eq!(checking_leg["tag_ids"].as_array().map(Vec::len), Some(0));
}

#[test]
fn one_new_tag_named_twice_is_created_once() {
    let ctx = TestContext::new();
    let (checking, expenses) = setup_accounts(&ctx);
    let out = ctx
        .command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Split",
            "--posting",
            &checking,
            "-50.00",
            "AUD",
            "--tag",
            "person:a",
            "--posting",
            &expenses,
            "50.00",
            "AUD",
            "--tag",
            "person:a",
        ])
        .output()
        .expect("add");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert_eq!(
        stderr.matches("created tag 'person:a'").count(),
        1,
        "{stderr}"
    );
    let paths = tag_paths(&ctx);
    assert_eq!(
        paths.iter().filter(|p| *p == "person:a").count(),
        1,
        "{paths:?}"
    );
}

#[test]
#[expect(
    clippy::indexing_slicing,
    reason = "indexing JSON yields null for a missing field, which fails the assertion"
)]
fn add_sets_a_spread() {
    let ctx = TestContext::new();
    let (checking, expenses) = setup_accounts(&ctx);
    let tx = json_of(ctx.command().args([
        "--json",
        "transaction",
        "add",
        "--date",
        "2026-01-01",
        "--description",
        "Insurance",
        "--posting",
        &checking,
        "-1200.00",
        "AUD",
        "--posting",
        &expenses,
        "1200.00",
        "AUD",
        "--spread",
        "2026-01-01",
        "2026-12-31",
    ]));
    let leg = tx["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .find(|p| p["account_id"] == expenses.as_str())
        .expect("leg");
    assert_eq!(leg["spread_from"], "2026-01-01");
    assert_eq!(leg["spread_until"], "2026-12-31");
}

#[test]
fn add_rejects_a_posting_flag_before_any_posting() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "add",
        "--date",
        "2026-03-01",
        "--description",
        "X",
        "--cost",
        "1",
        "AUD",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[rstest::rstest]
#[case::path("Expenses:NoSuchAccount".to_owned())]
#[case::id(bc_models::AccountId::new().to_string())]
fn a_refused_add_creates_no_tag_for_an_unknown_account(#[case] unknown: String) {
    let ctx = TestContext::new();
    let (checking, _) = setup_accounts(&ctx);
    let out = ctx
        .command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Typo",
            "--posting",
            &checking,
            "-5.00",
            "AUD",
            "--posting",
            &unknown,
            "5.00",
            "AUD",
            "--tag",
            "x:new",
        ])
        .output()
        .expect("add");
    assert!(!out.status.success(), "the add is refused");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains(&format!("no account '{unknown}'")),
        "{stderr}"
    );
    let paths = tag_paths(&ctx);
    assert!(!paths.iter().any(|p| p == "x:new"), "{paths:?}");
}

#[test]
fn a_refused_add_creates_no_tag_for_a_lot_date_without_a_cost() {
    let ctx = TestContext::new();
    let (checking, _) = setup_accounts(&ctx);
    let brokerage = setup_brokerage(&ctx);
    let out = ctx
        .command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Sell shares",
            "--posting",
            &checking,
            "300",
            "AUD",
            "--posting",
            &brokerage,
            "-2",
            "AAPL",
            "--lot-date",
            "2024-03-01",
            "--tag",
            "x:new",
        ])
        .output()
        .expect("add");
    assert!(!out.status.success(), "the add is refused");
    let paths = tag_paths(&ctx);
    assert!(!paths.iter().any(|p| p == "x:new"), "{paths:?}");
}

#[test]
fn a_refused_add_creates_no_tag_for_two_elided_postings() {
    let ctx = TestContext::new();
    let (checking, expenses) = setup_accounts(&ctx);
    let out = ctx
        .command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Groceries",
            "--posting",
            &checking,
            "--posting",
            &expenses,
            "--tag",
            "x:new",
        ])
        .output()
        .expect("add");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the add is refused");
    assert!(stderr.contains("two or more elided postings"), "{stderr}");
    let paths = tag_paths(&ctx);
    assert!(!paths.iter().any(|p| p == "x:new"), "{paths:?}");
}

#[rstest::rstest]
#[case::not_a_number(
    "abc",
    "AUD",
    "--posting Assets:Checking abc AUD: invalid amount 'abc'"
)]
#[case::no_commodity("5", "", "amount '5' has no commodity")]
fn add_names_the_posting_with_a_malformed_amount(
    #[case] value: &str,
    #[case] code: &str,
    #[case] expected: &str,
) {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let out = ctx
        .command()
        .args([
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Groceries",
            "--posting",
            "Assets:Checking",
            value,
            code,
            "--posting",
            "Expenses:Groceries",
        ])
        .output()
        .expect("add");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the add is refused");
    assert!(stderr.contains(expected), "{stderr}");
}

// MARK: scoped edit flags

/// The `id` of a transaction's JSON.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn id_of(tx: &serde_json::Value) -> String {
    tx.get("id")
        .and_then(serde_json::Value::as_str)
        .expect("id")
        .to_owned()
}

#[test]
#[expect(
    clippy::indexing_slicing,
    reason = "indexing JSON yields null for a missing field, which fails the assertion"
)]
fn edit_tags_one_of_two_new_legs_on_one_account() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let household = create_household(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &id,
        "--add",
        &household,
        "5.00",
        "AUD",
        "--add",
        &household,
        "2.00",
        "AUD",
        "--tag",
        "person:a",
    ]));
    let tagged: Vec<&str> = edited["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .filter(|p| p["account_id"] == household.as_str())
        .filter(|p| p["tag_ids"].as_array().is_some_and(|t| !t.is_empty()))
        .filter_map(|p| p["amount"]["value"].as_str())
        .collect();
    assert_eq!(tagged, vec!["2.00"]);
}

#[test]
#[expect(
    clippy::indexing_slicing,
    reason = "indexing JSON yields null for a missing field, which fails the assertion"
)]
fn set_changes_only_what_it_names() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    ctx.command()
        .args([
            "transaction",
            "edit",
            &id,
            "--set",
            "Expenses:Groceries",
            "--meta",
            "note=first",
            "--meta",
            "receipt=A1",
            "--tag",
            "person:a",
            "--tag",
            "person:b",
        ])
        .assert()
        .success();
    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &id,
        "--set",
        "Expenses:Groceries",
        "--clear-meta",
        "note",
        "--untag",
        "person:a",
        "--untag",
        "never:made",
    ]));
    let leg = edited["postings"]
        .as_array()
        .expect("postings")
        .iter()
        .find(|p| p["metadata"].as_array().is_some_and(|m| !m.is_empty()))
        .expect("leg");
    assert_eq!(leg["metadata"].as_array().map(Vec::len), Some(1));
    assert_eq!(leg["metadata"][0]["key"], "receipt");
    assert_eq!(
        leg["tag_ids"],
        serde_json::json!([tag_id(&ctx, "person:b")]),
        "person:b is the tag kept"
    );
    assert_eq!(leg["amount"]["value"], "30.00", "the amount is kept");
    let paths = tag_paths(&ctx);
    assert!(
        !paths.iter().any(|p| p == "never:made"),
        "--untag of a missing path creates nothing: {paths:?}"
    );
}

#[test]
fn edit_untag_of_an_unknown_tag_id_is_an_error() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let unknown = bc_models::TagId::new().to_string();
    let out = ctx
        .command()
        .args([
            "transaction",
            "edit",
            &id,
            "--set",
            "Expenses:Groceries",
            "--untag",
            &unknown,
        ])
        .output()
        .expect("edit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the edit is refused");
    assert!(
        stderr.contains(&format!("no tag has the ID '{unknown}'")),
        "{stderr}"
    );
}

#[test]
#[expect(
    clippy::indexing_slicing,
    reason = "indexing JSON yields null for a missing field, which fails the assertion"
)]
fn edit_sets_transaction_tags_and_metadata() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let edited = json_of(ctx.command().args([
        "--json",
        "transaction",
        "edit",
        &id,
        "--meta",
        "invoice=1502",
        "--tag",
        "trip:example",
    ]));
    assert_eq!(edited["tag_ids"].as_array().map(Vec::len), Some(1));
    assert!(edited["metadata"].to_string().contains("1502"));
    let warnings = edited["warnings"].as_array().expect("warnings array");
    assert!(
        warnings
            .iter()
            .any(|w| w.as_str() == Some("created tag 'trip:example'")),
        "{warnings:?}"
    );
}

#[test]
fn edit_refuses_one_posting_opened_twice() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        &id,
        "--set",
        "Expenses:Groceries",
        "--tag",
        "person:a",
        "--remove",
        "Expenses:Groceries",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
    let paths = tag_paths(&ctx);
    assert!(
        !paths.iter().any(|p| p == "person:a"),
        "a refused edit creates no tag: {paths:?}"
    );
}

#[test]
fn edit_refuses_one_posting_set_twice() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        &id,
        "--set",
        "Expenses:Groceries",
        "--tag",
        "person:a",
        "--set",
        "Expenses:Groceries",
        "30.00",
        "AUD",
        "--meta",
        "note=x",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn set_without_a_modifier_is_refused() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let mut cmd = ctx.command();
    cmd.args(["transaction", "edit", &id, "--set", "Expenses:Groceries"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[rstest::rstest]
#[case::unknown_new_account(&["--add", "Expenses:NoSuchAccount", "5.00", "AUD", "--tag", "x:new"])]
#[case::unknown_set_account(&["--set", "Expenses:Groceries", "--account", "Expenses:NoSuchAccount", "--tag", "x:new"])]
#[case::unknown_selector(&["--set", "Expenses:NoSuchAccount", "--tag", "x:new"])]
#[case::lot_date_without_a_cost(&["--set", "Expenses:Groceries", "--lot-date", "2026-03-01", "--tag", "x:new"])]
#[case::two_elided(&["--add", "Expenses:Groceries", "--add", "Assets:Checking", "--tag", "x:new"])]
#[case::no_postings(&["--remove", "Assets:Checking", "--remove", "Expenses:Groceries", "--tag", "x:new"])]
#[case::lone_elided(&["--remove", "Assets:Checking", "--remove", "Expenses:Groceries", "--add", "Expenses:Groceries", "--tag", "x:new"])]
fn a_refused_edit_creates_no_tag(#[case] extra: &[&str]) {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let id = id_of(&add_groceries(&ctx));
    let out = ctx
        .command()
        .args(["transaction", "edit", &id])
        .args(extra)
        .output()
        .expect("edit");
    assert!(!out.status.success(), "the edit is refused");
    let paths = tag_paths(&ctx);
    assert!(!paths.iter().any(|p| p == "x:new"), "{paths:?}");
}

#[test]
fn edit_refuses_a_date_after_a_posting_flag() {
    let ctx = TestContext::new();
    setup_accounts(&ctx);
    let tx = add_groceries(&ctx);
    let id = id_of(&tx);
    let mut cmd = ctx.command();
    cmd.args([
        "transaction",
        "edit",
        &id,
        "--set",
        "Expenses:Groceries",
        "--tag",
        "person:a",
        "--date",
        "2026-04-01",
    ]);
    cmd_snapshot!(ctx, &mut cmd);
}
