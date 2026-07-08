//! Import sub-command.

use core::str::FromStr as _;

use crate::context::AppContext;
use crate::error::CliResult;

/// Arguments for the `import` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// Name of the import profile to use.
    #[arg(long, value_name = "NAME")]
    pub profile: String,

    /// Account to import transactions into.
    #[arg(long, value_name = "ACCOUNT")]
    pub account: String,
}

/// Executes the `import` subcommand.
///
/// # Errors
///
/// Returns [`crate::error::CliError`] if the profile does not exist, the
/// file cannot be read, or the importer fails to parse it.
#[inline]
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    let account_id = bc_models::AccountId::from_str(&args.account).map_err(|e| {
        crate::error::CliError::Arg(format!("invalid account ID '{}': {e}", args.account))
    })?;

    // Find the import profile by name.
    let profiles = ctx.profiles.list_all().await?;
    let profile = profiles
        .iter()
        .find(|p| p.name == args.profile)
        .ok_or_else(|| {
            crate::error::CliError::Core(bc_core::BcError::NotFound(format!(
                "import profile '{}'",
                args.profile
            )))
        })?;

    // Create the importer.
    let importer = ctx
        .importers
        .create_for_name(&profile.importer)
        .ok_or_else(|| {
            crate::error::CliError::Arg(format!(
                "unknown importer '{}' for profile '{}'",
                profile.importer, profile.name
            ))
        })?;

    // Source and parse the profile's files (the importer reads them itself).
    let raw_txs = importer
        .import(&profile.config)
        .map_err(|e| crate::error::CliError::Arg(format!("import parse error: {e}")))?;

    let count =
        bc_core::execute_import(&ctx.transactions, &ctx.sources, &account_id, &raw_txs).await?;
    if ctx.json {
        return crate::output::print_json(&serde_json::json!({ "imported": count }));
    }

    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Imported {count} transactions.");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    #[test]
    fn args_parse_without_a_file_positional() {
        // Wrapper needed because `Args` is a subcommand arg group.
        #[derive(clap::Parser)]
        struct Wrap {
            #[command(flatten)]
            args: super::Args,
        }

        let ok = Wrap::try_parse_from(["x", "--profile", "nab", "--account", "acc-1"]);
        assert!(ok.is_ok(), "profile + account parse with no positional");

        let rejected = Wrap::try_parse_from([
            "x",
            "--profile",
            "nab",
            "--account",
            "acc-1",
            "some/file.csv",
        ]);
        assert!(
            rejected.is_err(),
            "a trailing file path is no longer accepted"
        );
    }
}
