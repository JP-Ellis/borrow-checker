//! Commodity registry sub-commands: list, create, update, delete.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `commodity` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The commodity operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available commodity operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List every registered commodity.
    List,
}

/// Executes the `commodity` subcommand.
///
/// # Arguments
///
/// * `args` - The parsed subcommand arguments.
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Returns a [`crate::error::CliError`] if a service call fails or an argument
/// is invalid.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List => list(ctx).await,
    }
}

/// Lists every registered commodity as a table or JSON.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
///
/// # Errors
///
/// Propagates [`crate::error::CliError::Core`] from the commodity service or
/// [`crate::error::CliError::Json`] from JSON serialisation.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let all = ctx.commodities.list_all().await?;
    let rows: Vec<Vec<String>> = all
        .iter()
        .map(|c| {
            vec![
                c.code().to_owned(),
                c.name().unwrap_or_default().to_owned(),
                c.symbol().unwrap_or_default().to_owned(),
                c.decimals().to_string(),
                if c.is_iso() { "yes" } else { "no" }.to_owned(),
                c.aliases().join(", "),
            ]
        })
        .collect();

    if ctx.json {
        return crate::output::print_json(&rows);
    }
    if rows.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No commodities.");
        }
        return Ok(());
    }
    crate::output::print_table(
        &["CODE", "NAME", "SYMBOL", "DECIMALS", "ISO", "ALIASES"],
        &rows,
    );
    Ok(())
}
