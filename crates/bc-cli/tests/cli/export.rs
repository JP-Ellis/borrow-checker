//! Integration tests for the `export` subcommand.
//!
//! The exporter plugins are WASM components that a debug build compiles on
//! every start, so only the `export` calls load them, and each loads just the
//! plugins it needs. Seeding goes through plain `ctx.command()`, which loads
//! none. The plugins come from `mise run build-plugins`.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::path::Path;
use std::path::PathBuf;
use std::process::Command as StdCommand;

use assert_cmd::Command;
use pretty_assertions::assert_eq;
use pretty_assertions::assert_ne;

use crate::cmd_snapshot;
use crate::common::TestContext;

/// Stages the named built plugins in a directory of their own.
///
/// # Panics
///
/// Panics, naming the fix, when a plugin has not been built.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn stage_plugins(ctx: &TestContext, names: &[&str]) -> PathBuf {
    let built = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("target")
        .join("plugins");
    let dir = ctx.home_dir.path().join("plugins");
    std::fs::create_dir_all(&dir).expect("create plugin dir");
    for name in names {
        let source = built.join(format!("{name}.wasm"));
        assert!(
            source.is_file(),
            "the {name} plugin is not built at {}: run `mise run build-plugins`",
            source.display()
        );
        std::fs::copy(&source, dir.join(format!("{name}.wasm"))).expect("stage plugin");
    }
    dir
}

/// A command that loads the named plugins; the harness clears the environment.
fn command(ctx: &TestContext, plugins: &[&str]) -> Command {
    let mut cmd = ctx.command();
    cmd.env("BORROW_CHECKER_PLUGIN_DIR", stage_plugins(ctx, plugins));
    cmd
}

