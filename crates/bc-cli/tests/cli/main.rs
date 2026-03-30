//! CLI integration test harness.
//!
//! Each module corresponds to one subcommand or feature area.
//! `TestContext` in `common` provides an isolated DB + command runner.

mod common;
// Subcommand test modules — uncommented as each command is implemented:
// mod account;
// mod export;
// mod help;
// mod import;
// mod report;
// mod transaction;
// mod version;

/// Capture binary output as a formatted snapshot string.
#[expect(
    unused_macros,
    reason = "used by test modules added in subsequent tasks"
)]
macro_rules! cmd_snapshot {
    ($ctx:expr, $cmd:expr) => {{
        let output = $ctx.run($cmd);
        insta::assert_snapshot!(output);
    }};
}
#[expect(
    unused_imports,
    reason = "used by test modules added in subsequent tasks"
)]
pub(crate) use cmd_snapshot;

/// Bind a snapshot suffix for parametrised tests.
#[expect(
    unused_macros,
    reason = "used by test modules added in subsequent tasks"
)]
macro_rules! set_snapshot_suffix {
    ($($expr:tt)*) => {
        let _guard = {
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_suffix(format!($($expr)*));
            settings.bind_to_scope()
        };
    };
}
#[expect(
    unused_imports,
    reason = "used by test modules added in subsequent tasks"
)]
pub(crate) use set_snapshot_suffix;
