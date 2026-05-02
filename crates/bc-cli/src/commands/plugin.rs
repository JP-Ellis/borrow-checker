//! Plugin management sub-commands.

use std::path::PathBuf;

use bc_core::Importer as _;
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

/// Lists all loaded plugins, emitting their name, version, ABI, and source path.
fn list(ctx: &AppContext) -> CliResult<()> {
    if ctx.json {
        let plugins: Vec<serde_json::Value> = ctx
            .plugin_registry
            .plugins()
            .map(|p| {
                serde_json::json!({
                    "name": p.name(),
                    "version": p.version(),
                    "sdk_abi": p.sdk_abi(),
                    "source_path": p.source_path().display().to_string(),
                })
            })
            .collect();
        return crate::output::print_json(&serde_json::json!({ "plugins": plugins }));
    }

    if ctx.plugin_registry.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No plugins installed.");
        }
        return Ok(());
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    for p in ctx.plugin_registry.plugins() {
        println!(
            "{}  {}  (ABI v{})  {}",
            p.name(),
            p.version(),
            p.sdk_abi(),
            p.source_path().display()
        );
    }
    Ok(())
}

/// Resolves the user plugin directory (XDG data home).
///
/// # Returns
///
/// The path to `~/.local/share/borrow-checker/plugins/` (or the platform
/// equivalent).
///
/// # Errors
///
/// Returns [`crate::error::CliError::Arg`] if the home directory cannot be
/// determined.
fn user_plugin_dir() -> CliResult<PathBuf> {
    directories::BaseDirs::new()
        .map(|b| b.data_dir().join("borrow-checker").join("plugins"))
        .ok_or_else(|| {
            crate::error::CliError::Arg("cannot determine user data directory".to_owned())
        })
}

/// Copies a `.wasm` plugin and its sidecar manifest into `dest_dir`.
///
/// The destination filename is derived from the plugin's manifest `name` field
/// (e.g. `"ledger"` → `ledger.wasm` + `ledger.toml`), not from the source
/// filename. This ensures `plugin install` and `plugin remove` use consistent
/// naming.
///
/// # Arguments
///
/// * `source` - Path to the source `.wasm` file.
/// * `dest_dir` - Directory to install into. Created if it does not exist.
///
/// # Returns
///
/// The canonical plugin name from the manifest.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the source is not a `.wasm` file, the
/// sidecar manifest is missing or invalid, the directory cannot be created, or
/// any file copy fails.
fn install_to_dir(source: &std::path::Path, dest_dir: &std::path::Path) -> CliResult<String> {
    if source.extension().and_then(|e| e.to_str()) != Some("wasm") {
        return Err(crate::error::CliError::Arg(
            "source must be a .wasm file".to_owned(),
        ));
    }

    let manifest_path = source.with_extension("toml");
    if !manifest_path.exists() {
        return Err(crate::error::CliError::Arg(format!(
            "sidecar manifest not found: {}",
            manifest_path.display()
        )));
    }

    // Load the manifest first to get the canonical plugin name and validate ABI.
    let manifest = bc_plugins::PluginManifest::load(&manifest_path)
        .map_err(|e| crate::error::CliError::Arg(format!("invalid plugin manifest: {e}")))?;
    let plugin_name = manifest.name;

    std::fs::create_dir_all(dest_dir).map_err(crate::error::CliError::Io)?;

    // Use the manifest name as the destination stem so `plugin remove <name>` can
    // find the files regardless of what the source filename was called.
    let wasm_dest = dest_dir.join(format!("{plugin_name}.wasm"));
    let toml_dest = dest_dir.join(format!("{plugin_name}.toml"));

    std::fs::copy(source, &wasm_dest).map_err(crate::error::CliError::Io)?;
    std::fs::copy(&manifest_path, &toml_dest).map_err(crate::error::CliError::Io)?;

    Ok(plugin_name)
}