/// Runs `args` through `cmd` and panics unless the command succeeds.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn run_ok(mut cmd: Command, args: &[&str]) {
    let output = cmd.args(args).output().expect("command ran");
    assert!(
        output.status.success(),
        "{args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Runs a beancount export with `args` and returns its formatted output.
fn export(ctx: &TestContext, args: &[&str]) -> String {
    let mut cmd = command(ctx, &["beancount"]);
    cmd.args(["export", "--format", "beancount"]).args(args);
    ctx.run(&mut cmd)
}

/// Creates the two accounts and adds a transaction, without loading plugins.
fn seed_tx(ctx: &TestContext, tags: &[&str], date: &str, description: &str, amount: &str) {
    let negative = format!("-{amount}");
    let mut args = vec![
        "transaction",
        "add",
        "--date",
        date,
        "--description",
        description,
    ];
    for tag in tags {
        args.extend(["--tag", tag]);
    }
    args.extend([
        "--posting",
        "Assets:Checking",
        &negative,
        "AUD",
        "--posting",
        "Expenses:Groceries",
        amount,
        "AUD",
    ]);
    run_ok(ctx.command(), &args);
}

/// Seeds two accounts and two dated transactions.
fn seed(ctx: &TestContext) {
    run_ok(ctx.command(), &["account", "create", "Assets:Checking"]);
    run_ok(ctx.command(), &["account", "create", "Expenses:Groceries"]);
    seed_tx(ctx, &[], "2026-03-01", "Supermarket", "50.00");
    seed_tx(ctx, &[], "2026-04-02", "Bakery", "12.50");
}

/// Replaces the random handle in a `bc` metadata line so text is stable.
#[expect(clippy::expect_used, reason = "test helper — panics are acceptable")]
fn mask_handles(text: &str) -> String {
    let re = regex::Regex::new(r#"(bc: ")[^"]+(")"#).expect("valid regex");
    re.replace_all(text, "${1}[HANDLE]${2}").into_owned()
}

#[test]
fn export_refusals() {
    let ctx = TestContext::new();
    let mut ledger = command(&ctx, &["beancount", "ledger"]);
    ledger.args(["export", "--format", "ledger", "--level", "archive"]);
    insta::assert_snapshot!("ledger_is_unsupported", ctx.run(&mut ledger));

    let mut unknown = command(&ctx, &["beancount"]);
    unknown.args(["export", "--format", "nonesuch", "-o", "x"]);
    insta::assert_snapshot!("unknown_format", ctx.run(&mut unknown));

    insta::assert_snapshot!("bad_query", export(&ctx, &["--query", "(", "-o", "x"]));
}

#[test]
fn export_working_without_output_is_refused() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["export", "--format", "beancount"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn export_working_writes_file_and_sidecar() {
    let ctx = TestContext::new();
    seed(&ctx);
    let out = ctx.home_dir.path().join("out.beancount");
    let out_arg = out.to_str().expect("UTF-8 path");
    insta::assert_snapshot!(export(&ctx, &["-o", out_arg]));

    let text = std::fs::read_to_string(&out).expect("export file");
    insta::assert_snapshot!("file", mask_handles(&text));

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

    // The JSON report lists the sidecar beside the file.
    let json_out = ctx.home_dir.path().join("json.beancount");
    let mut json = command(&ctx, &["beancount"]);
    json.args([
        "--json",
        "export",
        "--format",
        "beancount",
        "-o",
        json_out.to_str().expect("UTF-8 path"),
    ]);
    insta::assert_snapshot!("json_report", ctx.run(&mut json));

    // A date window keeps one transaction.
    let windowed = ctx.home_dir.path().join("window.beancount");
    run_ok(
        command(&ctx, &["beancount"]),
        &[
            "export",
            "--format",
            "beancount",
            "--from",
            "2026-04-01",
            "--until",
            "2026-05-01",
            "-o",
            windowed.to_str().expect("UTF-8 path"),
        ],
    );
    let windowed_text = std::fs::read_to_string(&windowed).expect("windowed file");
    assert!(windowed_text.contains("Bakery"));
    assert!(!windowed_text.contains("Supermarket"));

    // A query keeps the other.
    let queried = ctx.home_dir.path().join("query.beancount");
    run_ok(
        command(&ctx, &["beancount"]),
        &[
            "export",
            "--format",
            "beancount",
            "--query",
            "Supermarket",
            "-o",
            queried.to_str().expect("UTF-8 path"),
        ],
    );
    let queried_text = std::fs::read_to_string(&queried).expect("queried file");
    assert!(queried_text.contains("Supermarket"));
    assert!(!queried_text.contains("Bakery"));
}

#[test]
fn export_period_keeps_its_boundary_days() {
    let ctx = TestContext::new();
    seed(&ctx);
    seed_tx(&ctx, &[], "2026-03-31", "Chemist", "7.00");
    seed_tx(&ctx, &[], "2026-04-01", "Butcher", "20.00");
    seed_tx(&ctx, &[], "2026-04-30", "Greengrocer", "9.00");
    seed_tx(&ctx, &[], "2026-05-01", "Florist", "15.00");
    let out = ctx.home_dir.path().join("april.beancount");
    run_ok(
        command(&ctx, &["beancount"]),
        &[
            "export",
            "--format",
            "beancount",
            "--period",
            "monthly",
            "--date",
            "2026-04-15",
            "-o",
            out.to_str().expect("UTF-8 path"),
        ],
    );
    let text = std::fs::read_to_string(&out).expect("april file");
    for kept in ["Butcher", "Bakery", "Greengrocer"] {
        assert!(text.contains(kept), "April keeps {kept}:\n{text}");
    }
    for dropped in ["Supermarket", "Chemist", "Florist"] {
        assert!(!text.contains(dropped), "April drops {dropped}:\n{text}");
    }
}

#[test]
fn export_custom_period_is_refused() {
    let ctx = TestContext::new();
    let out = ctx.home_dir.path().join("custom.beancount");
    insta::assert_snapshot!(export(
        &ctx,
        &[
            "--period",
            "custom",
            "-o",
            out.to_str().expect("UTF-8 path")
        ],
    ));
    assert!(!out.exists(), "nothing is written");
}

#[test]
fn export_writes_no_file_when_the_sidecar_cannot_be_written() {
    let ctx = TestContext::new();
    seed(&ctx);
    let out = ctx.home_dir.path().join("blocked.beancount");
    std::fs::create_dir_all(out.with_extension("beancount.bc.json"))
        .expect("block the sidecar path");
    let output = export(&ctx, &["--force", "-o", out.to_str().expect("UTF-8 path")]);
    assert!(output.contains("success: false"), "{output}");
    assert!(!out.exists(), "no export file without its sidecar");
}

#[test]
fn export_working_to_a_directory_is_refused() {
    let ctx = TestContext::new();
    seed(&ctx);
    let dir = ctx.home_dir.path().join("existing");
    std::fs::create_dir_all(&dir).expect("create dir");
    let existing = export(&ctx, &["-o", dir.to_str().expect("UTF-8 path")]);
    assert!(existing.contains("is a directory"), "{existing}");
    assert!(!Path::new(&format!("{}.bc.json", dir.display())).exists());

    let slashed = format!(
        "{}{}",
        ctx.home_dir.path().join("new").display(),
        std::path::MAIN_SEPARATOR
    );
    let trailing = export(&ctx, &["-o", &slashed]);
    assert!(trailing.contains("is a directory"), "{trailing}");
    assert!(
        !ctx.home_dir.path().join("new").exists(),
        "nothing is created"
    );
}

#[test]
fn export_refuses_to_overwrite_without_force() {
    let ctx = TestContext::new();
    seed(&ctx);
    let home = ctx.home_dir.path();

    // The working file exists: it and nothing else is named.
    let file = home.join("file.beancount");
    std::fs::write(&file, "kept\n").expect("existing file");
    insta::assert_snapshot!(
        "overwrite_working_file",
        export(&ctx, &["-o", file.to_str().expect("UTF-8 path")])
    );
    assert_eq!(std::fs::read_to_string(&file).expect("file"), "kept\n");
    assert!(!home.join("file.beancount.bc.json").exists(), "no sidecar");

    // The sidecar alone exists: the export file is not written either.
    let lone = home.join("lone.beancount");
    std::fs::write(home.join("lone.beancount.bc.json"), "{}").expect("existing sidecar");
    insta::assert_snapshot!(
        "overwrite_sidecar",
        export(&ctx, &["-o", lone.to_str().expect("UTF-8 path")])
    );
    assert!(!lone.exists(), "no export file");

    // One archive file exists: no file of the archive is written.
    let dir = home.join("archive");
    std::fs::create_dir_all(&dir).expect("archive dir");
    std::fs::write(dir.join("main.bean"), "kept\n").expect("existing main.bean");
    insta::assert_snapshot!(
        "overwrite_archive",
        export(
            &ctx,
            &[
                "--level",
                "archive",
                "-o",
                dir.to_str().expect("UTF-8 path")
            ]
        )
    );
    let names: Vec<_> = std::fs::read_dir(&dir).expect("archive dir").collect();
    assert_eq!(names.len(), 1, "only the existing file is there");

    // --force overwrites every one.
    run_ok(
        command(&ctx, &["beancount"]),
        &[
            "export",
            "--format",
            "beancount",
            "--force",
            "-o",
            file.to_str().expect("UTF-8 path"),
        ],
    );
    assert!(
        std::fs::read_to_string(&file)
            .expect("file")
            .contains("Supermarket")
    );
    assert!(home.join("file.beancount.bc.json").exists());
    run_ok(
        command(&ctx, &["beancount"]),
        &[
            "export",
            "--format",
            "beancount",
            "--level",
            "archive",
            "--force",
            "-o",
            dir.to_str().expect("UTF-8 path"),
        ],
    );
    assert_ne!(
        std::fs::read_to_string(dir.join("main.bean")).expect("main.bean"),
        "kept\n"
    );
}

#[test]
fn export_archive_writes_layout() {
    let ctx = TestContext::new();
    seed(&ctx);
    let dir = ctx.home_dir.path().join("archive");
    let dir_arg = dir.to_str().expect("UTF-8 path");
    insta::assert_snapshot!(export(&ctx, &["--level", "archive", "-o", dir_arg]));

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
    insta::assert_debug_snapshot!("files", names);
    let main = std::fs::read_to_string(dir.join("main.bean")).expect("main.bean");
    insta::assert_snapshot!("main", main);

    // Several files cannot go to stdout.
    insta::assert_snapshot!("stdout_refused", export(&ctx, &["--level", "archive"]));

    // A narrowed archive warns that its balances cover only what it holds.
    let partial = ctx.home_dir.path().join("partial");
    insta::assert_snapshot!(
        "narrowed",
        export(
            &ctx,
            &[
                "--level",
                "archive",
                "--from",
                "2026-04-01",
                "-o",
                partial.to_str().expect("UTF-8 path"),
            ],
        )
    );
}

#[test]
fn export_unspellable_lists_names() {
    let ctx = TestContext::new();
    run_ok(ctx.command(), &["account", "create", "Assets:Checking"]);
    run_ok(ctx.command(), &["account", "create", "Expenses:Groceries"]);
    seed_tx(
        &ctx,
        &["my tag", "another tag"],
        "2026-03-01",
        "Supermarket",
        "5.00",
    );
    let out = ctx.home_dir.path().join("bad.beancount");
    insta::assert_snapshot!(export(&ctx, &["-o", out.to_str().expect("UTF-8 path")]));
    assert!(!out.exists(), "nothing is written on failure");

    // A failed export does not warn about narrowed balances.
    let dir = ctx.home_dir.path().join("bad");
    insta::assert_snapshot!(
        "narrowed_failure",
        export(
            &ctx,
            &[
                "--level",
                "archive",
                "--from",
                "2026-01-01",
                "-o",
                dir.to_str().expect("UTF-8 path"),
            ],
        )
    );
}

/// Runs only where `bean-check` is on `PATH`, and fails without it when
/// `BC_REQUIRE_BEAN_CHECK` is set, as CI sets it.
#[test]
fn export_archive_passes_bean_check() {
    let Ok(bean_check) = which("bean-check") else {
        assert!(
            std::env::var_os("BC_REQUIRE_BEAN_CHECK").is_none(),
            "BC_REQUIRE_BEAN_CHECK is set, but bean-check is not on PATH"
        );
        eprintln!("skipping: bean-check is not on PATH");
        return;
    };
    let ctx = TestContext::new();
    seed(&ctx);
    let dir = ctx.home_dir.path().join("checked");
    run_ok(
        command(&ctx, &["beancount"]),
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
