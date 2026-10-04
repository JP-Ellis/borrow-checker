//! The ledger facts that query text resolves against, sent whole to the palette.

use serde::Deserialize;
use serde::Serialize;

use crate::MetaTypeDto;

/// Every fact query text resolves against: the server's query catalog.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct QueryCatalog {
    /// Every account, archived ones included.
    pub accounts: Vec<CatalogPath>,
    /// Every tag.
    pub tags: Vec<CatalogPath>,
    /// Every commodity, for currency markers.
    pub commodities: Vec<CatalogCommodity>,
    /// Every registered metadata key.
    pub meta_keys: Vec<CatalogKey>,
    /// The ids of the archived accounts in `accounts`: they resolve, but the
    /// palette does not offer them.
    pub archived: Vec<String>,
}

impl QueryCatalog {
    /// Creates a catalog.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Every account, archived ones included.
    /// * `tags` - Every tag.
    /// * `commodities` - Every commodity.
    /// * `meta_keys` - Every registered metadata key.
    /// * `archived` - The ids of the archived accounts in `accounts`.
    #[must_use]
    #[inline]
    pub const fn new(
        accounts: Vec<CatalogPath>,
        tags: Vec<CatalogPath>,
        commodities: Vec<CatalogCommodity>,
        meta_keys: Vec<CatalogKey>,
        archived: Vec<String>,
    ) -> Self {
        Self {
            accounts,
            tags,
            commodities,
            meta_keys,
            archived,
        }
    }
}

/// An account or tag: its id and path segments from the root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CatalogPath {
    /// The account or tag id.
    pub id: String,
    /// The path from the root, one segment per level.
    pub path: Vec<String>,
}

impl CatalogPath {
    /// Creates a path entry.
    ///
    /// # Arguments
    ///
    /// * `id` - The account or tag id.
    /// * `path` - The path segments from the root.
    #[must_use]
    #[inline]
    pub fn new(id: impl Into<String>, path: Vec<String>) -> Self {
        Self {
            id: id.into(),
            path,
        }
    }
}

/// A commodity's currency markers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CatalogCommodity {
    /// The canonical code.
    pub code: String,
    /// The display symbol.
    pub symbol: Option<String>,
    /// Further exact-match markers.
    pub aliases: Vec<String>,
}

impl CatalogCommodity {
    /// Creates a commodity's markers.
    ///
    /// # Arguments
    ///
    /// * `code` - The canonical code.
    /// * `symbol` - The display symbol.
    /// * `aliases` - Further exact-match markers.
    #[must_use]
    #[inline]
    pub fn new(code: impl Into<String>, symbol: Option<String>, aliases: Vec<String>) -> Self {
        Self {
            code: code.into(),
            symbol,
            aliases,
        }
    }
}

/// A registered metadata key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CatalogKey {
    /// The normalised (lowercase) key.
    pub key: String,
    /// Its registered type.
    pub ty: MetaTypeDto,
    /// How many stored values disagree with `ty`.
    pub mismatched: u64,
}

impl CatalogKey {
    /// Creates a key entry.
    ///
    /// # Arguments
    ///
    /// * `key` - The normalised key.
    /// * `ty` - Its registered type.
    /// * `mismatched` - How many stored values disagree with `ty`.
    #[must_use]
    #[inline]
    pub fn new(key: impl Into<String>, ty: MetaTypeDto, mismatched: u64) -> Self {
        Self {
            key: key.into(),
            ty,
            mismatched,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn a_catalog_survives_json() {
        let catalog = QueryCatalog::new(
            vec![CatalogPath::new(
                "a1",
                vec!["Assets".to_owned(), "Bank".to_owned()],
            )],
            vec![CatalogPath::new("t1", vec!["trip".to_owned()])],
            vec![CatalogCommodity::new(
                "AUD",
                Some("A$".to_owned()),
                vec!["AU$".to_owned()],
            )],
            vec![CatalogKey::new("km", MetaTypeDto::Number, 2)],
            vec!["a1".to_owned()],
        );
        let json = serde_json::to_string(&catalog).expect("serialises");
        assert_eq!(
            serde_json::from_str::<QueryCatalog>(&json).expect("deserialises"),
            catalog
        );
    }
}