/// Removes a plugin by name from `dest_dir`.
///
/// # Arguments
///
/// * `name` - The manifest name of the plugin to remove (e.g. `"ledger"`).
/// * `dest_dir` - The plugin directory to remove from.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the plugin is not found or the files
/// cannot be deleted.
fn remove_from_dir(name: &str, dest_dir: &std::path::Path) -> CliResult<()> {
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

    Ok(())
}

/// Copies a `.wasm` plugin and its sidecar manifest into the user plugin directory.
fn install(source: &std::path::Path, ctx: &AppContext) -> CliResult<()> {
    let dest_dir = user_plugin_dir()?;
    let plugin_name = install_to_dir(source, &dest_dir)?;

    let wasm_dest = dest_dir.join(format!("{plugin_name}.wasm"));

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
    let dest_dir = user_plugin_dir()?;
    remove_from_dir(name, &dest_dir)?;

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Removed plugin '{name}'.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// Creates a minimal valid plugin manifest TOML string with the given name.
    fn manifest_toml(name: &str) -> String {
        format!(
            "[plugin]\nname = \"{name}\"\nversion = \"0.1.0\"\nsdk_abi = 1\nmin_host = \"0.1.0\"\n"
        )
    }

    #[test]
    fn install_to_dir_uses_manifest_name_not_source_filename() {
        // Source file is named differently from the manifest's plugin name.
        let src_dir = tempfile::tempdir().expect("tempdir");
        let wasm_src = src_dir.path().join("bc_plugin_ledger.wasm");
        std::fs::write(&wasm_src, b"wasm").expect("write wasm");
        std::fs::write(
            src_dir.path().join("bc_plugin_ledger.toml"),
            manifest_toml("ledger"),
        )
        .expect("write toml");

        let dest_dir = tempfile::tempdir().expect("tempdir");
        let name = install_to_dir(&wasm_src, dest_dir.path()).expect("install");

        assert_eq!(name, "ledger");
        assert!(
            dest_dir.path().join("ledger.wasm").exists(),
            "ledger.wasm must exist"
        );
        assert!(
            dest_dir.path().join("ledger.toml").exists(),
            "ledger.toml must exist"
        );
        assert!(
            !dest_dir.path().join("bc_plugin_ledger.wasm").exists(),
            "source filename must not be used"
        );
    }

    #[test]
    fn install_to_dir_then_remove_from_dir_round_trips() {
        let src_dir = tempfile::tempdir().expect("tempdir");
        let wasm_src = src_dir.path().join("bc_plugin_csv.wasm");
        std::fs::write(&wasm_src, b"wasm").expect("write wasm");
        std::fs::write(
            src_dir.path().join("bc_plugin_csv.toml"),
            manifest_toml("csv"),
        )
        .expect("write toml");

        let dest_dir = tempfile::tempdir().expect("tempdir");
        install_to_dir(&wasm_src, dest_dir.path()).expect("install");
        remove_from_dir("csv", dest_dir.path()).expect("remove");

        assert!(
            !dest_dir.path().join("csv.wasm").exists(),
            "csv.wasm must be removed"
        );
    }

    #[test]
    fn install_to_dir_propagates_manifest_error() {
        let src_dir = tempfile::tempdir().expect("tempdir");
        let wasm_src = src_dir.path().join("bad.wasm");
        std::fs::write(&wasm_src, b"wasm").expect("write wasm");
        std::fs::write(
            src_dir.path().join("bad.toml"),
            "[plugin]\nname = \"bad\"\nversion = \"0.1.0\"\nsdk_abi = 99\nmin_host = \"0.1.0\"\n",
        )
        .expect("write toml");

        let dest_dir = tempfile::tempdir().expect("tempdir");
        let result = install_to_dir(&wasm_src, dest_dir.path());
        assert!(
            result.is_err(),
            "unsupported ABI manifest must return an error"
        );
    }

    #[test]
    fn remove_from_dir_returns_not_found_for_missing_plugin() {
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let result = remove_from_dir("nonexistent", dest_dir.path());
        assert!(
            result.is_err(),
            "removing absent plugin must return an error"
        );
    }
}
