//! Colon-separated account paths and their resolution to account IDs.

use std::collections::HashMap;

use bc_models::AccountId;

use crate::BcError;
use crate::BcResult;

/// A validated, colon-separated account path such as `Assets:Bank:Checking`.
///
/// Segments are trimmed of surrounding whitespace but otherwise preserved
/// verbatim: matching against stored account names is exact and
/// case-sensitive, because Beancount capitalises its roots and is itself
/// case-sensitive, so normalising would invent ambiguity rather than remove it.
/// Spaces *inside* a segment are kept, since Ledger allows them in account
/// names.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPath {
    /// Path segments, root first. Never empty; no segment is empty.
    segments: Vec<String>,
}

impl AccountPath {
    /// Parses a colon-separated account path.
    ///
    /// # Arguments
    ///
    /// * `raw` - The path as written by an importer, e.g. `"Assets:Bank:Checking"`.
    ///
    /// # Returns
    ///
    /// The parsed [`AccountPath`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if `raw` is empty, or if any segment is
    /// empty or whitespace-only — both mean the importer emitted a path no
    /// account could ever match.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::AccountPath;
    ///
    /// let path = AccountPath::parse("Assets:Bank:Checking")?;
    /// assert_eq!(path.to_string(), "Assets:Bank:Checking");
    /// assert!(AccountPath::parse("Assets::Bank").is_err());
    /// # Ok::<(), bc_core::BcError>(())
    /// ```
    #[inline]
    pub fn parse(raw: &str) -> BcResult<Self> {
        let segments: Vec<String> = raw.split(':').map(|s| s.trim().to_owned()).collect();
        if segments.iter().any(String::is_empty) {
            return Err(BcError::BadData(format!(
                "malformed account path '{raw}': segments must be non-empty"
            )));
        }
        Ok(Self { segments })
    }

    /// Returns the path segments, root first.
    #[inline]
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

impl core::fmt::Display for AccountPath {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.segments.join(":"))
    }
}

/// The outcome of resolving one [`AccountPath`] against the account tree.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// The path names an existing account.
    Resolved {
        /// The resolved account.
        id: AccountId,
        /// Whether that account is archived. Import proceeds, with a warning.
        archived: bool,
    },
    /// The path names no existing account.
    Missing {
        /// The deepest colon-joined prefix that did resolve; empty if the root
        /// segment itself is absent.
        resolved_prefix: String,
        /// The first segment that could not be resolved.
        missing_segment: String,
    },
}

/// Renders one account's path by walking `named` from the account to its root.
///
/// # Arguments
///
/// * `id` - The account to render, as its id string.
/// * `named` - Every account's `(name, parent id)`, keyed by id string.
///
/// # Returns
///
/// The colon-separated path, root first. An account whose parent is absent from
/// `named` renders from as far as the walk got, which is the same partial answer
/// [`AccountResolver::resolve`] gives for a path it cannot complete.
fn render_path(id: &str, named: &HashMap<String, (&str, Option<String>)>) -> String {
    let mut segments: Vec<&str> = Vec::new();
    let mut current: Option<&str> = Some(id);
    // Bounded by the account count: a parent cycle would otherwise spin here,
    // and this renders a path rather than validating the tree.
    for _ in 0..named.len() {
        let Some(key) = current else { break };
        let Some(&(name, ref parent)) = named.get(key) else {
            break;
        };
        segments.push(name);
        current = parent.as_deref();
    }
    segments.reverse();
    segments.join(":")
}

/// Resolves colon-separated account paths to [`AccountId`]s.
///
/// Built once per import from a single snapshot of the accounts table, so
/// resolving thousands of legs costs no further queries. Archived accounts are
/// included: they exist, so a path naming one resolves rather than reporting a
/// missing account.
///
/// Resolution relies on sibling names being unique
/// (`idx_accounts_sibling_unique`), which is what makes path → id a function.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct AccountResolver {
    /// `(parent id string or "" for a root, name)` → account, plus its archived flag.
    by_parent_and_name: HashMap<(String, String), (AccountId, bool)>,
    /// Account id string → the rendered path that resolves to it.
    ///
    /// The inverse of [`Self::resolve`], for callers holding an id that came
    /// from the database rather than from a document, and needing the path a
    /// human reads.
    paths_by_id: HashMap<String, String>,
}

impl AccountResolver {
    /// Loads every account into an in-memory resolution map.
    ///
    /// # Arguments
    ///
    /// * `accounts` - The account service to snapshot.
    ///
    /// # Returns
    ///
    /// A resolver over every account, archived included.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn load(accounts: &crate::AccountService) -> BcResult<Self> {
        let all = accounts.list_all().await?;
        let by_parent_and_name = all
            .iter()
            .map(|account| {
                let parent_key = account
                    .parent_id()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                (
                    (parent_key, account.name().to_owned()),
                    (account.id().clone(), account.archived_at().is_some()),
                )
            })
            .collect();

