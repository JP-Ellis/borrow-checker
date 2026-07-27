//! Integration tests for the `profile` subcommand.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]
#![expect(clippy::expect_used, reason = "tests panic on setup failure")]

use crate::cmd_snapshot;
use crate::common::TestContext;

/// A minimal, obviously-fake CSV importer config used across these tests.
const BANK_CONFIG: &str = "account = \"Assets:Bank:Checking\"\n\
                           source_dir = \"Assets/Bank/Checking\"\n\
                           date_column = \"Date\"\n";

/// Creates an isolated context that already contains a `bank` profile.
///
/// The seeding command's own output is asserted but not snapshotted, so that
/// each test snapshots exactly one command.
fn ctx_with_bank_profile() -> TestContext {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.toml");
    std::fs::write(&cfg, BANK_CONFIG).expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    let output = ctx.run(&mut cmd);
    assert!(
        output.contains("success: true"),
        "seeding profile create failed:\n{output}"
    );

    ctx
}

#[test]
fn profile_create_succeeds() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.toml");
    std::fs::write(&cfg, BANK_CONFIG).expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_accepts_json_config() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.json");
    std::fs::write(
        &cfg,
        r#"{"account": "Assets:Bank:Checking", "source_dir": "Assets/Bank/Checking"}"#,
    )
    .expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_a_duplicate_name() {
    let ctx = ctx_with_bank_profile();
    let cfg = ctx.home_dir.path().join("bank.toml");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "ofx",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_a_malformed_config() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("broken.toml");
    std::fs::write(&cfg, "this is not = = toml\n").expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_a_non_table_config() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("array.json");
    std::fs::write(&cfg, "[1, 2, 3]\n").expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_reads_config_from_stdin() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
        "-",
    ])
    .write_stdin(BANK_CONFIG);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_rejects_empty_stdin() {
    let ctx = TestContext::new();

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
        "-",
    ])
    .write_stdin("");
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_create_json_output() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.toml");
    std::fs::write(&cfg, BANK_CONFIG).expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args([
        "profile",
        "create",
        "--name",
        "bank",
        "--importer",
        "csv",
        "--config",
    ])
    .arg(&cfg)
    .arg("--json");
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_list_is_empty_by_default() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["profile", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_list_shows_a_created_profile() {
    let ctx = ctx_with_bank_profile();
    let mut cmd = ctx.command();
    cmd.args(["profile", "list"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_list_json_output() {
    let ctx = ctx_with_bank_profile();
    let mut cmd = ctx.command();
    cmd.args(["profile", "list", "--json"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_remove_deletes_the_profile() {
    let ctx = ctx_with_bank_profile();

    let mut cmd = ctx.command();
    cmd.args(["profile", "remove", "bank"]);
    let removed = ctx.run(&mut cmd);
    assert!(
        removed.contains("success: true"),
        "remove failed:\n{removed}"
    );

    let mut list = ctx.command();
    list.args(["profile", "list"]);
    cmd_snapshot!(ctx, &mut list);
}

#[test]
fn profile_remove_json_output() {
    let ctx = ctx_with_bank_profile();

    let mut cmd = ctx.command();
    cmd.args(["profile", "remove", "bank", "--json"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_remove_unknown_name_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["profile", "remove", "nonexistent"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_show_renders_config_as_toml() {
    let ctx = ctx_with_bank_profile();
    let mut cmd = ctx.command();
    cmd.args(["profile", "show", "bank"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_show_json_output() {
    let ctx = ctx_with_bank_profile();
    let mut cmd = ctx.command();
    cmd.args(["profile", "show", "bank", "--json"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_show_unknown_name_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["profile", "show", "nonexistent"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_show_rejects_a_null_inside_an_array() {
    let ctx = TestContext::new();
    let cfg = ctx.home_dir.path().join("bank.json");
    std::fs::write(
        &cfg,
        r#"{"account": "Assets:Bank:Checking", "columns": ["Date", null, "Amount"]}"#,
    )
    .expect("write config fixture");

    let mut create = ctx.command();
    create
        .args([
            "profile",
            "create",
            "--name",
            "bank",
            "--importer",
            "csv",
            "--config",
        ])
        .arg(&cfg);
    let created = ctx.run(&mut create);
    assert!(
        created.contains("success: true"),
        "seeding profile create failed:\n{created}"
    );

    let mut show = ctx.command();
    show.args(["profile", "show", "bank"]);
    cmd_snapshot!(ctx, &mut show);
}

#[test]
fn profile_edit_replaces_the_config() {
    let ctx = ctx_with_bank_profile();
    let cfg = ctx.home_dir.path().join("bank-v2.toml");
    std::fs::write(
        &cfg,
        "account = \"Assets:Bank:Savings\"\nsource_dir = \"Assets/Bank/Savings\"\n",
    )
    .expect("write config fixture");

    let mut cmd = ctx.command();
    cmd.args(["profile", "edit", "bank", "--config"]).arg(&cfg);
    let edited = ctx.run(&mut cmd);
    assert!(edited.contains("success: true"), "edit failed:\n{edited}");

    let mut show = ctx.command();
    show.args(["profile", "show", "bank"]);
    cmd_snapshot!(ctx, &mut show);
}

#[test]
fn profile_edit_renames_the_profile() {
    let ctx = ctx_with_bank_profile();

    let mut cmd = ctx.command();
    cmd.args(["profile", "edit", "bank", "--name", "savings"]);
    let renamed = ctx.run(&mut cmd);
    assert!(
        renamed.contains("success: true"),
        "rename failed:\n{renamed}"
    );

    let mut list = ctx.command();
    list.args(["profile", "list"]);
    cmd_snapshot!(ctx, &mut list);
}

#[test]
fn profile_edit_with_no_changes_errors() {
    let ctx = ctx_with_bank_profile();
    let mut cmd = ctx.command();
    cmd.args(["profile", "edit", "bank"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[test]
fn profile_edit_unknown_name_errors() {
    let ctx = TestContext::new();
    let mut cmd = ctx.command();
    cmd.args(["profile", "edit", "nonexistent", "--importer", "ofx"]);
    cmd_snapshot!(ctx, &mut cmd);
}
