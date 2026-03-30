//! CLI integration test harness.
//!
//! Each module corresponds to one subcommand or feature area.
//! `TestContext` in `common` provides an isolated DB + command runner.

mod common;
mod help;
mod version;
// Subcommand test modules — uncommented as each command is implemented:
mod account;
mod export;
mod import;
// mod report;
mod transaction;

/// Capture binary output as a formatted snapshot string.
macro_rules! cmd_snapshot {
    ($ctx:expr, $cmd:expr) => {{
        let output = $ctx.run($cmd);
        insta::assert_snapshot!(output);
    }};
}
pub(crate) use cmd_snapshot;

/// Bind a snapshot suffix for parametrised tests.
macro_rules! set_snapshot_suffix {
    ($($expr:tt)*) => {
        let _guard = {
            let mut settings = insta::Settings::clone_current();
            settings.set_snapshot_suffix(format!($($expr)*));
            settings.bind_to_scope()
        };
    };
}
pub(crate) use set_snapshot_suffix;
