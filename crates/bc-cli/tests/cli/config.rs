//! The CLI refuses to run on a config it cannot load.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use crate::common::TestContext;

/// Writes `text` as the user config under the test's isolated home.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn write_user_config(ctx: &TestContext, text: &str) {
    let dir = ctx.home_dir.path().join(".config").join("borrow-checker");
    std::fs::create_dir_all(&dir).expect("create config dir");
    std::fs::write(dir.join("config.toml"), text).expect("write config");
}

#[test]
fn both_spellings_stop_the_cli() {
    let ctx = TestContext::new();
    write_user_config(
        &ctx,
        "display_commodity = \"USD\"\ndisplay-commodity = \"EUR\"\n",
    );

    let output = ctx
        .command()
        .env("XDG_CONFIG_HOME", ctx.home_dir.path().join(".config"))
        .args(["account", "list"])
        .output()
        .expect("run");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("display-commodity"), "{stderr}");
    assert!(
        !ctx.db_path.exists(),
        "no database is opened on a config error"
    );
}

#[test]
fn retired_variable_stops_the_cli() {
    let ctx = TestContext::new();

    let output = ctx
        .command()
        .env("XDG_CONFIG_HOME", ctx.home_dir.path().join(".config"))
        .env("BC_DB_PATH", ctx.home_dir.path().join("old.sqlite"))
        .args(["account", "list"])
        .output()
        .expect("run");

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("BC_DB__PATH"));
}

#[test]
fn relative_db_path_resolves_beside_the_config() {
    let ctx = TestContext::new();
    write_user_config(&ctx, "[db]\npath = \"ledger/db.sqlite\"\n");

    let output = ctx
        .bare_command()
        .env("XDG_CONFIG_HOME", ctx.home_dir.path().join(".config"))
        .env("BC_BACKUP__DIR", ctx.home_dir.path().join("bk"))
        .args(["account", "list"])
        .output()
        .expect("run");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = ctx
        .home_dir
        .path()
        .join(".config/borrow-checker/ledger/db.sqlite");
    assert!(
        expected.exists(),
        "database created at {}",
        expected.display()
    );
}
