//! Metadata key-value arguments and the metadata key registry.
//!
//! This module owns the `--meta KEY=VALUE` grammar shared by the `transaction`
//! commands. The type a value takes is the registry's whenever the registry
//! holds the key, and is inferred from the value's syntax otherwise, which is
//! the type auto-registration then records.
//!
//! Nothing here decides whether a value *fits*. A value the registered type
//! cannot read is handed on as text, and `bc-core` derives the stored
//! [`bc_models::MetaEntry::mismatched`] flag from the value against the
//! registered type. The flag is the store's verdict, so no ingress path
//! asserts it.

use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaType;
use bc_models::MetaValue;
use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// Arguments for the `meta` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The registry operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available registry operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// List every registered key with its type.
    List,
    /// Change a key's type, refitting every entry stored under it.
    ///
    /// Widening to `text` keeps every value. Narrowing parses each one and
    /// flags whatever will not read as the new type; nothing is discarded.
    Retype {
        /// The registered key to retype.
        key: String,
        /// The type it should hold from now on.
        #[arg(value_enum, value_name = "TYPE")]
        ty: TypeArg,
    },
    /// Rename a key, carrying every entry under it across.
    Rename {
        /// The registered key to rename.
        from: String,
        /// Its new name.
        to: String,
    },
}

/// CLI representation of [`MetaType`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum TypeArg {
    /// Free text; anything reads as this, so a text key never mismatches.
    Text,
    /// An arbitrary-precision decimal number.
    Number,
    /// `true` or `false`.
    Boolean,
    /// A `YYYY-MM-DD` calendar date.
    Date,
    /// An RFC 3339 instant.
    Timestamp,
    /// A decimal value and a commodity code, e.g. `42.00 AUD`.
    Amount,
    /// A reference to an account, written as a path or an id.
    Account,
}

impl From<TypeArg> for MetaType {
    #[inline]
    fn from(arg: TypeArg) -> Self {
        match arg {
            TypeArg::Text => Self::Text,
            TypeArg::Number => Self::Number,
            TypeArg::Boolean => Self::Boolean,
            TypeArg::Date => Self::Date,
            TypeArg::Timestamp => Self::Timestamp,
            TypeArg::Amount => Self::Amount,
            TypeArg::Account => Self::Account,
        }
    }
}

/// Renders a type as the name the registry stores and the CLI accepts.
///
/// # Arguments
///
/// * `ty` - The type to name.
fn type_name(ty: MetaType) -> &'static str {
    match ty {
        MetaType::Text => "text",
        MetaType::Number => "number",
        MetaType::Boolean => "boolean",
        MetaType::Date => "date",
        MetaType::Timestamp => "timestamp",
        MetaType::Amount => "amount",
        MetaType::Account => "account",
    }
}

/// Executes the `meta` subcommand.
///
/// # Errors
///
/// Returns [`CliError::Arg`] for an invalid key, and [`CliError::Core`] from
/// the registry.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::List => list(ctx).await,
        Command::Retype { key, ty } => retype(ctx, &key, ty.into()).await,
        Command::Rename { from, to } => rename(ctx, &from, &to).await,
    }
}

/// Lists every registered key.
///
/// # Errors
///
/// Returns [`CliError::Core`] from the registry and [`CliError::Json`] from
/// serialisation.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let keys = ctx.metadata.list().await?;
    if ctx.json {
        return crate::output::print_json(&keys);
    }
    if keys.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No metadata keys.");
        }
        return Ok(());
    }
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|def| {
            vec![
                def.key().to_string(),
                type_name(def.ty()).to_owned(),
                def.created_at().to_string(),
            ]
        })
        .collect();
    crate::output::print_table(&["KEY", "TYPE", "REGISTERED"], &rows);
    Ok(())
}

/// Retypes one key.
///
/// # Errors
///
/// Returns [`CliError::Arg`] for an invalid key, and [`CliError::Core`] when
/// the key is not registered or the refit fails.
async fn retype(ctx: &AppContext, raw_key: &str, ty: MetaType) -> CliResult<()> {
    let key = parse_meta_key(raw_key)?;
    // `retype` answers with the type it replaced, so naming both costs no
    // second query.
    let from = ctx.metadata.retype(&key, ty).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "key": key.as_str(),
            "from": type_name(from),
            "to": type_name(ty),
        }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        if from == ty {
            println!("Key '{key}' is already {}", type_name(ty));
        } else {
            println!("Retyped '{key}': {} -> {}", type_name(from), type_name(ty));
        }
    }
    Ok(())
}

