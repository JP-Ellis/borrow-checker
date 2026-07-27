//! Import profile management sub-commands: create, list, show, edit, remove.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// The wire format of a profile config document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    /// A TOML document.
    Toml,
    /// A JSON document.
    Json,
}

/// Chooses the config format from the source label and the document body.
///
/// A `.toml` or `.json` extension selects directly. Anything else — including
/// stdin, spelled `-` — is sniffed: a document whose first non-whitespace byte
/// is `{` is JSON, because a TOML document cannot begin with `{`.
///
/// # Arguments
///
/// * `label` - The `--config` argument: a file path, or `-` for stdin.
/// * `text` - The full document body.
///
/// # Returns
///
/// The [`ConfigFormat`] to parse `text` with.
fn config_format(label: &str, text: &str) -> ConfigFormat {
    match std::path::Path::new(label)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
    {
        Some("toml") => ConfigFormat::Toml,
        Some("json") => ConfigFormat::Json,
        _ if text.trim_start().starts_with('{') => ConfigFormat::Json,
        _ => ConfigFormat::Toml,
    }
}

/// Parses a config document into a JSON value.
///
/// # Arguments
///
/// * `text` - The document body.
/// * `format` - The format to parse `text` as.
/// * `label` - The source name, used in error messages.
///
/// # Returns
///
/// The parsed configuration as a [`serde_json::Value`], guaranteed to be an
/// object.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the document is malformed, or if its root is
/// not a table of key/value pairs.
fn parse_config(text: &str, format: ConfigFormat, label: &str) -> CliResult<serde_json::Value> {
    let value: serde_json::Value = match format {
        ConfigFormat::Toml => toml::from_str(text)
            .map_err(|e| CliError::Arg(format!("invalid TOML in {label}: {e}")))?,
        ConfigFormat::Json => serde_json::from_str(text)
            .map_err(|e| CliError::Arg(format!("invalid JSON in {label}: {e}")))?,
    };

    if !value.is_object() {
        return Err(CliError::Arg(format!(
            "config in {label} must be a table of key/value pairs"
        )));
    }

    Ok(value)
}

/// Reads and parses a profile config from a file, or from stdin.
///
/// # Arguments
///
/// * `label` - A file path, or `-` to read stdin.
///
/// # Returns
///
/// The parsed configuration as a [`serde_json::Value`].
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the source cannot be read or does not parse.
fn load_config(label: &str) -> CliResult<serde_json::Value> {
    let text = if label == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .map_err(|e| CliError::Arg(format!("cannot read config from stdin: {e}")))?;
        buf
    } else {
        std::fs::read_to_string(label)
            .map_err(|e| CliError::Arg(format!("cannot read config file '{label}': {e}")))?
    };

    parse_config(&text, config_format(label, &text), label)
}

/// Arguments for the `profile` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The import profile operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available import profile operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Create an import profile from a TOML or JSON config file.
    Create {
        /// Unique name for the profile.
        #[arg(long, value_name = "NAME")]
        name: String,
        /// Stable identifier of the importer plugin (e.g. `csv`).
        #[arg(long, value_name = "PLUGIN")]
        importer: String,
        /// Path to a TOML or JSON config file, or `-` to read stdin.
        #[arg(long, value_name = "FILE")]
        config: String,
    },
    /// List all import profiles.
    List,
    /// Remove an import profile by name.
    Remove {
        /// Name of the profile to remove.
        #[arg(value_name = "NAME")]
        name: String,
    },
}

/// Executes the `profile` subcommand.
///
/// # Arguments
///
/// * `args` - The parsed subcommand arguments.
/// * `ctx` - The application context.
///
/// # Errors
///
/// Returns a [`CliError`] if a config file cannot be read or parsed, or if a
/// service call fails.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::Create {
            name,
            importer,
            config,
        } => create(ctx, name, importer, config).await,
        Command::List => list(ctx).await,
        Command::Remove { name } => remove(ctx, name).await,
    }
}

/// Warns on stderr when no installed importer matches `importer`.
///
/// This is deliberately a warning rather than an error: an unrecognised
/// importer name is not an unrepresentable state, a profile may legitimately
/// be written before its plugin is installed, and `plugin remove` must not
/// strand profiles that can no longer be inspected or edited. `import`
/// hard-errors at the point of use instead.
///
/// # Arguments
///
/// * `ctx` - The application context, holding the importer registry.
/// * `importer` - The importer name supplied on the command line.
fn warn_unknown_importer(ctx: &AppContext, importer: &str) {
    if ctx.importers.names().any(|n| n == importer) {
        return;
    }

    let installed: Vec<&str> = ctx.importers.names().collect();
    let list = if installed.is_empty() {
        "none".to_owned()
    } else {
        installed.join(", ")
    };

    #[expect(clippy::print_stderr, reason = "user-visible validation warning")]
    {
        eprintln!("warning: no installed importer named '{importer}' (installed: {list})");
    }
}

