//! Per-subcommand handler modules.
//!
//! Each module exposes a `struct Args` (clap-derived) and an async
//! `fn execute(args: Args, ctx: &AppContext) -> CliResult<()>`.

pub mod account;
pub mod budget;
pub mod completions;
pub mod export;
pub mod import;
pub mod plugin;
pub mod report;
pub mod transaction;