/// Renames one key.
///
/// # Errors
///
/// Returns [`CliError::Arg`] for an invalid key, and [`CliError::Core`] when
/// `from` is not registered or `to` already is.
async fn rename(ctx: &AppContext, raw_from: &str, raw_to: &str) -> CliResult<()> {
    let from = parse_meta_key(raw_from)?;
    let to = parse_meta_key(raw_to)?;
    ctx.metadata.rename(&from, &to).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "from": from.as_str(),
            "to": to.as_str(),
        }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Renamed '{from}' to '{to}'");
    }
    Ok(())
}

/// Splits one `--meta KEY=VALUE` argument into its key and its raw value.
///
/// The split is on the **first** `=`, so a value may contain further ones. A
/// key never can: [`MetaKey`] admits only `[a-z][a-z0-9_-]*`.
///
/// # Arguments
///
/// * `spec` - One `--meta` argument, as typed.
///
/// # Returns
///
/// The normalised key and the value text, which may be empty.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when `spec` holds no `=`, or when the key is not
/// a valid metadata key.
pub(crate) fn parse_meta_arg(spec: &str) -> CliResult<(MetaKey, String)> {
    let Some((raw_key, value)) = spec.split_once('=') else {
        return Err(CliError::Arg(format!(
            "invalid --meta '{spec}': expected KEY=VALUE"
        )));
    };
    let key = MetaKey::new(raw_key)
        .map_err(|e| CliError::Arg(format!("invalid metadata key '{raw_key}': {e}")))?;
    Ok((key, value.to_owned()))
}

/// Parses one `--clear-meta KEY` argument.
///
/// # Arguments
///
/// * `raw_key` - The key to clear, as typed.
///
/// # Returns
///
/// The normalised key.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when `raw_key` is not a valid metadata key.
pub(crate) fn parse_meta_key(raw_key: &str) -> CliResult<MetaKey> {
    MetaKey::new(raw_key)
        .map_err(|e| CliError::Arg(format!("invalid metadata key '{raw_key}': {e}")))
}

/// Reports whether `raw` is a decimal value followed by a commodity code.
///
/// [`bc_models::CommodityCode`] is unvalidated free text, so
/// [`MetaType::Amount`]'s parser accepts `3 kids`. Inference must not: a key
/// whose first value happens to read as a count of something would be
/// registered as an amount for good.
///
/// # Arguments
///
/// * `raw` - The value text.
fn looks_like_amount(raw: &str) -> bool {
    let Some((value, commodity)) = raw.split_once(' ') else {
        return false;
    };
    if value.parse::<rust_decimal::Decimal>().is_err() {
        return false;
    }
    let mut chars = commodity.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '.' || c == '_')
}

/// Infers the type of a value written under a key the registry does not hold.
///
/// The first matching rule wins: `true`/`false` exactly, then a date, a
/// timestamp, a decimal, an amount, and text for everything else.
///
/// [`MetaType::Account`] is never inferred. An account path is
/// indistinguishable from text and an account id is opaque, so guessing would
/// mint account-typed keys out of prose. Registering one takes a write
/// followed by `borrow-checker meta retype KEY account`.
///
/// # Arguments
///
/// * `raw` - The value text.
///
/// # Returns
///
/// A type whose [`MetaType::parse_value`] accepts `raw`.
pub(crate) fn infer_type(raw: &str) -> MetaType {
    if raw == "true" || raw == "false" {
        MetaType::Boolean
    } else if raw.parse::<jiff::civil::Date>().is_ok() {
        MetaType::Date
    } else if raw.parse::<jiff::Timestamp>().is_ok() {
        MetaType::Timestamp
    } else if raw.parse::<rust_decimal::Decimal>().is_ok() {
        MetaType::Number
    } else if looks_like_amount(raw) {
        MetaType::Amount
    } else {
        MetaType::Text
    }
}

/// Resolves `raw` against the account tree for a key registered as
/// [`MetaType::Account`].
///
/// Coercion in `bc-core` is pure and holds no database, so it cannot turn a
/// path into an id; the caller owns that, and at the CLI the caller is this
/// function. An id is taken as-is, a path is resolved, and anything else is
/// handed on as text for the store to flag.
///
/// # Arguments
///
/// * `ctx` - The application context, for the account tree.
/// * `key` - The key being written, for the warning.
/// * `raw` - The value text.
///
/// # Errors
///
/// Returns [`CliError::Core`] when the account tree cannot be read.
async fn resolve_account(ctx: &AppContext, key: &MetaKey, raw: &str) -> CliResult<MetaValue> {
    if let Ok(id) = raw.parse::<bc_models::AccountId>() {
        return Ok(MetaValue::Account(id));
    }
    if let Ok(path) = bc_core::AccountPath::parse(raw) {
        let resolver = bc_core::AccountResolver::load(&ctx.accounts).await?;
        if let bc_core::Resolution::Resolved { id, .. } = resolver.resolve(&path) {
            return Ok(MetaValue::Account(id));
        }
    }
    tracing::warn!(
        key = key.as_str(),
        value = raw,
        "no account of that path or id; storing the value as flagged text"
    );
    Ok(MetaValue::Text(raw.to_owned()))
}

