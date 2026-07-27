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
/// Returns [`CliError::Arg`] if the document is malformed, if its root is not
/// a table of key/value pairs, or if that table is empty.
fn parse_config(text: &str, format: ConfigFormat, label: &str) -> CliResult<serde_json::Value> {
    let value: serde_json::Value = match format {
        ConfigFormat::Toml => toml::from_str(text)
            .map_err(|e| CliError::Arg(format!("invalid TOML in {label}: {e}")))?,
        ConfigFormat::Json => serde_json::from_str(text)
            .map_err(|e| CliError::Arg(format!("invalid JSON in {label}: {e}")))?,
    };

    let serde_json::Value::Object(object) = value else {
        return Err(CliError::Arg(format!(
            "config in {label} must be a table of key/value pairs"
        )));
    };

    if object.is_empty() {
        return Err(CliError::Arg(format!("config in {label} is empty")));
    }

    Ok(serde_json::Value::Object(object))
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

    let source = if label == "-" { "stdin" } else { label };
    parse_config(&text, config_format(label, &text), source)
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
    /// Print a profile's config as TOML, ready to edit and feed back in.
    Show {
        /// Name of the profile to show.
        #[arg(value_name = "NAME")]
        name: String,
    },
    /// Change a profile's name, importer, or config.
    Edit {
        /// Name of the profile to edit.
        #[arg(value_name = "NAME")]
        name: String,
        /// New name for the profile.
        #[arg(long = "name", value_name = "NEW_NAME")]
        new_name: Option<String>,
        /// New importer plugin identifier.
        #[arg(long, value_name = "PLUGIN")]
        importer: Option<String>,
        /// Path to a replacement TOML or JSON config file, or `-` for stdin.
        #[arg(long, value_name = "FILE")]
        config: Option<String>,
    },
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
        Command::Show { name } => show(ctx, name).await,
        Command::Edit {
            name,
            new_name,
            importer,
            config,
        } => edit(ctx, name, new_name, importer, config).await,
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

/// Recursively removes null-valued object keys from a JSON value.
///
/// TOML has no null, so a stored `"payee_column": null` cannot be rendered at
/// all. Dropping it is faithful rather than lossy for these configs: to serde,
/// an absent key and an explicit null mean the same thing for an `Option`
/// field. `profile show --json` remains the exact view.
///
/// Array elements are left untouched: unlike an object key, a null inside an
/// array is not equivalent to its absence — dropping it would shift every
/// later index. A null should never appear inside an array in the first
/// place; if one does, rendering as TOML fails loudly instead of silently
/// corrupting the array.
///
/// # Arguments
///
/// * `value` - The JSON value to strip.
///
/// # Returns
///
/// The same value with every null object key removed.
fn strip_nulls(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => serde_json::Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_null())
                .map(|(k, v)| (k, strip_nulls(v)))
                .collect(),
        ),
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(strip_nulls).collect())
        }
        other @ (serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_)) => other,
    }
}

/// Prints a profile's config as TOML, with its metadata as leading comments.
///
/// TOML ignores the comment lines, so `profile show bank > bank.toml` can be
/// edited and fed straight back to `profile edit bank --config bank.toml`
/// with no stripping step.
///
/// # Arguments
///
/// * `ctx` - The application context.
/// * `name` - Name of the profile to show.
///
/// # Errors
///
/// Returns [`CliError::Core`] if no profile with that name exists, or
/// [`CliError::Arg`] if the stored config cannot be rendered as TOML.
async fn show(ctx: &AppContext, name: String) -> CliResult<()> {
    let profile = ctx.profiles.find_by_name(&name).await?;

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "id": profile.id.to_string(),
            "name": profile.name,
            "importer": profile.importer,
            "config": profile.config.as_value(),
            "created_at": profile.created_at.to_string(),
        }));
    }

    let stripped = strip_nulls(profile.config.as_value().clone());
    let body = toml::to_string_pretty(&stripped)
        .map_err(|e| CliError::Arg(format!("cannot render config as TOML: {e}")))?;

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("# profile: {} ({})", profile.name, profile.id);
        println!("# importer: {}", profile.importer);
        print!("{body}");
    }
    Ok(())
}