/// Creates an import profile from a config file.
///
/// # Arguments
///
/// * `ctx` - The application context.
/// * `name` - Unique name for the new profile.
/// * `importer` - Stable identifier of the importer plugin.
/// * `config` - Path to a TOML or JSON config file, or `-` for stdin.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the config cannot be read or parsed, or
/// [`CliError::Core`] if the name is already taken.
async fn create(ctx: &AppContext, name: String, importer: String, config: String) -> CliResult<()> {
    let value = load_config(&config)?;
    let id = ctx
        .profiles
        .create(&name, &importer, bc_core::ImportConfig::from_value(value))
        .await?;
    warn_unknown_importer(ctx, &importer);

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "id": id.to_string(),
            "name": name,
            "importer": importer,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created import profile '{name}' ({importer}): {id}");
    }
    Ok(())
}

/// Lists all import profiles as a table of name, importer, and creation time.
///
/// `created_at` is rendered in full RFC 3339 form rather than a friendlier
/// date so that the integration harness's `[TIMESTAMP]` filter redacts it and
/// snapshots stay stable across runs.
///
/// # Arguments
///
/// * `ctx` - The application context.
///
/// # Errors
///
/// Returns [`CliError::Core`] if the profile service call fails, or
/// [`CliError::Json`] if JSON serialisation fails.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let profiles = ctx.profiles.list_all().await?;

    if ctx.json {
        let items: Vec<serde_json::Value> = profiles
            .iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id.to_string(),
                    "name": p.name,
                    "importer": p.importer,
                    "created_at": p.created_at.to_string(),
                })
            })
            .collect();
        return crate::output::print_json(&serde_json::json!({ "profiles": items }));
    }

    if profiles.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No import profiles.");
        }
        return Ok(());
    }

    let rows: Vec<Vec<String>> = profiles
        .iter()
        .map(|p| vec![p.name.clone(), p.importer.clone(), p.created_at.to_string()])
        .collect();
    crate::output::print_table(&["NAME", "IMPORTER", "CREATED"], &rows);
    Ok(())
}

/// Removes an import profile by name.
///
/// # Arguments
///
/// * `ctx` - The application context.
/// * `name` - Name of the profile to remove.
///
/// # Errors
///
/// Returns [`CliError::Core`] if no profile with that name exists, or the
/// delete fails.
async fn remove(ctx: &AppContext, name: String) -> CliResult<()> {
    let profile = ctx.profiles.find_by_name(&name).await?;
    ctx.profiles.delete(&profile.id).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "removed": name }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Removed import profile '{name}'.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn toml_and_json_configs_produce_the_same_value() {
        let from_toml = parse_config(
            "account = \"Assets:Bank:Checking\"\nsource_dir = \"Assets/Bank/Checking\"\n",
            ConfigFormat::Toml,
            "bank.toml",
        )
        .expect("TOML config should parse");
        let from_json = parse_config(
            r#"{"account": "Assets:Bank:Checking", "source_dir": "Assets/Bank/Checking"}"#,
            ConfigFormat::Json,
            "bank.json",
        )
        .expect("JSON config should parse");

        assert_eq!(from_toml, from_json);
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known structure")]
    fn nested_tables_survive_the_toml_conversion() {
        let value = parse_config(
            "account = \"Assets:Bank:Checking\"\n\n[preamble]\nstrategy = \"skip_lines\"\nlines = 3\n",
            ConfigFormat::Toml,
            "bank.toml",
        )
        .expect("TOML config should parse");

        assert_eq!(value["preamble"]["strategy"], "skip_lines");
        assert_eq!(value["preamble"]["lines"], 3_i32);
    }

    #[test]
    fn extensions_select_the_format() {
        assert_eq!(config_format("bank.toml", "{}"), ConfigFormat::Toml);
        assert_eq!(config_format("bank.json", "a = 1"), ConfigFormat::Json);
    }

    #[test]
    fn unknown_extensions_and_stdin_are_sniffed() {
        assert_eq!(config_format("-", "  {\"a\": 1}"), ConfigFormat::Json);
        assert_eq!(config_format("-", "a = 1"), ConfigFormat::Toml);
        assert_eq!(config_format("bank.conf", "{\"a\": 1}"), ConfigFormat::Json);
        assert_eq!(config_format("bank.conf", "a = 1"), ConfigFormat::Toml);
    }

    #[test]
    fn a_non_table_root_is_rejected() {
        let result = parse_config("[1, 2, 3]", ConfigFormat::Json, "bank.json");
        assert!(result.is_err(), "a JSON array root must be rejected");
    }

    #[test]
    fn malformed_input_names_the_source() {
        let err = parse_config("this is not = = toml", ConfigFormat::Toml, "bank.toml")
            .expect_err("malformed TOML must be rejected");
        assert!(
            err.to_string().contains("bank.toml"),
            "error must name the source, got: {err}"
        );
    }

    #[test]
    fn load_config_reports_a_missing_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("absent.toml");
        let label = missing.to_str().expect("path is UTF-8");
        let err = load_config(label).expect_err("a missing file must be an error");
        assert!(
            err.to_string().contains("absent.toml"),
            "error must name the path, got: {err}"
        );
    }
}
