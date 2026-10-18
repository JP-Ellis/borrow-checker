//! Integration tests for the `export` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use pretty_assertions::assert_eq;

use crate::cmd_snapshot;
use crate::common::TestContext;

/// The directory `mise run build-plugins` stages the plugins in.
fn plugin_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("plugins")
}

/// A command that finds the built plugins; the harness clears the environment.
fn command(ctx: &TestContext) -> Command {
    let mut cmd = ctx.command();
    cmd.env(
        "BORROW_CHECKER_PLUGIN_DIR",
        plugin_dir().canonicalize().unwrap_or_else(|_| plugin_dir()),
    );
    cmd
}

/// Runs `args` and panics unless the command succeeds.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn run_ok(ctx: &TestContext, args: &[&str]) {
    let output = command(ctx).args(args).output().expect("command ran");
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Seeds two accounts and two dated transactions.
fn seed(ctx: &TestContext) {
    run_ok(ctx, &["account", "create", "Assets:Checking"]);
    run_ok(ctx, &["account", "create", "Expenses:Groceries"]);
    for (date, description, amount) in [
        ("2026-03-01", "Supermarket", "50.00"),
        ("2026-04-02", "Bakery", "12.50"),
    ] {
        let negative = format!("-{amount}");
        run_ok(
            ctx,
            &[
                "transaction",
                "add",
                "--date",
                date,
                "--description",
                description,
                "--posting",
                "Assets:Checking",
                &negative,
                "AUD",
                "--posting",
                "Expenses:Groceries",
                amount,
                "AUD",
            ],
        );
    }
}

/// Replaces the random handle in a `bc` metadata line so text is stable.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn mask_handles(text: &str) -> String {
    let re = regex::Regex::new(r#"(bc: ")[^"]+(")"#).expect("valid regex");
    re.replace_all(text, "${1}[HANDLE]${2}").into_owned()
}

#[test]
fn export_ledger_is_unsupported() {
    let ctx = TestContext::new();
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "ledger", "--level", "archive"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_unknown_format_is_refused() {
    let ctx = TestContext::new();
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "nonesuch", "-o", "x"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_working_writes_file_and_sidecar() {
    let ctx = TestContext::new();
    seed(&ctx);
    let out = ctx.home_dir.path().join("out.beancount");
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "beancount", "-o"])
        .arg(&out);
    cmd_snapshot!(ctx, &mut cmd);

    let text = std::fs::read_to_string(&out).expect("export file");
    insta::assert_snapshot!(mask_handles(&text));

    let sidecar_path = out.with_extension("beancount.bc.json");
    let sidecar: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&sidecar_path).expect("sidecar"))
            .expect("sidecar is JSON");
    let entries = sidecar
        .get("entries")
        .and_then(serde_json::Value::as_array)
        .expect("entries");
    assert_eq!(entries.len(), 2);
    for entry in entries {
        let handle = entry
            .get("handle")
            .and_then(serde_json::Value::as_str)
            .expect("handle");
        assert!(text.contains(handle), "export mentions handle {handle}");
    }
}

#[test]
fn export_working_without_output_is_refused() {
    let ctx = TestContext::new();
    seed(&ctx);
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "beancount"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_working_filters_by_window() {
    let ctx = TestContext::new();
    seed(&ctx);
    let out = ctx.home_dir.path().join("window.beancount");
    run_ok(
        &ctx,
        &[
            "export",
            "--format",
            "beancount",
            "--from",
            "2026-04-01",
            "--until",
            "2026-05-01",
            "-o",
            out.to_str().expect("UTF-8 path"),
        ],
    );
    let text = std::fs::read_to_string(&out).expect("export file");
    assert!(text.contains("Bakery"));
    assert!(!text.contains("Supermarket"));
}

#[test]
fn export_working_filters_by_query() {
    let ctx = TestContext::new();
    seed(&ctx);
    let out = ctx.home_dir.path().join("query.beancount");
    run_ok(
        &ctx,
        &[
            "export",
            "--format",
            "beancount",
            "--query",
            "Supermarket",
            "-o",
            out.to_str().expect("UTF-8 path"),
        ],
    );
    let text = std::fs::read_to_string(&out).expect("export file");
    assert!(text.contains("Supermarket"));
    assert!(!text.contains("Bakery"));
}

#[test]
fn export_with_bad_query_is_refused() {
    let ctx = TestContext::new();
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "beancount", "--query", "(", "-o", "x"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_archive_writes_layout() {
    let ctx = TestContext::new();
    seed(&ctx);
    let dir = ctx.home_dir.path().join("archive");
    let mut cmd = command(&ctx);
    cmd.args([
        "export",
        "--format",
        "beancount",
        "--level",
        "archive",
        "-o",
    ])
    .arg(&dir);
    cmd_snapshot!(ctx, &mut cmd);

    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("archive dir")
        .map(|entry| {
            entry
                .expect("entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    names.sort();
    insta::assert_debug_snapshot!(names);
    let main = std::fs::read_to_string(dir.join("main.bean")).expect("main.bean");
    insta::assert_snapshot!(main);
}

#[test]
fn export_archive_without_output_is_refused_when_it_has_many_files() {
    let ctx = TestContext::new();
    seed(&ctx);
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "beancount", "--level", "archive"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_archive_with_a_filter_warns_about_balances() {
    let ctx = TestContext::new();
    seed(&ctx);
    let dir = ctx.home_dir.path().join("partial");
    let mut cmd = command(&ctx);
    cmd.args([
        "export",
        "--format",
        "beancount",
        "--level",
        "archive",
        "--from",
        "2026-04-01",
        "-o",
    ])
    .arg(&dir);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_unspellable_lists_names() {
    let ctx = TestContext::new();
    run_ok(&ctx, &["account", "create", "Assets:Checking"]);
    run_ok(&ctx, &["account", "create", "Expenses:Groceries"]);
    run_ok(
        &ctx,
        &[
            "transaction",
            "add",
            "--date",
            "2026-03-01",
            "--description",
            "Supermarket",
            "--tag",
            "my tag",
            "--tag",
            "another tag",
            "--posting",
            "Assets:Checking",
            "-5.00",
            "AUD",
            "--posting",
            "Expenses:Groceries",
            "5.00",
            "AUD",
        ],
    );
    let out = ctx.home_dir.path().join("bad.beancount");
    let mut cmd = command(&ctx);
    cmd.args(["export", "--format", "beancount", "-o"])
        .arg(&out);
    cmd_snapshot!(ctx, &mut cmd);
    assert!(!out.exists(), "nothing is written on failure");
}

#[test]
fn export_archive_passes_bean_check() {
    let Ok(bean_check) = which("bean-check") else {
        eprintln!("skipping: bean-check is not on PATH");
        return;
    };
    let ctx = TestContext::new();
    seed(&ctx);
    let dir = ctx.home_dir.path().join("checked");
    run_ok(
        &ctx,
        &[
            "export",
            "--format",
            "beancount",
            "--level",
            "archive",
            "-o",
            dir.to_str().expect("UTF-8 path"),
        ],
    );
    let output = StdCommand::new(bean_check)
        .arg(dir.join("main.bean"))
        .output()
        .expect("bean-check ran");
    assert!(
        output.status.success(),
        "bean-check failed:\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Finds `name` on `PATH`.
fn which(name: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
        .ok_or(())
}