/// Changes a profile's name, importer, or config.
///
/// Unspecified fields keep their current values. Supplying none of them is an
/// error rather than a silent no-op.
///
/// # Arguments
///
/// * `ctx` - The application context.
/// * `name` - Name of the profile to edit.
/// * `new_name` - Optional replacement name.
/// * `importer` - Optional replacement importer identifier.
/// * `config` - Optional path to a replacement config file, or `-` for stdin.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if no field was supplied or a config file cannot
/// be read, or [`CliError::Core`] if the profile does not exist or the new
/// name is already taken.
async fn edit(
    ctx: &AppContext,
    name: String,
    new_name: Option<String>,
    importer: Option<String>,
    config: Option<String>,
) -> CliResult<()> {
    if new_name.is_none() && importer.is_none() && config.is_none() {
        return Err(CliError::Arg(
            "nothing to change: pass at least one of --name, --importer, --config".to_owned(),
        ));
    }

    let profile = ctx.profiles.find_by_name(&name).await?;
    let importer_supplied = importer.is_some();
    let next_name = new_name.unwrap_or_else(|| profile.name.clone());
    let next_importer = importer.unwrap_or_else(|| profile.importer.clone());
    let next_config = match config {
        Some(label) => bc_core::ImportConfig::from_value(load_config(&label)?),
        None => profile.config.clone(),
    };

    ctx.profiles
        .update(&profile.id, &next_name, &next_importer, next_config)
        .await?;
    if importer_supplied {
        warn_unknown_importer(ctx, &next_importer);
    }

    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "id": profile.id.to_string(),
            "name": next_name,
            "importer": next_importer,
        }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Updated import profile '{next_name}'.");
    }
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
        return crate::output::print_json(&serde_json::json!({
            "removed": {
                "id": profile.id.to_string(),
                "name": name,
            },
        }));
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
    fn an_empty_table_is_rejected() {
        let empty_toml_err = parse_config("", ConfigFormat::Toml, "stdin")
            .expect_err("an empty TOML document must be rejected");
        assert!(
            empty_toml_err.to_string().contains("stdin"),
            "error must name the source, got: {empty_toml_err}"
        );
        assert!(
            empty_toml_err.to_string().contains("empty"),
            "error must say the config is empty, got: {empty_toml_err}"
        );

        let empty_json_err = parse_config("{}", ConfigFormat::Json, "bank.json")
            .expect_err("an empty JSON object must be rejected");
        assert!(
            empty_json_err.to_string().contains("bank.json"),
            "error must name the source, got: {empty_json_err}"
        );
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

    #[test]
    fn strip_nulls_removes_null_entries_recursively() {
        let value = serde_json::json!({
            "account": "Assets:Bank:Checking",
            "payee_column": null,
            "preamble": { "strategy": "none", "lines": null },
        });

        let stripped = strip_nulls(value);

        assert_eq!(
            stripped,
            serde_json::json!({
                "account": "Assets:Bank:Checking",
                "preamble": { "strategy": "none" },
            })
        );
    }

    #[test]
    fn strip_nulls_preserves_nulls_inside_arrays() {
        let value = serde_json::json!({
            "columns": ["Date", null, "Amount"],
            "nested": [{ "name": "Date", "skip": null }],
        });

        let stripped = strip_nulls(value);

        assert_eq!(
            stripped,
            serde_json::json!({
                "columns": ["Date", null, "Amount"],
                "nested": [{ "name": "Date" }],
            })
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known structure")]
    fn a_stripped_config_renders_as_toml() {
        let value = serde_json::json!({
            "account": "Assets:Bank:Checking",
            "payee_column": null,
            "preamble": { "strategy": "skip_lines", "lines": 3_i32 },
        });

        let rendered = toml::to_string_pretty(&strip_nulls(value)).expect("config renders as TOML");
        let reparsed = parse_config(&rendered, ConfigFormat::Toml, "rendered")
            .expect("rendered TOML reparses");

        assert_eq!(reparsed["account"], "Assets:Bank:Checking");
        assert_eq!(reparsed["preamble"]["lines"], 3_i32);
        assert!(reparsed.get("payee_column").is_none());
    }
}
