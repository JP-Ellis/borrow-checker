//! Tests for `--version` and `-V` flags.

use rstest::rstest;

use crate::cmd_snapshot;
use crate::common::TestContext;
use crate::set_snapshot_suffix;

#[rstest]
#[case("--version")]
#[case("-V")]
fn version(#[case] flag: &str) {
    let ctx = TestContext::new();
    set_snapshot_suffix!("{}", flag.trim_start_matches('-'));
    let mut cmd = ctx.command();
    cmd.arg(flag);
    cmd_snapshot!(ctx, &mut cmd);
}
