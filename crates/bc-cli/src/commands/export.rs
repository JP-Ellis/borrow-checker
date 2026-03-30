//! Export sub-command.

use std::path::PathBuf;

use crate::context::AppContext;
use crate::error::CliResult;

/// Supported export formats.
#[non_exhaustive]
#[derive(Debug, Clone, clap::ValueEnum)]
pub enum Format {
    /// Ledger journal format.
    Ledger,
    /// Beancount journal format.
    Beancount,
}

/// Arguments for the `export` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Output format.
    #[arg(long, value_enum)]
    pub format: Format,

    /// Output file path. Writes to stdout when omitted.
    #[arg(long, short)]
    pub output: Option<PathBuf>,
}

/// Executes the `export` subcommand.
///
/// Collects all accounts and transactions and serialises them using the
/// requested exporter. Writes to `--output <file>` or stdout if omitted.
///
/// # Errors
///
/// Propagates any [`crate::error::CliError`] from the core engine or I/O.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    // Gather domain data.
    let accounts = ctx.accounts.list_active().await?;
    let transactions = ctx.transactions.list().await?;
    // Commodities and tags are not yet exposed by the service layer.
    let tags: &[bc_models::Tag] = &[];
    let commodities: &[bc_models::Commodity] = &[];

    let export_data = bc_core::ExportData::new(&accounts, commodities, &transactions, tags);

    let exporter: Box<dyn bc_core::Exporter> = match args.format {
        Format::Ledger => Box::new(bc_format_ledger::LedgerExporter::default()),
        Format::Beancount => Box::new(bc_format_beancount::BeancountExporter::default()),
    };

    let bytes = exporter
        .export(&export_data)
        .map_err(|e| crate::error::CliError::Arg(format!("export error: {e}")))?;

    match args.output {
        Some(ref path) => {
            std::fs::write(path, &bytes).map_err(crate::error::CliError::Io)?;
            if ctx.json {
                crate::output::print_json(&serde_json::json!({
                    "output": path.display().to_string(),
                    "bytes": bytes.len(),
                }))?;
            } else {
                #[expect(clippy::print_stdout, reason = "CLI output")]
                {
                    println!("Exported to {}", path.display());
                }
            }
        }
        None => {
            #[expect(clippy::print_stdout, reason = "export output to stdout")]
            {
                print!("{}", String::from_utf8_lossy(&bytes));
            }
        }
    }

    Ok(())
}
