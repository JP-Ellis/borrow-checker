//! Tag management sub-commands: create, rename, delete, list.

use clap::Subcommand;

use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// Arguments for the `tag` subcommand.
#[non_exhaustive]
#[derive(Debug, clap::Args)]
pub struct Args {
    /// The tag operation to perform.
    #[command(subcommand)]
    pub command: Command,
}

/// Available tag operations.
#[derive(Debug, Subcommand)]
#[non_exhaustive]
pub enum Command {
    /// Create a tag hierarchy from a colon-path (e.g. `person:josh`).
    Create {
        /// The colon-joined tag path to create.
        path: String,
    },
    /// Rename a tag's leaf segment.
    Rename {
        /// The tag ID to rename.
        id: String,
        /// The new leaf name.
        new_name: String,
    },
    /// Delete a tag and its subtree.
    Delete {
        /// The tag ID to delete.
        id: String,
    },
    /// List all tags as ID + resolved path.
    List,
}

/// Executes the `tag` subcommand.
///
/// # Errors
///
/// Returns a [`crate::error::CliError`] if a service call fails or an argument is invalid.
pub async fn execute(args: Args, ctx: &AppContext) -> CliResult<()> {
    match args.command {
        Command::Create { path } => create(ctx, path).await,
        Command::Rename { id, new_name } => rename(ctx, id, new_name).await,
        Command::Delete { id } => delete(ctx, id).await,
        Command::List => list(ctx).await,
    }
}

/// Creates a tag hierarchy from a colon-separated path string.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the path is not a valid [`bc_models::TagPath`],
/// or propagates [`CliError::Core`] from the tag service.
async fn create(ctx: &AppContext, path: String) -> CliResult<()> {
    let parsed = path
        .parse::<bc_models::TagPath>()
        .map_err(|e| CliError::Arg(format!("invalid tag path '{path}': {e}")))?;
    let id = ctx.tags.create_path(&parsed).await?;
    if ctx.json {
        return crate::output::print_json(&id.to_string());
    }
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Created tag {path} ({id})");
    }
    Ok(())
}

/// Renames a tag's leaf segment.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the ID is not a valid [`bc_models::TagId`],
/// or propagates [`CliError::Core`] from the tag service.
async fn rename(ctx: &AppContext, id: String, new_name: String) -> CliResult<()> {
    let tag_id = id
        .parse::<bc_models::TagId>()
        .map_err(|e| CliError::Arg(format!("invalid tag id '{id}': {e}")))?;
    ctx.tags.rename(&tag_id, &new_name).await?;
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Renamed {id} to {new_name}");
    }
    Ok(())
}

/// Deletes a tag and its entire subtree.
///
/// # Errors
///
/// Returns [`CliError::Arg`] if the ID is not a valid [`bc_models::TagId`],
/// or propagates [`CliError::Core`] from the tag service.
async fn delete(ctx: &AppContext, id: String) -> CliResult<()> {
    let tag_id = id
        .parse::<bc_models::TagId>()
        .map_err(|e| CliError::Arg(format!("invalid tag id '{id}': {e}")))?;
    ctx.tags.delete(&tag_id).await?;
    #[expect(clippy::print_stdout, reason = "CLI output")]
    {
        println!("Deleted tag {id}");
    }
    Ok(())
}

/// Lists all tags as a table of ID and resolved path.
///
/// # Errors
///
/// Propagates [`CliError::Core`] from the tag service or [`CliError::Json`]
/// from JSON serialisation.
async fn list(ctx: &AppContext) -> CliResult<()> {
    let forest = ctx.tags.forest().await?;
    let tags = ctx.tags.list().await?;
    let rows: Vec<Vec<String>> = tags
        .iter()
        .map(|t| {
            let path = forest
                .path_of(t.id())
                .map_or_else(|| t.name().to_owned(), |p| p.to_string());
            vec![t.id().to_string(), path]
        })
        .collect();
    if ctx.json {
        return crate::output::print_json(&rows);
    }
    if rows.is_empty() {
        #[expect(clippy::print_stdout, reason = "CLI output")]
        {
            println!("No tags.");
        }
        return Ok(());
    }
    crate::output::print_table(&["ID", "PATH"], &rows);
    Ok(())
}
