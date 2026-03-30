//! Tests for `--help`, `-h`, `--version`, shell completions, and stub commands.

use rstest::rstest;

use crate::cmd_snapshot;
use crate::common::TestContext;
use crate::set_snapshot_suffix;

#[rstest]
#[case("--help")]
#[case("-h")]
fn top_level_help(#[case] flag: &str) {
    let ctx = TestContext::new();
    set_snapshot_suffix!("{}", flag.trim_start_matches('-'));
    let mut cmd = ctx.command();
    cmd.arg(flag);
    cmd_snapshot!(ctx, &mut cmd);
}

#[rstest]
#[case("account")]
#[case("transaction")]
#[case("import")]
#[case("export")]
#[case("report")]
#[case("budget")]
#[case("plugin")]
#[case("completions")]
fn subcommand_help(#[case] subcommand: &str) {
    let ctx = TestContext::new();
    set_snapshot_suffix!("{subcommand}");
    let mut cmd = ctx.command();
    cmd.args([subcommand, "--help"]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[rstest]
#[case("budget", "status")]
#[case("plugin", "list")]
fn stub_commands(#[case] cmd_name: &str, #[case] subcommand: &str) {
    let ctx = TestContext::new();
    set_snapshot_suffix!("{cmd_name}_{subcommand}");
    let mut cmd = ctx.command();
    cmd.args([cmd_name, subcommand]);
    cmd_snapshot!(ctx, &mut cmd);
}

#[rstest]
#[case("bash")]
#[case("zsh")]
#[case("fish")]
fn completions_smoke(#[case] shell: &str) {
    let ctx = TestContext::new();
    let output = ctx
        .command()
        .args(["completions", shell])
        .output()
        .expect("run completions");
    assert!(
        output.status.success(),
        "completions {shell} should exit 0, got: {}",
        output.status
    );
    assert!(
        !output.stdout.is_empty(),
        "completions {shell} should produce output"
    );
}
