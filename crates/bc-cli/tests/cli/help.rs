//! Tests for `--help` and `-h` flags.

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
