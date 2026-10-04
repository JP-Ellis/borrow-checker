//! The ledger facts a query resolves against, read from the database: every
//! account and tag path, commodity and metadata key.

use std::collections::HashMap;

use bc_query::Catalog;
use bc_query::catalog::MetaKey;
use bc_query::catalog::MetaType as QueryType;
use bc_query::catalog::PathEntry;
use bc_query::currency::Commodity;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;

/// How many parent links a path may follow before it counts as a cycle.
const MAX_DEPTH: u16 = 64;

/// One `accounts` or `tags` row: id, name and parent id.
type TreeRow = (String, String, Option<String>);

/// Every ledger fact query resolution and subtree expansion need, loaded once
/// per query.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct DbCatalog {
    /// Every account, archived ones included, with its path.
    accounts: Vec<PathEntry>,
    /// Every tag with its path.
    tags: Vec<PathEntry>,
    /// Every commodity, for currency markers.
    commodities: Vec<Commodity>,
    /// Every registered metadata key with its mismatch count.
    meta_keys: Vec<MetaKey>,
}

impl DbCatalog {
    /// Loads the catalog.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] when a parent chain is broken or cyclic,
    /// [`BcError::Serialisation`] when a stored key type is unreadable, and
    /// [`BcError`] on database failure.
    pub async fn load(pool: &SqlitePool) -> BcResult<Self> {
        let accounts: Vec<TreeRow> = sqlx::query_as("SELECT id, name, parent_id FROM accounts")
            .fetch_all(pool)
            .await?;
        let tags: Vec<TreeRow> = sqlx::query_as("SELECT id, name, parent_id FROM tags")
            .fetch_all(pool)
            .await?;
        let commodities = crate::commodity::Service::new(pool.clone())
            .list_all()
            .await?
            .iter()
            .map(|c| {
                let aliases: Vec<&str> = c.aliases().iter().map(String::as_str).collect();
                Commodity::new(c.code(), c.symbol(), &aliases)
            })
            .collect();
        let keys: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT k.key, k.value_type, \
                    (SELECT COUNT(*) FROM transaction_metadata m \
                      WHERE m.key = k.key AND m.mismatched = 1) \
                  + (SELECT COUNT(*) FROM posting_metadata m \
                      WHERE m.key = k.key AND m.mismatched = 1) \
             FROM metadata_keys k ORDER BY k.key",
        )
        .fetch_all(pool)
        .await?;
        let meta_keys = keys
            .into_iter()
            .map(|(key, stored_type, mismatched)| {
                let ty = query_type(from_db_str::<bc_models::MetaType>(&stored_type)?);
                Ok(MetaKey::new(
                    key,
                    ty,
                    usize::try_from(mismatched).unwrap_or(0),
                ))
            })
            .collect::<BcResult<Vec<_>>>()?;
        Ok(Self {
            accounts: paths("account", &accounts)?,
            tags: paths("tag", &tags)?,
            commodities,
            meta_keys,
        })
    }

    /// Builds a catalog from parts, for tests that need no database.
    #[cfg(test)]
    pub(crate) const fn from_parts(
        accounts: Vec<PathEntry>,
        tags: Vec<PathEntry>,
        commodities: Vec<Commodity>,
        meta_keys: Vec<MetaKey>,
    ) -> Self {
        Self {
            accounts,
            tags,
            commodities,
            meta_keys,
        }
    }

    /// Ids of the account `id` and, when `subtree`, every account beneath it;
    /// empty when no account carries `id`.
    pub(crate) fn account_ids(&self, id: &str, subtree: bool) -> Vec<&str> {
        expand(&self.accounts, id, subtree)
    }

    /// Ids of the tag `id` and, when `subtree`, every tag beneath it; empty
    /// when no tag carries `id`.
    pub(crate) fn tag_ids(&self, id: &str, subtree: bool) -> Vec<&str> {
        expand(&self.tags, id, subtree)
    }

    /// Ids of the accounts whose path is `path`, or lies beneath it when
    /// `subtree`, compared exactly: a resolved path carries the catalog's casing.
    pub(crate) fn account_ids_by_path(&self, path: &[String], subtree: bool) -> Vec<&str> {
        self.accounts
            .iter()
            .filter(|entry| under(&entry.path, path, subtree, |a, b| a == b))
            .map(|entry| entry.id.as_str())
            .collect()
    }
}

