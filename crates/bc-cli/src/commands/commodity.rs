//! Commodity registry sub-commands: list, create, update, delete.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliError;
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
    /// Register a new commodity.
    Create(CreateArgs),
    /// Change a registered commodity's metadata or aliases.
    Update(UpdateArgs),
    /// Remove a commodity, provided nothing references it.
    Delete {
        /// The commodity's code, symbol, or alias.
        #[arg(value_name = "MARKER")]
        marker: String,
    },
}

/// Arguments for `commodity create`.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct CreateArgs {
    /// Canonical code for the commodity (e.g. `BNB`).
    #[arg(value_name = "CODE")]
    pub code: String,
    /// Human-readable full name (e.g. `Bitcoin`).
    #[arg(long)]
    pub name: Option<String>,
    /// Display symbol used when formatting amounts (e.g. `$`).
    #[arg(long)]
    pub symbol: Option<String>,
    /// Exchange or market where the commodity trades (e.g. `NASDAQ`).
    #[arg(long)]
    pub exchange: Option<String>,
    /// Free-text description (e.g. an ISIN).
    #[arg(long)]
    pub description: Option<String>,
    /// Additional input marker for this commodity. Repeatable.
    #[arg(long = "alias", value_name = "ALIAS")]
    pub aliases: Vec<String>,
    /// Number of minor-unit digits shown when formatting amounts.
    #[arg(long, default_value_t = 2)]
    pub decimals: u8,
    /// Treat the commodity as a non-ISO-4217 asset (e.g. crypto, equities).
    #[arg(long = "no-iso")]
    pub no_iso: bool,
    /// Place the display symbol after the amount (e.g. `100 ETH`).
    #[arg(long)]
    pub symbol_after: bool,
    /// First date from which the commodity is valid (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub active_from: Option<String>,
    /// Last date on which the commodity is valid (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub active_until: Option<String>,
}

/// Arguments for `commodity update`.
///
/// Every field is optional: an omitted flag leaves the stored value untouched.
/// The two booleans take explicit on/off pairs, since a bare flag can only ever
/// set a boolean true and an update must be able to set it back to false.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "iso/no-iso and symbol-after/no-symbol-after are clap on/off pairs, not independent flags"
)]
pub struct UpdateArgs {
    /// The commodity's code, symbol, or alias.
    #[arg(value_name = "MARKER")]
    pub marker: String,
    /// Replace the human-readable name. Pass an empty string to clear it.
    #[arg(long)]
    pub name: Option<String>,
    /// Replace the display symbol. Pass an empty string to clear it.
    #[arg(long)]
    pub symbol: Option<String>,
    /// Replace the exchange. Pass an empty string to clear it.
    #[arg(long)]
    pub exchange: Option<String>,
    /// Replace the description. Pass an empty string to clear it.
    #[arg(long)]
    pub description: Option<String>,
    /// Add an input marker. Repeatable.
    #[arg(long = "add-alias", value_name = "ALIAS")]
    pub add_alias: Vec<String>,
    /// Remove an existing input marker. Repeatable.
    #[arg(long = "remove-alias", value_name = "ALIAS")]
    pub remove_alias: Vec<String>,
    /// Replace the minor-unit digit count.
    #[arg(long)]
    pub decimals: Option<u8>,
    /// Treat the commodity as an ISO 4217 currency.
    #[arg(long, overrides_with = "no_iso")]
    pub iso: bool,
    /// Treat the commodity as a non-ISO-4217 asset.
    #[arg(long = "no-iso", overrides_with = "iso")]
    pub no_iso: bool,
    /// Place the display symbol after the amount.
    #[arg(long, overrides_with = "no_symbol_after")]
    pub symbol_after: bool,
    /// Place the display symbol before the amount.
    #[arg(long = "no-symbol-after", overrides_with = "symbol_after")]
    pub no_symbol_after: bool,
    /// Replace the first valid date (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub active_from: Option<String>,
    /// Replace the last valid date (YYYY-MM-DD).
    #[arg(long, value_name = "YYYY-MM-DD")]
    pub active_until: Option<String>,
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
        Command::Create(create_args) => create(ctx, create_args).await,
        Command::Update(update_args) => update(ctx, update_args).await,
        Command::Delete { marker } => delete(ctx, &marker).await,
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

/// Parses an optional `YYYY-MM-DD` argument, preserving `None`.
///
/// [`crate::commands::parse_date_or_today`] substitutes today's date for
/// `None`; an unset commodity validity bound must stay unset instead.
///
/// # Arguments
///
/// * `flag` - The flag name, used in the error message.
/// * `value` - The optional date string.
///
/// # Returns
///
/// The parsed date, or `None` when `value` is `None`.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the string is not a valid `YYYY-MM-DD` date.
fn parse_optional_date(flag: &str, value: Option<&str>) -> CliResult<Option<jiff::civil::Date>> {
    value
        .map(|d| {
            d.parse::<jiff::civil::Date>()
                .map_err(|e| CliError::Arg(format!("invalid --{flag} '{d}': {e}")))
        })
        .transpose()
}

/// Registers a new commodity.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
/// * `args` - The parsed `create` arguments.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if a date argument is malformed, or propagates
/// [`CliError::Core`] from the commodity service — notably
/// [`bc_core::BcError::MarkerConflict`] when the code, symbol, or an alias is
/// already taken.
async fn create(ctx: &AppContext, args: CreateArgs) -> CliResult<()> {
    let active_from = parse_optional_date("active-from", args.active_from.as_deref())?;
    let active_until = parse_optional_date("active-until", args.active_until.as_deref())?;

    let commodity = bc_models::Commodity::builder()
        .code(args.code.clone())
        .maybe_name(args.name)
        .maybe_symbol(args.symbol)
        .maybe_exchange(args.exchange)
        .maybe_description(args.description)
        .aliases(args.aliases)
        .decimals(args.decimals)
        .is_iso(!args.no_iso)
        .symbol_after(args.symbol_after)
        .maybe_active_from(active_from)
        .maybe_active_until(active_until)
        .build();

    let stored = ctx.commodities.create(&commodity).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({
            "id": stored.id().to_string(),
            "code": stored.code(),
        }));
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created commodity {} ({})", stored.code(), stored.id());
    }
    Ok(())
}