        let named: HashMap<String, (&str, Option<String>)> = all
            .iter()
            .map(|account| {
                (
                    account.id().to_string(),
                    (account.name(), account.parent_id().map(ToString::to_string)),
                )
            })
            .collect();
        let paths_by_id = named
            .keys()
            .map(|id| (id.clone(), render_path(id, &named)))
            .collect();

        Ok(Self {
            by_parent_and_name,
            paths_by_id,
        })
    }

    /// Returns the rendered path of the account `id` names.
    ///
    /// The inverse of [`Self::resolve`], over the same snapshot: a path this
    /// returns resolves back to `id`.
    ///
    /// # Arguments
    ///
    /// * `id` - The account to render.
    ///
    /// # Returns
    ///
    /// The colon-separated path, or `None` if the snapshot does not hold the
    /// account — it was created or deleted after the resolver was loaded.
    #[inline]
    #[must_use]
    pub fn path_of(&self, id: &AccountId) -> Option<&str> {
        self.paths_by_id.get(&id.to_string()).map(String::as_str)
    }

    /// Resolves a path by walking its segments down the account tree.
    ///
    /// # Arguments
    ///
    /// * `path` - The parsed path to resolve.
    ///
    /// # Returns
    ///
    /// [`Resolution::Resolved`] naming the account, or [`Resolution::Missing`]
    /// describing how far the walk got.
    #[inline]
    #[must_use]
    pub fn resolve(&self, path: &AccountPath) -> Resolution {
        let mut parent_key = String::new();
        let mut walked: Vec<&str> = Vec::new();
        let mut found: Option<(AccountId, bool)> = None;

        for segment in path.segments() {
            let key = (parent_key.clone(), segment.clone());
            match self.by_parent_and_name.get(&key) {
                Some((id, archived)) => {
                    parent_key = id.to_string();
                    walked.push(segment.as_str());
                    found = Some((id.clone(), *archived));
                }
                None => {
                    return Resolution::Missing {
                        resolved_prefix: walked.join(":"),
                        missing_segment: segment.clone(),
                    };
                }
            }
        }

        match found {
            Some((id, archived)) => Resolution::Resolved { id, archived },
            // Unreachable: AccountPath::parse guarantees at least one segment,
            // so the loop either returns Missing or sets `found`.
            None => Resolution::Missing {
                resolved_prefix: String::new(),
                missing_segment: path.to_string(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use sqlx::SqlitePool;

    use super::*;

    #[test]
    fn parses_a_multi_segment_path() {
        let path = AccountPath::parse("Assets:Bank:Checking").expect("valid path");
        assert_eq!(
            path.segments(),
            [
                "Assets".to_owned(),
                "Bank".to_owned(),
                "Checking".to_owned()
            ]
        );
    }

    #[test]
    fn parses_a_single_segment_path() {
        let path = AccountPath::parse("Assets").expect("valid path");
        assert_eq!(path.segments(), ["Assets".to_owned()]);
    }

    #[test]
    fn trims_whitespace_around_segments() {
        let path = AccountPath::parse(" Assets : Bank ").expect("valid path");
        assert_eq!(path.segments(), ["Assets".to_owned(), "Bank".to_owned()]);
    }

    #[test]
    fn preserves_internal_spaces_in_a_segment() {
        // Ledger allows spaces inside an account name.
        let path = AccountPath::parse("Assets:Joint Savings").expect("valid path");
        assert_eq!(
            path.segments(),
            ["Assets".to_owned(), "Joint Savings".to_owned()]
        );
    }

    #[test]
    fn matching_is_case_sensitive_so_case_survives_parsing() {
        let path = AccountPath::parse("assets:bank").expect("valid path");
        assert_eq!(path.segments(), ["assets".to_owned(), "bank".to_owned()]);
    }

    #[rstest]
    #[case::empty("")]
    #[case::only_whitespace("   ")]
    #[case::only_separator(":")]
    #[case::leading_separator(":Assets")]
    #[case::trailing_separator("Assets:")]
    #[case::empty_middle_segment("Assets::Bank")]
    #[case::whitespace_segment("Assets: :Bank")]
    fn rejects_malformed_paths(#[case] raw: &str) {
        assert!(
            AccountPath::parse(raw).is_err(),
            "'{raw}' must be rejected as a malformed account path"
        );
    }

    #[test]
    fn displays_as_a_colon_joined_string() {
        let path = AccountPath::parse(" Assets : Bank ").expect("valid path");
        assert_eq!(path.to_string(), "Assets:Bank");
    }

    /// Creates an account under an optional parent, returning its ID.
    async fn account(pool: &SqlitePool, name: &str, parent: Option<&AccountId>) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create()
            .name(name)
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .maybe_parent_id(parent)
            .call()
            .await
            .expect("create account")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolves_a_nested_path_to_its_leaf(pool: SqlitePool) {
        let assets = account(&pool, "Assets", None).await;
        let bank = account(&pool, "Bank", Some(&assets)).await;
        let checking = account(&pool, "Checking", Some(&bank)).await;

        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");
        let path = AccountPath::parse("Assets:Bank:Checking").expect("valid");

        assert_eq!(
            resolver.resolve(&path),
            Resolution::Resolved {
                id: checking,
                archived: false
            }
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn same_name_under_different_parents_resolves_distinctly(pool: SqlitePool) {
        // Expenses:Food and Income:Food are different accounts sharing a leaf name.
        let expenses = account(&pool, "Expenses", None).await;
        let income = account(&pool, "Income", None).await;
        let expense_food = account(&pool, "Food", Some(&expenses)).await;
        let income_food = account(&pool, "Food", Some(&income)).await;

        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");

        assert_eq!(
            resolver.resolve(&AccountPath::parse("Expenses:Food").expect("valid")),
            Resolution::Resolved {
                id: expense_food,
                archived: false
            }
        );
        assert_eq!(
            resolver.resolve(&AccountPath::parse("Income:Food").expect("valid")),
            Resolution::Resolved {
                id: income_food,
                archived: false
            }
        );
    }

    /// `path_of` is the inverse of `resolve`, so the round trip must land back
    /// on the account it started from — a renderer that dropped or reordered a
    /// segment would produce a path naming a different account, or none.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_rendered_path_resolves_back_to_its_account(pool: SqlitePool) {
        let assets = account(&pool, "Assets", None).await;
        let bank = account(&pool, "Bank", Some(&assets)).await;
        let checking = account(&pool, "Checking", Some(&bank)).await;

        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");

        let rendered = resolver.path_of(&checking).expect("the account is loaded");
        assert_eq!(rendered, "Assets:Bank:Checking");
        assert_eq!(
            resolver.resolve(&AccountPath::parse(rendered).expect("valid")),
            Resolution::Resolved {
                id: checking,
                archived: false
            }
        );
        assert_eq!(
            resolver.path_of(&assets),
            Some("Assets"),
            "a root is itself"
        );
    }

    /// An id the snapshot never held renders nothing rather than an empty path,
    /// which would otherwise read as a root account.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unknown_account_renders_no_path(pool: SqlitePool) {
        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");

        assert_eq!(resolver.path_of(&AccountId::new()), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reports_the_resolved_prefix_and_missing_segment(pool: SqlitePool) {
        let expenses = account(&pool, "Expenses", None).await;
        account(&pool, "Food", Some(&expenses)).await;

        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");
        let path = AccountPath::parse("Expenses:Food:Restaurants").expect("valid");

        assert_eq!(
            resolver.resolve(&path),
            Resolution::Missing {
                resolved_prefix: "Expenses:Food".to_owned(),
                missing_segment: "Restaurants".to_owned(),
            },
            "the diagnostic must name what did resolve and what did not"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_missing_root_reports_an_empty_prefix(pool: SqlitePool) {
        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");
        let path = AccountPath::parse("Liabilities:Card").expect("valid");

        assert_eq!(
            resolver.resolve(&path),
            Resolution::Missing {
                resolved_prefix: String::new(),
                missing_segment: "Liabilities".to_owned(),
            }
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deeper_path_than_the_tree_is_missing_not_resolved(pool: SqlitePool) {
        // 'Assets' exists as a leaf; 'Assets:Bank' must not silently resolve to it.
        account(&pool, "Assets", None).await;

        let svc = crate::AccountService::new(pool.clone());
        let resolver = AccountResolver::load(&svc).await.expect("load resolver");

        assert_eq!(
            resolver.resolve(&AccountPath::parse("Assets:Bank").expect("valid")),
            Resolution::Missing {
                resolved_prefix: "Assets".to_owned(),
                missing_segment: "Bank".to_owned(),
            }
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolves_an_archived_account_and_flags_it(pool: SqlitePool) {
        let assets = account(&pool, "Assets", None).await;
        let old = account(&pool, "OldBank", Some(&assets)).await;
        let svc = crate::AccountService::new(pool.clone());
        svc.archive(&old).await.expect("archive");

        let resolver = AccountResolver::load(&svc).await.expect("load resolver");
        assert_eq!(
            resolver.resolve(&AccountPath::parse("Assets:OldBank").expect("valid")),
            Resolution::Resolved {
                id: old,
                archived: true
            },
            "an archived account exists, so it resolves \u{2014} flagged, not missing"
        );
    }
}