impl Catalog for DbCatalog {
    type Commodity = Commodity;

    fn accounts(&self) -> &[PathEntry] {
        &self.accounts
    }

    fn tags(&self) -> &[PathEntry] {
        &self.tags
    }

    fn commodities(&self) -> &[Commodity] {
        &self.commodities
    }

    fn meta_keys(&self) -> &[MetaKey] {
        &self.meta_keys
    }
}

/// Whether `candidate` is `path`, or lies beneath it when `subtree`, comparing
/// segments with `same`.
///
/// # Arguments
///
/// * `candidate` - The path tested.
/// * `path` - The path it must equal or extend.
/// * `subtree` - Whether a longer `candidate` may match.
/// * `same` - Segment equality.
pub(crate) fn under(
    candidate: &[String],
    path: &[String],
    subtree: bool,
    same: impl Fn(&str, &str) -> bool,
) -> bool {
    let long_enough = candidate.len() == path.len() || (subtree && candidate.len() > path.len());
    long_enough && candidate.iter().zip(path).all(|(c, p)| same(c, p))
}

/// The ids of the entry `id` and, when `subtree`, every entry beneath it.
fn expand<'c>(entries: &'c [PathEntry], id: &str, subtree: bool) -> Vec<&'c str> {
    let Some(root) = entries.iter().find(|entry| entry.id == id) else {
        return Vec::new();
    };
    if !subtree {
        return vec![root.id.as_str()];
    }
    entries
        .iter()
        .filter(|entry| under(&entry.path, &root.path, true, |a, b| a == b))
        .map(|entry| entry.id.as_str())
        .collect()
}

/// Builds each row's path by walking its parents.
///
/// # Errors
///
/// Returns [`BcError::BadData`] when a parent id names no row, or a chain is
/// longer than [`MAX_DEPTH`] (a cycle).
fn paths(kind: &str, rows: &[TreeRow]) -> BcResult<Vec<PathEntry>> {
    let by_id: HashMap<&str, (&str, Option<&str>)> = rows
        .iter()
        .map(|(id, name, parent)| (id.as_str(), (name.as_str(), parent.as_deref())))
        .collect();
    rows.iter()
        .map(|(id, _, _)| {
            let mut segments: Vec<String> = Vec::new();
            let mut current = Some(id.as_str());
            for _step in 0..MAX_DEPTH {
                let Some(cursor) = current else {
                    break;
                };
                let Some(&(name, parent)) = by_id.get(cursor) else {
                    return Err(BcError::BadData(format!(
                        "{kind} {id} has a parent {cursor} that does not exist"
                    )));
                };
                segments.push(name.to_owned());
                current = parent;
            }
            if current.is_some() {
                return Err(BcError::BadData(format!(
                    "{kind} {id} sits in a parent cycle or a tree deeper than {MAX_DEPTH} levels"
                )));
            }
            segments.reverse();
            Ok(PathEntry::new(id.clone(), segments))
        })
        .collect()
}