/// Resolves a user-supplied marker to the commodity it names.
///
/// Reuses [`bc_core::CommodityResolver`], the same matcher the importer uses,
/// so a marker an import accepts is a marker the CLI accepts: **codes match
/// case-insensitively; symbols and aliases match exactly**.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
/// * `marker` - A commodity code, symbol, or alias.
///
/// # Returns
///
/// The stored commodity the marker names.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if nothing matches, or propagates
/// [`CliError::Core`] from the commodity service.
async fn find(ctx: &AppContext, marker: &str) -> CliResult<bc_models::Commodity> {
    let all = ctx.commodities.list_all().await?;
    let resolver = bc_core::CommodityResolver::from_commodities(&all);
    let code = resolver
        .resolve(&bc_models::CommodityCode::from(marker))
        .ok_or_else(|| CliError::Arg(format!("no commodity matching '{marker}'")))?
        .to_owned();
    all.into_iter()
        .find(|c| c.code() == code)
        .ok_or_else(|| CliError::Arg(format!("no commodity matching '{marker}'")))
}

/// Deletes the commodity a marker names.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
/// * `marker` - A commodity code, symbol, or alias.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the marker matches nothing, or propagates
/// [`CliError::Core`] — notably [`bc_core::BcError::CommodityInUse`], whose
/// message summarises the remaining references.
async fn delete(ctx: &AppContext, marker: &str) -> CliResult<()> {
    let target = find(ctx, marker).await?;
    ctx.commodities.delete(target.id()).await?;
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Deleted commodity {}", target.code());
    }
    Ok(())
}

/// Applies `--add-alias` / `--remove-alias` to a stored alias list.
///
/// Removal is validated: naming an alias the commodity does not hold is an
/// error rather than a no-op, so a typo in a script surfaces immediately.
/// Adding an alias already present is likewise rejected, since the resulting
/// duplicate would be silently collapsed.
///
/// # Arguments
///
/// * `stored` - The commodity's current aliases.
/// * `add` - Aliases to add.
/// * `remove` - Aliases to remove.
///
/// # Returns
///
/// The merged alias list, preserving the stored order and appending additions.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if an alias to remove is absent, or an alias to
/// add is already present.
fn merge_aliases(stored: &[String], add: &[String], remove: &[String]) -> CliResult<Vec<String>> {
    for alias in remove {
        if !stored.contains(alias) {
            return Err(CliError::Arg(format!("no alias '{alias}' to remove")));
        }
    }
    for alias in add {
        if stored.contains(alias) {
            return Err(CliError::Arg(format!("alias '{alias}' already set")));
        }
    }
    let mut merged: Vec<String> = stored
        .iter()
        .filter(|a| !remove.contains(a))
        .cloned()
        .collect();
    merged.extend(add.iter().cloned());
    Ok(merged)
}

