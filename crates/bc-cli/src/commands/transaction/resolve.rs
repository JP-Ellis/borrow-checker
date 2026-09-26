//! Looking up the accounts, metadata keys and tags a command names.

use core::str::FromStr as _;
use std::collections::HashMap;

use super::changes::Changes;
use super::changes::Resolved;
use super::spec;
use crate::commands::meta;
use crate::context::AppContext;
use crate::error::CliError;
use crate::error::CliResult;

/// Tag IDs for every tag a command names.
#[derive(Debug, Default)]
pub(super) struct Tags {
    /// ID per text as typed; an untagged path naming nothing is absent.
    ids: HashMap<String, bc_models::TagId>,
    /// Paths this command created, sorted.
    created: Vec<String>,
}

impl Tags {
    /// The ID for `text`, as typed.
    pub(super) fn get(&self, text: &str) -> Option<&bc_models::TagId> {
        self.ids.get(text)
    }

    /// The paths this command created, sorted.
    pub(super) fn created(&self) -> &[String] {
        &self.created
    }
}

/// Resolves every `--tag` and `--untag` value, creating the paths `--tag`
/// names that do not exist yet.
///
/// A tag is an ID when it parses as one, and a path otherwise.
///
/// # Errors
///
/// Returns [`CliError::Arg`] for an ID naming no tag or a malformed path,
/// and [`CliError::Core`] from the tag service.
pub(super) async fn tags(ctx: &AppContext, tagged: &[&str], untagged: &[&str]) -> CliResult<Tags> {
    let known: std::collections::HashSet<String> = ctx
        .tags
        .list()
        .await?
        .iter()
        .map(|t| t.id().to_string())
        .collect();
    let mut out = Tags::default();
    let mut to_create: Vec<(String, bc_models::TagPath)> = Vec::new();
    for (text, create) in tagged
        .iter()
        .map(|t| (*t, true))
        .chain(untagged.iter().map(|t| (*t, false)))
    {
        if out.ids.contains_key(text) {
            continue;
        }
        if let Ok(id) = bc_models::TagId::from_str(text) {
            if !known.contains(&id.to_string()) {
                return Err(CliError::Arg(format!("no tag has the ID '{text}'")));
            }
            out.ids.insert(text.to_owned(), id);
            continue;
        }
        let path = bc_models::TagPath::from_str(text)
            .map_err(|e| CliError::Arg(format!("invalid tag path '{text}': {e}")))?;
        if create {
            to_create.push((text.to_owned(), path));
        } else if let Some(id) = ctx.tags.find_by_path(&path).await? {
            out.ids.insert(text.to_owned(), id);
        }
    }
    let paths: Vec<bc_models::TagPath> = to_create.iter().map(|(_, p)| p.clone()).collect();
    let created = ctx.tags.create_paths(&paths).await?;
    for (text, path) in to_create {
        if let Some(id) = created.ids.get(&path.to_string()) {
            out.ids.insert(text, id.clone());
        }
    }
    out.created = created.created;
    Ok(out)
}

/// Looks up everything `changes` names.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when `--account` names no account, and
/// [`CliError::Core`] from the key registry.
pub(super) async fn resolved(
    ctx: &AppContext,
    changes: Changes,
    lookup: &spec::Lookup<'_>,
    tags: &Tags,
) -> CliResult<Resolved> {
    let account = match &changes.account {
        Some(text) => {
            Some(lookup(text).ok_or_else(|| CliError::Arg(format!("no account '{text}'")))?)
        }
        None => None,
    };
    let mut entries = Vec::with_capacity(changes.meta.len());
    for (key, raw) in &changes.meta {
        entries.push(meta::entry_for(ctx, key.clone(), raw).await?);
    }
    let pick = |texts: &[String]| texts.iter().filter_map(|t| tags.get(t).cloned()).collect();
    Ok(Resolved {
        tags: pick(&changes.tags),
        untags: pick(&changes.untags),
        account,
        entries,
        changes,
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::str::FromStr as _;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    #[rstest]
    #[case::word("person")]
    #[case::path("person:a")]
    #[case::bare_prefix("tag")]
    #[case::prefixed_word("tag_person")]
    fn a_tag_path_never_reads_as_an_id(#[case] text: &str) {
        assert!(
            bc_models::TagId::from_str(text).is_err(),
            "'{text}' parsed as an ID"
        );
    }

    #[test]
    fn a_tag_id_reads_as_one() {
        let id = bc_models::TagId::new();
        let parsed = bc_models::TagId::from_str(&id.to_string()).expect("parses");
        assert_eq!(parsed, id);
    }
}
