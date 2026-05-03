//! Plugin management sub-commands.

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
    /// Install a plugin from a `.wasm` file or an https:// URL.
    Install {
        /// Path to a local `.wasm` file, or an `http(s)://` URL.
        #[arg(value_name = "SOURCE")]
        source: String,
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

/// Returns `true` if `source` looks like an HTTP(S) URL.
fn is_url(source: &str) -> bool {
    source.starts_with("http://") || source.starts_with("https://")
}

/// Lists all loaded plugins, emitting their name, ABI, and source path.
fn list(ctx: &AppContext) -> CliResult<()> {
    if ctx.json {
        let plugins: Vec<serde_json::Value> = ctx
            .plugin_registry
            .plugins()
            .map(|p| {
                serde_json::json!({
                    "name": p.name(),
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
            "{}  (ABI v{})  {}",
            p.name(),
            p.sdk_abi(),
            p.source_path().display()
        );
    }
    Ok(())
}

/// Copies a `.wasm` plugin into `dest_dir`, naming it by its self-reported plugin name.
///
/// The destination filename is derived from the plugin's `name()` export
/// (e.g. `"ledger"` → `ledger.wasm`), not from the source filename. This
/// ensures `plugin install` and `plugin remove` use consistent naming.
///
/// # Arguments
///
/// * `source` - Path to the source `.wasm` file.
/// * `dest_dir` - Directory to install into. Created if it does not exist.
///
/// # Returns
///
/// The canonical plugin name from the WASM component.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the source is not a `.wasm` file, the
/// plugin metadata cannot be queried, the directory cannot be created, or
/// any file copy fails.
fn install_to_dir(source: &std::path::Path, dest_dir: &std::path::Path) -> CliResult<String> {
    if source.extension().and_then(|e| e.to_str()) != Some("wasm") {
        return Err(crate::error::CliError::Arg(
            "source must be a .wasm file".to_owned(),
        ));
    }

    let meta = bc_plugins::query_metadata(source)
        .map_err(|e| crate::error::CliError::Arg(format!("cannot read plugin metadata: {e}")))?;
    let plugin_name = meta.name;

    std::fs::create_dir_all(dest_dir).map_err(crate::error::CliError::Io)?;

    let wasm_dest = dest_dir.join(format!("{plugin_name}.wasm"));
    std::fs::copy(source, &wasm_dest).map_err(crate::error::CliError::Io)?;

    Ok(plugin_name)
}

/// Removes a plugin by name from `dest_dir`.
///
/// # Arguments
///
/// * `name` - The plugin name to remove (e.g. `"ledger"`).
/// * `dest_dir` - The plugin directory to remove from.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the plugin is not found or the file
/// cannot be deleted.
fn remove_from_dir(name: &str, dest_dir: &std::path::Path) -> CliResult<()> {
    let wasm_path = dest_dir.join(format!("{name}.wasm"));

    if !wasm_path.exists() {
        return Err(crate::error::CliError::Core(bc_core::BcError::NotFound(
            format!("plugin '{name}'"),
        )));
    }

    std::fs::remove_file(&wasm_path).map_err(crate::error::CliError::Io)?;
    Ok(())
}

/// Downloads a URL to a temp file and returns the temp file path.
///
/// # Arguments
///
/// * `url` - The HTTP(S) URL to download from.
///
/// # Returns
///
/// A named temporary file containing the downloaded bytes.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the temporary file cannot be created,
/// the HTTP request fails, or writing the response body fails.
fn download_to_temp(url: &str) -> CliResult<tempfile::NamedTempFile> {
    let mut tmp = tempfile::Builder::new()
        .suffix(".wasm")
        .tempfile()
        .map_err(crate::error::CliError::Io)?;

    let bytes = reqwest::blocking::get(url)
        .and_then(reqwest::blocking::Response::error_for_status)
        .and_then(reqwest::blocking::Response::bytes)
        .map_err(|e| crate::error::CliError::Arg(format!("download failed: {e}")))?;

    std::io::Write::write_all(&mut tmp, &bytes).map_err(crate::error::CliError::Io)?;
    Ok(tmp)
}

/// Installs a plugin from a local file path or https:// URL.
///
/// # Arguments
///
/// * `source` - A local filesystem path to a `.wasm` file, or an `http(s)://` URL.
/// * `ctx` - The application context, used to determine output mode.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the user data directory cannot be
/// determined, the download fails (for URLs), or the file cannot be installed.
fn install(source: &str, ctx: &AppContext) -> CliResult<()> {
    let dest_dir = bc_config::user_plugin_dir().ok_or_else(|| {
        crate::error::CliError::Arg("cannot determine user data directory".to_owned())
    })?;

    let plugin_name = if is_url(source) {
        let tmp = download_to_temp(source)?;
        install_to_dir(tmp.path(), &dest_dir)?
    } else {
        install_to_dir(std::path::Path::new(source), &dest_dir)?
    };

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
///
/// # Arguments
///
/// * `name` - The plugin name to remove.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the user data directory cannot be
/// determined or the plugin is not found.
fn remove(name: &str) -> CliResult<()> {
    let dest_dir = bc_config::user_plugin_dir().ok_or_else(|| {
        crate::error::CliError::Arg("cannot determine user data directory".to_owned())
    })?;
    remove_from_dir(name, &dest_dir)?;

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Removed plugin '{name}'.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_url_recognises_http_and_https() {
        assert!(is_url("http://example.com/plugin.wasm"));
        assert!(is_url("https://example.com/plugin.wasm"));
        assert!(!is_url("/local/path/plugin.wasm"));
        assert!(!is_url("relative/plugin.wasm"));
        assert!(!is_url("plugin.wasm"));
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

    #[test]
    fn install_to_dir_rejects_non_wasm_extension() {
        let src_dir = tempfile::tempdir().expect("tempdir");
        let bad = src_dir.path().join("plugin.txt");
        std::fs::write(&bad, b"not wasm").expect("write");
        let dest_dir = tempfile::tempdir().expect("tempdir");
        let result = install_to_dir(&bad, dest_dir.path());
        assert!(result.is_err(), "non-.wasm source must be rejected");
    }
}