/// Builds one entry, taking its type from the registry or from `raw`.
///
/// # Arguments
///
/// * `ctx` - The application context, for the registry and the account tree.
/// * `key` - The key to file the value under.
/// * `raw` - The value text.
///
/// # Returns
///
/// An unflagged entry. A value the registered type cannot read becomes
/// [`MetaValue::Text`], which the write path stores flagged.
///
/// # Errors
///
/// Returns [`CliError::Core`] when the registry or the account tree cannot be
/// read.
pub(crate) async fn entry_for(ctx: &AppContext, key: MetaKey, raw: &str) -> CliResult<MetaEntry> {
    let ty = match ctx.metadata.get(&key).await? {
        Some(def) => def.ty(),
        None => infer_type(raw),
    };
    let value = if ty == MetaType::Account {
        resolve_account(ctx, &key, raw).await?
    } else {
        ty.parse_value(raw).unwrap_or_else(|e| {
            tracing::warn!(
                key = key.as_str(),
                error = %e,
                "value does not fit the key's registered type; storing it as flagged text"
            );
            MetaValue::Text(raw.to_owned())
        })
    };
    Ok(MetaEntry::new(key, value))
}

/// Builds one entry per `--meta` argument, in argument order.
///
/// # Arguments
///
/// * `ctx` - The application context.
/// * `specs` - The `--meta` arguments, as typed.
///
/// # Errors
///
/// Returns [`CliError::Arg`] for a malformed argument, and [`CliError::Core`]
/// when the registry or the account tree cannot be read.
pub(crate) async fn entries_for(ctx: &AppContext, specs: &[String]) -> CliResult<Vec<MetaEntry>> {
    let mut entries = Vec::with_capacity(specs.len());
    for spec in specs {
        let (key, raw) = parse_meta_arg(spec)?;
        entries.push(entry_for(ctx, key, &raw).await?);
    }
    Ok(entries)
}

/// Applies an amendment's `--meta` and `--clear-meta` arguments to a stored
/// list.
///
/// A key named by `entries` keeps the position its first stored entry held,
/// with every stored entry under it dropped and every new one inserted there;
/// a key absent from `current` appends. A key named by `cleared` loses every
/// entry. Everything else keeps its value and its position, because position
/// orders an owner's entries globally and is the display order.
///
/// # Arguments
///
/// * `current` - The stored list.
/// * `entries` - The replacement entries, in argument order.
/// * `cleared` - The keys to remove entirely.
///
/// # Returns
///
/// The amended list.
pub(crate) fn apply_changes(
    current: &bc_models::Metadata,
    entries: &[MetaEntry],
    cleared: &[MetaKey],
) -> bc_models::Metadata {
    let replaced: std::collections::HashSet<MetaKey> =
        entries.iter().map(|e| e.key().clone()).collect();
    let mut spliced: std::collections::HashSet<MetaKey> = std::collections::HashSet::new();

    let mut out: Vec<MetaEntry> = Vec::new();
    for entry in current.iter() {
        if cleared.contains(entry.key()) {
            continue;
        }
        if replaced.contains(entry.key()) {
            if spliced.insert(entry.key().clone()) {
                out.extend(entries.iter().filter(|e| e.key() == entry.key()).cloned());
            }
            continue;
        }
        out.push(entry.clone());
    }
    for entry in entries {
        if !spliced.contains(entry.key()) {
            out.push(entry.clone());
        }
    }
    bc_models::Metadata::new(out)
}

