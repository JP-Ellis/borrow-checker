//! Plugin management sub-commands (stub — requires Milestone 6).

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `plugin` subcommand.
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The plugin operation to perform.
    #[command(subcommand)]
    pub command: PluginCommand,
}

/// Available plugin operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum PluginCommand {
    /// Install a plugin from a `.wasm` file or the plugin registry.
    Install {
        /// Plugin source (file path or registry name).
        source: String,
    },
    /// List installed plugins.
    List,
    /// Remove an installed plugin by name.
    Remove {
        /// Plugin name to remove.
        name: String,
    },
}

/// Executes the `plugin` subcommand.
///
/// All operations print a "not yet implemented" stub message to stderr.
///
/// # Errors
///
/// Always returns `Ok(())`.
pub async fn execute(args: Args, _ctx: &AppContext) -> CliResult<()> {
    let op = match args.command {
        PluginCommand::Install { .. } => "install",
        PluginCommand::List => "list",
        PluginCommand::Remove { .. } => "remove",
    };
    eprintln!("plugin {op}: requires Milestone 6 (plugin system) — not yet implemented");
    Ok(())
}
