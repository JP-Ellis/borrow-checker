//! Integration tests for `transaction apply`.
//!
//! Each test exports a seeded ledger at working level, edits the file text
//! and applies it. Only the export and apply calls load the beancount plugin;
//! seeding and inspection go through plain `ctx.command()`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::path::Path;
use std::path::PathBuf;

use pretty_assertions::assert_eq;
use pretty_assertions::assert_ne;

use crate::common::TestContext;
use crate::export::command;
use crate::export::run_ok;
use crate::export::seed;

// MARK: Helpers

/// Exports every transaction at working level to `slice.bean` in the home
/// directory, with its sidecar beside it.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn export_working(ctx: &TestContext) -> PathBuf {
    let out = ctx.home_dir.path().join("slice.bean");
    run_ok(
        command(ctx, &["beancount"]),
        &[
            "export",
            "--format",
            "beancount",
            "-o",
            out.to_str().expect("UTF-8 path"),
        ],
    );
    out
}

/// Every handle the export's sidecar records.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn handles(file: &Path) -> Vec<String> {
    let mut sidecar = file.as_os_str().to_owned();
    sidecar.push(".bc.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(sidecar).expect("sidecar"))
            .expect("sidecar is JSON");
    json.get("entries")
        .and_then(serde_json::Value::as_array)
        .expect("entries")
        .iter()
        .map(|entry| {
            entry
                .get("handle")
                .and_then(serde_json::Value::as_str)
                .expect("handle")
                .to_owned()
        })
        .collect()
}

/// Replaces the export's handles and any batch ID in `text`, so output is
/// stable.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn mask(text: &str, handles: &[String]) -> String {
    let mut masked = regex::Regex::new("import_batch_[0-9a-z]{26}")
        .expect("valid regex")
        .replace_all(text, "[BATCH_ID]")
        .into_owned();
    for handle in handles {
        let re = regex::Regex::new(&format!(r"\b{}\b", regex::escape(handle))).expect("regex");
        masked = re.replace_all(&masked, "[HANDLE]").into_owned();
    }
    masked
}

/// Runs `transaction apply` on `file` with `extra` arguments, loading the
/// beancount plugin, and returns its masked output.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn apply(ctx: &TestContext, file: &Path, extra: &[&str]) -> String {
    let handles = handles(file);
    let mut cmd = command(ctx, &["beancount"]);
    cmd.args(["transaction", "apply", file.to_str().expect("UTF-8 path")])
        .args(extra);
    mask(&ctx.run(&mut cmd), &handles)
}

/// Runs a plugin-free command and returns its formatted output.
fn plain(ctx: &TestContext, args: &[&str]) -> String {
    let mut cmd = ctx.command();
    cmd.args(args);
    ctx.run(&mut cmd)
}

/// Rewrites the exported file through `edit`.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn edit_file(file: &Path, edit: impl FnOnce(String) -> String) {
    let text = std::fs::read_to_string(file).expect("export file");
    let edited = edit(text.clone());
    assert_ne!(edited, text, "the edit changed nothing");
    std::fs::write(file, edited).expect("write edited file");
}

/// Splits the Supermarket purchase's groceries leg into two.
fn split_supermarket(text: &str) -> String {
    text.replacen(
        "  Expenses:Groceries                               50.00 AUD\n",
        "  Expenses:Groceries  40.00 AUD\n  Expenses:Household  10.00 AUD\n",
        1,
    )
}

/// A purchase with no handle, which apply creates.
const NEWSAGENT: &str = "\n2026-04-10 txn \"Newsagent\"\n  Assets:Checking  -5.00 AUD\n  Expenses:Groceries  5.00 AUD\n";

/// Seeds the ledger with the household account the split books to.
fn seed_with_household(ctx: &TestContext) {
    seed(ctx);
    run_ok(ctx.command(), &["account", "create", "Expenses:Household"]);
}

// MARK: Tests

#[test]
fn apply_dry_run_reports_and_writes_nothing() {
    let ctx = TestContext::new();
    seed_with_household(&ctx);
    let file = export_working(&ctx);
    edit_file(&file, |exported| {
        let mut text = split_supermarket(&exported);
        text.push_str(NEWSAGENT);
        text
    });
    let transactions = plain(&ctx, &["transaction", "list"]);
    let batches = plain(&ctx, &["import", "list"]);
    let backups = plain(&ctx, &["backup", "list"]);

    insta::assert_snapshot!(apply(&ctx, &file, &["--dry-run"]));

    assert_eq!(plain(&ctx, &["transaction", "list"]), transactions);
    assert_eq!(plain(&ctx, &["import", "list"]), batches);
    assert_eq!(plain(&ctx, &["backup", "list"]), backups);
}

#[test]
fn apply_adds_leg_and_prints_batch() {
    let ctx = TestContext::new();
    seed_with_household(&ctx);
    let file = export_working(&ctx);
    // The split and the new purchase apply; the renamed Bakery purchase is
    // refused.
    edit_file(&file, |text| {
        let mut edited = split_supermarket(&text).replacen("\"Bakery\"", "\"Patisserie\"", 1);
        edited.push_str(NEWSAGENT);
        edited
    });

    insta::assert_snapshot!(apply(&ctx, &file, &[]));

    let ledger = plain(&ctx, &["transaction", "list"]);
    insta::assert_snapshot!("ledger", ledger);
    let backups = plain(&ctx, &["backup", "list"]);
    assert!(backups.contains("pre-import"), "{backups}");

    // A rerun of the same file writes nothing.
    insta::assert_snapshot!("rerun", apply(&ctx, &file, &[]));
    assert_eq!(plain(&ctx, &["transaction", "list"]), ledger);
}

#[test]
fn apply_unknown_account_exits_5_listing_all() {
    let ctx = TestContext::new();
    seed(&ctx);
    let file = export_working(&ctx);
    edit_file(&file, |text| {
        text.replacen(
            "Expenses:Groceries                               50.00 AUD",
            "Expenses:Nonesuch  50.00 AUD",
            1,
        )
        .replacen(
            "Expenses:Groceries                               12.50 AUD",
            "Expenses:Missing  12.50 AUD",
            1,
        )
        .replacen("\"Bakery\"", "\"Bakery\" #nonesuch", 1)
    });
    let transactions = plain(&ctx, &["transaction", "list"]);

    insta::assert_snapshot!(apply(&ctx, &file, &[]));

    assert_eq!(plain(&ctx, &["transaction", "list"]), transactions);
}

#[test]
fn apply_without_sidecar_is_refused() {
    let ctx = TestContext::new();
    let file = ctx.home_dir.path().join("orphan.bean");
    std::fs::write(
        &file,
        "2026-03-01 txn \"Supermarket\"\n  Assets:Checking  -5.00 AUD\n  Expenses:Groceries  5.00 AUD\n",
    )
    .expect("write file");
    let mut cmd = ctx.command();
    cmd.args(["transaction", "apply", file.to_str().expect("UTF-8 path")]);
    crate::cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_then_apply_unchanged_reports_zero_changes() {
    let ctx = TestContext::new();
    seed(&ctx);
    let file = export_working(&ctx);
    let transactions = plain(&ctx, &["transaction", "list"]);

    insta::assert_snapshot!(apply(&ctx, &file, &[]));

    assert_eq!(plain(&ctx, &["transaction", "list"]), transactions);
    let batches = plain(&ctx, &["import", "list"]);
    assert!(!batches.contains("bc-apply"), "{batches}");
}