/// Renders one entry as `key=value`, marking a flagged one with a leading `!`.
///
/// An account value renders as its path when `resolver` holds one, and as its
/// id otherwise — which is what a tombstoned entry, whose account has been
/// deleted, has left to show.
///
/// # Arguments
///
/// * `entry` - The entry to render.
/// * `resolver` - The account tree, when one has been loaded.
pub(crate) fn render_entry(
    entry: &MetaEntry,
    resolver: Option<&bc_core::AccountResolver>,
) -> String {
    let value = match *entry.value() {
        MetaValue::Account(ref id) => resolver
            .and_then(|r| r.path_of(id))
            .map_or_else(|| id.to_string(), str::to_owned),
        MetaValue::Text(_)
        | MetaValue::Number(_)
        | MetaValue::Boolean(_)
        | MetaValue::Date(_)
        | MetaValue::Timestamp(_)
        | MetaValue::Amount(_) => entry.value().canonical(),
    };
    let flag = if entry.mismatched() { "!" } else { "" };
    format!("{flag}{}={value}", entry.key())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[test]
    fn meta_arg_splits_on_the_first_equals() {
        let (key, value) = parse_meta_arg("payee=Generic Grocer").expect("valid spec");
        assert_eq!(key.as_str(), "payee");
        assert_eq!(value, "Generic Grocer");
    }

    #[test]
    fn meta_arg_keeps_later_equals_in_the_value() {
        let (key, value) = parse_meta_arg("query=a=b=c").expect("valid spec");
        assert_eq!(key.as_str(), "query");
        assert_eq!(value, "a=b=c");
    }

    #[test]
    fn meta_arg_normalises_the_key() {
        let (key, _value) = parse_meta_arg("Invoice=1502").expect("valid spec");
        assert_eq!(key.as_str(), "invoice");
    }

    #[test]
    fn meta_arg_accepts_an_empty_value() {
        let (_key, value) = parse_meta_arg("note=").expect("valid spec");
        assert_eq!(value, "");
    }

    #[test]
    fn meta_arg_without_an_equals_is_rejected() {
        let err = parse_meta_arg("payee").expect_err("no '=' is not a pair");
        assert_eq!(
            err.to_string(),
            "invalid --meta 'payee': expected KEY=VALUE"
        );
    }

    #[test]
    fn meta_arg_with_an_invalid_key_is_rejected() {
        let err = parse_meta_arg("1nvoice=1502").expect_err("keys start with a letter");
        assert_eq!(
            err.to_string(),
            "invalid metadata key '1nvoice': metadata key must start with a letter, found '1'"
        );
    }

    #[rstest]
    #[case("true", MetaType::Boolean)]
    #[case("false", MetaType::Boolean)]
    #[case("2026-01-15", MetaType::Date)]
    #[case("2023-11-14T22:13:20Z", MetaType::Timestamp)]
    #[case("1502", MetaType::Number)]
    #[case("-42.50", MetaType::Number)]
    #[case("42.00 AUD", MetaType::Amount)]
    #[case("Generic Grocer", MetaType::Text)]
    #[case("", MetaType::Text)]
    #[case("True", MetaType::Text)]
    fn a_new_key_takes_the_type_its_value_reads_as(#[case] raw: &str, #[case] expected: MetaType) {
        assert_eq!(infer_type(raw), expected);
    }

    /// A commodity code is unvalidated free text, so the amount parser reads
    /// `3 kids` as three of them. Inference is stricter than the parser on
    /// purpose.
    #[rstest]
    #[case("3 kids")]
    #[case("2 large boxes")]
    fn a_count_of_something_is_not_inferred_as_an_amount(#[case] raw: &str) {
        assert_eq!(infer_type(raw), MetaType::Text);
    }

    /// Every inferred type must be able to read the value it was inferred from,
    /// or auto-registration would record a type the first write cannot satisfy.
    #[rstest]
    #[case("true")]
    #[case("2026-01-15")]
    #[case("1502")]
    #[case("42.00 AUD")]
    #[case("Generic Grocer")]
    fn an_inferred_type_reads_the_value_it_was_inferred_from(#[case] raw: &str) {
        let parsed = infer_type(raw).parse_value(raw).expect("inferred type");
        assert_eq!(parsed.canonical(), raw);
    }

    /// An account path is text as far as inference is concerned; the key has to
    /// be retyped before a path resolves.
    #[test]
    fn an_account_path_is_not_inferred_as_an_account() {
        assert_eq!(infer_type("Assets:Bank:Savings"), MetaType::Text);
    }

    #[test]
    fn a_flagged_entry_renders_with_a_leading_bang() {
        let key = MetaKey::new("invoice").expect("valid key");
        let entry = MetaEntry::mismatch(key, "not-a-number");
        assert_eq!(render_entry(&entry, None), "!invoice=not-a-number");
    }

    #[test]
    fn an_entry_renders_as_its_canonical_form() {
        let key = MetaKey::new("invoice").expect("valid key");
        let entry = MetaEntry::new(
            key,
            MetaValue::Number(rust_decimal::Decimal::new(150_250, 2)),
        );
        assert_eq!(render_entry(&entry, None), "invoice=1502.50");
    }
}
