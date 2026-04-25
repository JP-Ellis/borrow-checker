//! Plugin management sub-commands.

use std::path::PathBuf;

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `plugin` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The plugin operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available plugin operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List installed plugins.
    List,
    /// Install a plugin from a `.wasm` file.
    Install {
        /// Path to the `.wasm` file to install.
        #[arg(value_name = "PATH")]
        source: PathBuf,
    },
    /// Remove an installed plugin by name.
    Remove {
        /// Name of the plugin to remove.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

/// Executes the `plugin` subcommand.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] on I/O or not-found errors.
#[expect(
    clippy::unused_async,
    reason = "signature required by command dispatch"
)]
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List => list(ctx),
        Command::Install { source } => install(&source, ctx),
        Command::Remove { name } => remove(&name),
    }
}

/// Lists all loaded plugins, emitting their names.
fn list(ctx: &AppContext) -> CliResult<()> {
    let names: Vec<&str> = ctx.importers.names().collect();

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "plugins": names }));
    }

    if names.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No plugins installed.");
        }
        return Ok(());
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    for name in names {
        println!("{name}");
    }
    Ok(())
}

/// Copies a `.wasm` plugin and its sidecar manifest into the user plugin directory.
fn install(source: &std::path::Path, ctx: &AppContext) -> CliResult<()> {
    // Validate the source is a .wasm file.
    if source.extension().and_then(|e| e.to_str()) != Some("wasm") {
        return Err(crate::error::CliError::Arg(
            "source must be a .wasm file".to_owned(),
        ));
    }

    // Derive the sidecar manifest path.
    let manifest_path = source.with_extension("toml");
    if !manifest_path.exists() {
        return Err(crate::error::CliError::Arg(format!(
            "sidecar manifest not found: {}",
            manifest_path.display()
        )));
    }

    // Determine the user plugin directory (XDG data home).
    let dest_dir = directories::BaseDirs::new()
        .map(|b| b.data_dir().join("borrow-checker").join("plugins"))
        .ok_or_else(|| {
            crate::error::CliError::Arg("cannot determine user data directory".to_owned())
        })?;

    std::fs::create_dir_all(&dest_dir).map_err(crate::error::CliError::Io)?;

    let wasm_dest = dest_dir.join(
        source
            .file_name()
            .ok_or_else(|| crate::error::CliError::Arg("invalid source path".to_owned()))?,
    );
    let toml_dest = wasm_dest.with_extension("toml");

    std::fs::copy(source, &wasm_dest).map_err(crate::error::CliError::Io)?;
    std::fs::copy(&manifest_path, &toml_dest).map_err(crate::error::CliError::Io)?;

    let plugin_name = wasm_dest
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    if ctx.json {
        return crate::output::print_json(
            &serde_json::json!({ "installed": plugin_name, "path": wasm_dest }),
        );
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!(
            "Installed plugin '{plugin_name}' to {}",
            wasm_dest.display()
        );
    }
    Ok(())
}

/// Removes a plugin by name from the user plugin directory.
fn remove(name: &str) -> CliResult<()> {
    let dest_dir = directories::BaseDirs::new()
        .map(|b| b.data_dir().join("borrow-checker").join("plugins"))
        .ok_or_else(|| {
            crate::error::CliError::Arg("cannot determine user data directory".to_owned())
        })?;

    let wasm_path = dest_dir.join(format!("{name}.wasm"));
    let toml_path = dest_dir.join(format!("{name}.toml"));

    if !wasm_path.exists() {
        return Err(crate::error::CliError::Core(bc_core::BcError::NotFound(
            format!("plugin '{name}'"),
        )));
    }

    std::fs::remove_file(&wasm_path).map_err(crate::error::CliError::Io)?;
    if toml_path.exists() {
        std::fs::remove_file(&toml_path).map_err(crate::error::CliError::Io)?;
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Removed plugin '{name}'.");
    }
    Ok(())
}