/// The query language's name for a registered metadata type.
const fn query_type(ty: bc_models::MetaType) -> QueryType {
    match ty {
        bc_models::MetaType::Text => QueryType::Text,
        bc_models::MetaType::Number => QueryType::Number,
        bc_models::MetaType::Boolean => QueryType::Boolean,
        bc_models::MetaType::Date => QueryType::Date,
        bc_models::MetaType::Timestamp => QueryType::Timestamp,
        bc_models::MetaType::Amount => QueryType::Amount,
        bc_models::MetaType::Account => QueryType::Account,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::MetaEntry;
    use bc_models::MetaKey as ModelKey;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::TagPath;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    /// One single-leg transaction carrying `km` = `value`.
    fn with_km(account: &bc_models::AccountId, value: MetaValue) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 3, 1))
            .description("Test")
            .metadata(Metadata::new(vec![MetaEntry::new(
                ModelKey::new("km").expect("key"),
                value,
            )]))
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(account.clone())
                    .amount(Amount::new(dec!(1), "AUD"))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loads_paths_keys_and_mismatch_counts(pool: SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let assets = accounts
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Assets");
        let bank = accounts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("Bank");
        let tags = crate::tag::Service::new(pool.clone());
        let trip = tags
            .create_path(&"trip".parse::<TagPath>().expect("path"))
            .await
            .expect("trip");
        let flights = tags
            .create_path(&"trip:flights".parse::<TagPath>().expect("path"))
            .await
            .expect("flights");
        let txns = crate::transaction::Service::new(pool.clone());
        txns.create(with_km(&bank, MetaValue::Number(dec!(1200))))
            .await
            .expect("number");
        txns.create(with_km(&bank, MetaValue::Text("lots".to_owned())))
            .await
            .expect("mismatched");

        let catalog = DbCatalog::load(&pool).await.expect("catalog");

        let mut paths: Vec<String> = catalog.accounts().iter().map(PathEntry::display).collect();
        paths.sort();
        assert_eq!(paths, vec!["Assets".to_owned(), "Assets:Bank".to_owned()]);
        let mut subtree = catalog.account_ids(&assets.to_string(), true);
        subtree.sort_unstable();
        let mut want = [assets.to_string(), bank.to_string()];
        want.sort();
        assert_eq!(subtree, want.iter().map(String::as_str).collect::<Vec<_>>());
        assert_eq!(
            catalog.account_ids(&assets.to_string(), false),
            vec![assets.to_string().as_str()]
        );
        assert_eq!(catalog.account_ids("unknown", true), Vec::<&str>::new());
        assert_eq!(catalog.tag_ids(&trip.to_string(), true).len(), 2);
        assert_eq!(
            catalog.tag_ids(&flights.to_string(), true),
            vec![flights.to_string().as_str()]
        );
        assert_eq!(
            catalog
                .account_ids_by_path(&["Assets".to_owned()], true)
                .len(),
            2
        );
        assert_eq!(
            catalog.account_ids_by_path(&["Assets".to_owned(), "Bank".to_owned()], false),
            vec![bank.to_string().as_str()]
        );
        assert_eq!(
            catalog.meta_keys(),
            [MetaKey::new("km", QueryType::Number, 1)].as_slice()
        );
        assert_eq!(
            catalog.commodities().len(),
            crate::commodity::Service::new(pool.clone())
                .list_all()
                .await
                .expect("list")
                .len()
        );
    }

    #[test]
    fn under_compares_segment_by_segment() {
        let path = vec!["Assets".to_owned(), "Bank".to_owned()];
        let child = vec!["Assets".to_owned(), "Bank".to_owned(), "Sub".to_owned()];
        let lookalike = vec!["Assets".to_owned(), "Banking".to_owned()];
        let exact = |a: &str, b: &str| a == b;
        assert!(under(&path, &path, false, exact));
        assert!(!under(&child, &path, false, exact));
        assert!(under(&child, &path, true, exact));
        assert!(!under(&lookalike, &path, true, exact));
        assert!(!under(
            &["assets".to_owned(), "bank".to_owned()],
            &path,
            false,
            exact
        ));
        assert!(under(
            &["assets".to_owned(), "bank".to_owned()],
            &path,
            false,
            str::eq_ignore_ascii_case
        ));
    }
}
