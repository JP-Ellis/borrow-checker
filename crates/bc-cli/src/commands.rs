//! Per-subcommand handler modules.
//!
//! Each module exposes a `struct Args` (clap-derived) and an async
//! `fn execute(args: Args, ctx: &AppContext) -> CliResult<()>`.

pub mod account;
pub mod asset;
pub mod backup;
pub mod budget;
pub mod commodity;
pub mod completions;
pub mod export;
pub mod import;
pub mod meta;
pub mod plugin;
pub mod profile;
pub mod report;
pub mod restore;
pub mod tag;
pub mod transaction;
pub mod transfer;

/// Parses a `YYYY-MM-DD` date string, or returns today's date when `None`.
///
/// # Arguments
///
/// * `s` - Optional date string in `YYYY-MM-DD` format.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if the string cannot be parsed.
pub(crate) fn parse_date_or_today(s: Option<&str>) -> crate::error::CliResult<jiff::civil::Date> {
    match s {
        Some(d) => <jiff::civil::Date as core::str::FromStr>::from_str(d)
            .map_err(|e| crate::error::CliError::Arg(format!("invalid date '{d}': {e}"))),
        None => Ok(jiff::Zoned::now().date()),
    }
}