/// Normalises an optional text argument, mapping an empty string to `None`.
///
/// `--symbol ''` clears the field; an omitted flag leaves `stored` in place.
///
/// # Arguments
///
/// * `flag` - The supplied value, if any.
/// * `stored` - The currently persisted value.
///
/// # Returns
///
/// The value to persist.
fn text_field(flag: Option<String>, stored: Option<&str>) -> Option<String> {
    match flag {
        Some(v) if v.is_empty() => None,
        Some(v) => Some(v),
        None => stored.map(str::to_owned),
    }
}

/// Updates a registered commodity's metadata and aliases.
///
/// # Arguments
///
/// * `ctx` - The shared application context.
/// * `args` - The parsed `update` arguments.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the marker matches nothing, a date is
/// malformed, or an alias edit is invalid; propagates [`CliError::Core`] from
/// the commodity service.
async fn update(ctx: &AppContext, args: UpdateArgs) -> CliResult<()> {
    let stored = find(ctx, &args.marker).await?;

    let is_iso = if args.iso {
        true
    } else if args.no_iso {
        false
    } else {
        stored.is_iso()
    };
    let symbol_after = if args.symbol_after {
        true
    } else if args.no_symbol_after {
        false
    } else {
        stored.symbol_after()
    };

    let aliases = merge_aliases(stored.aliases(), &args.add_alias, &args.remove_alias)?;

    let active_from = match args.active_from.as_deref() {
        Some(v) => parse_optional_date("active-from", Some(v))?,
        None => stored.active_from(),
    };
    let active_until = match args.active_until.as_deref() {
        Some(v) => parse_optional_date("active-until", Some(v))?,
        None => stored.active_until(),
    };

    let edited = bc_models::Commodity::builder()
        .id(stored.id().clone())
        .code(stored.code().to_owned())
        .maybe_name(text_field(args.name, stored.name()))
        .maybe_symbol(text_field(args.symbol, stored.symbol()))
        .maybe_exchange(text_field(args.exchange, stored.exchange()))
        .maybe_description(text_field(args.description, stored.description()))
        .aliases(aliases)
        .decimals(args.decimals.unwrap_or(stored.decimals()))
        .is_iso(is_iso)
        .symbol_after(symbol_after)
        .maybe_active_from(active_from)
        .maybe_active_until(active_until)
        .build();

    ctx.commodities.update(&edited).await?;
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Updated commodity {}", stored.code());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn merging_appends_additions_after_survivors() {
        let merged = merge_aliases(
            &["US$".to_owned(), "usd".to_owned()],
            &["dollar".to_owned()],
            &["usd".to_owned()],
        )
        .expect("merge");
        assert_eq!(merged, vec!["US$".to_owned(), "dollar".to_owned()]);
    }

    #[test]
    fn removing_an_absent_alias_is_an_error() {
        let err = merge_aliases(&["US$".to_owned()], &[], &["nope".to_owned()])
            .expect_err("absent alias rejected");
        assert_eq!(err.to_string(), "no alias 'nope' to remove");
    }

    #[test]
    fn adding_a_present_alias_is_an_error() {
        let err = merge_aliases(&["US$".to_owned()], &["US$".to_owned()], &[])
            .expect_err("duplicate alias rejected");
        assert_eq!(err.to_string(), "alias 'US$' already set");
    }

    #[test]
    fn alias_removal_is_case_sensitive() {
        let err = merge_aliases(&["US$".to_owned()], &[], &["us$".to_owned()])
            .expect_err("aliases match exactly");
        assert_eq!(err.to_string(), "no alias 'us$' to remove");
    }

    #[test]
    fn an_omitted_text_field_keeps_the_stored_value() {
        assert_eq!(text_field(None, Some("A$")), Some("A$".to_owned()));
    }

    #[test]
    fn an_empty_text_field_clears_the_stored_value() {
        assert_eq!(text_field(Some(String::new()), Some("A$")), None);
    }

    #[test]
    fn a_supplied_text_field_replaces_the_stored_value() {
        assert_eq!(
            text_field(Some("AU$".to_owned()), Some("A$")),
            Some("AU$".to_owned())
        );
    }
}
