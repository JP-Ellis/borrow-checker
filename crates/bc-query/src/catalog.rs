//! What resolution needs to know about the ledger.

use std::collections::HashMap;

use crate::currency::Commodity;
use crate::currency::MarkerSource;
use crate::path::ends_with;

/// The ledger facts that [`resolve()`](crate::resolve()) types terms against.
///
/// The palette implements it from the lists it has loaded; `bc-core` from the
/// database.
pub trait Catalog {
    /// The commodity type the catalog holds.
    type Commodity: MarkerSource;

    /// Every live account.
    fn accounts(&self) -> &[PathEntry];

    /// Every tag.
    fn tags(&self) -> &[PathEntry];

    /// Every commodity, for currency markers.
    fn commodities(&self) -> &[Self::Commodity];

    /// Every registered metadata key.
    fn meta_keys(&self) -> &[MetaKey];

    /// Whether the account `id` is archived. An archived account still
    /// resolves; the palette only stops offering it.
    ///
    /// # Arguments
    ///
    /// * `id` - The account id.
    fn is_archived(&self, _id: &str) -> bool {
        false
    }
}

/// An account or tag: its id and path segments.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct PathEntry {
    /// The id the engine filters by.
    pub id: String,
    /// The path from the root, one segment per level.
    pub path: Vec<String>,
}

impl PathEntry {
    /// Creates an entry.
    ///
    /// # Arguments
    ///
    /// * `id` - The account or tag id.
    /// * `path` - The path segments from the root.
    #[must_use]
    pub fn new<S>(id: impl Into<String>, path: impl IntoIterator<Item = S>) -> Self
    where
        S: Into<String>,
    {
        Self {
            id: id.into(),
            path: path.into_iter().map(Into::into).collect(),
        }
    }

    /// The colon-joined path.
    #[must_use]
    pub fn display(&self) -> String {
        self.path.join(":")
    }
}

/// A metadata key's registered type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches MetaType exhaustively"
)]
pub enum MetaType {
    /// Free text.
    Text,
    /// A decimal number.
    Number,
    /// `true` or `false`.
    Boolean,
    /// A calendar date.
    Date,
    /// An RFC 3339 instant.
    Timestamp,
    /// A decimal with a commodity.
    Amount,
    /// An account path.
    Account,
}

impl MetaType {
    /// The type's name as the palette shows it: "number".
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "number",
            Self::Boolean => "boolean",
            Self::Date => "date",
            Self::Timestamp => "timestamp",
            Self::Amount => "amount",
            Self::Account => "account",
        }
    }

    /// The singular noun with its article: "a number".
    #[must_use]
    pub const fn singular(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "a number",
            Self::Boolean => "a boolean",
            Self::Date => "a date",
            Self::Timestamp => "a timestamp",
            Self::Amount => "an amount",
            Self::Account => "an account",
        }
    }

    /// The plural noun: "numbers".
    #[must_use]
    pub const fn plural(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Number => "numbers",
            Self::Boolean => "booleans",
            Self::Date => "dates",
            Self::Timestamp => "timestamps",
            Self::Amount => "amounts",
            Self::Account => "accounts",
        }
    }
}

/// A registered metadata key.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MetaKey {
    /// The normalised (lowercase) key.
    pub key: String,
    /// Its registered type.
    pub ty: MetaType,
    /// How many stored values disagree with `ty`.
    pub mismatched: usize,
}

impl MetaKey {
    /// Creates a key definition.
    ///
    /// # Arguments
    ///
    /// * `key` - The normalised key.
    /// * `ty` - Its registered type.
    /// * `mismatched` - How many stored values disagree with `ty`.
    #[must_use]
    pub fn new(key: impl Into<String>, ty: MetaType, mismatched: usize) -> Self {
        Self {
            key: key.into(),
            ty,
            mismatched,
        }
    }
}

/// Each entry's shortest trailing run of segments that names it alone, joined
/// by `:`, keyed by id.
///
/// Segments compare ignoring ASCII case, as resolution does. An entry whose
/// every ending is shared keeps its full path, which resolution prefers over
/// an ambiguous ending.
///
/// # Arguments
///
/// * `entries` - Every account, or every tag.
#[must_use]
pub fn shortest_endings(entries: &[PathEntry]) -> HashMap<String, String> {
    entries
        .iter()
        .map(|entry| {
            let len = entry.path.len();
            let unique = (1..=len)
                .find(|&n| {
                    let mine = tail(&entry.path, n);
                    !entries
                        .iter()
                        .any(|other| other.id != entry.id && ends_with(&other.path, mine))
                })
                .unwrap_or(len);
            (entry.id.clone(), tail(&entry.path, unique).join(":"))
        })
        .collect()
}

/// The last `n` segments of `path`, or all of them when it is shorter.
fn tail(path: &[String], n: usize) -> &[String] {
    path.get(path.len().saturating_sub(n)..).unwrap_or_default()
}

/// A [`Catalog`] held in memory: the palette's copy of the server's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Snapshot {
    /// Every account, archived ones included.
    pub accounts: Vec<PathEntry>,
    /// Every tag.
    pub tags: Vec<PathEntry>,
    /// Every commodity, for currency markers.
    pub commodities: Vec<Commodity>,
    /// Every registered metadata key.
    pub meta_keys: Vec<MetaKey>,
    /// The ids of archived accounts: they resolve, but the palette does not
    /// offer them.
    pub archived: Vec<String>,
}

impl Snapshot {
    /// Creates a snapshot.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Every account, archived ones included.
    /// * `tags` - Every tag.
    /// * `commodities` - Every commodity.
    /// * `meta_keys` - Every registered metadata key.
    #[must_use]
    pub const fn new(
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
            archived: Vec::new(),
        }
    }

    /// The snapshot with `ids` marked as archived accounts.
    ///
    /// # Arguments
    ///
    /// * `ids` - The ids of archived accounts.
    #[must_use]
    pub fn with_archived(self, ids: Vec<String>) -> Self {
        Self {
            archived: ids,
            ..self
        }
    }
}

impl Catalog for Snapshot {
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

    fn is_archived(&self, id: &str) -> bool {
        self.archived.iter().any(|archived| archived == id)
    }
}

#[cfg(feature = "ipc")]
impl From<bc_ipc::QueryCatalog> for Snapshot {
    /// Converts the server's catalog into the palette's copy.
    fn from(catalog: bc_ipc::QueryCatalog) -> Self {
        let path = |entry: bc_ipc::CatalogPath| PathEntry::new(entry.id, entry.path);
        Self::new(
            catalog.accounts.into_iter().map(path).collect(),
            catalog.tags.into_iter().map(path).collect(),
            catalog
                .commodities
                .into_iter()
                .map(|c| Commodity {
                    code: c.code,
                    symbol: c.symbol,
                    aliases: c.aliases,
                })
                .collect(),
            catalog
                .meta_keys
                .into_iter()
                .map(|k| {
                    MetaKey::new(
                        k.key,
                        meta_type(k.ty),
                        usize::try_from(k.mismatched).unwrap_or(usize::MAX),
                    )
                })
                .collect(),
        )
        .with_archived(catalog.archived)
    }
}

/// The query type for an IPC key type.
#[cfg(feature = "ipc")]
const fn meta_type(ty: bc_ipc::MetaTypeDto) -> MetaType {
    match ty {
        bc_ipc::MetaTypeDto::Text => MetaType::Text,
        bc_ipc::MetaTypeDto::Number => MetaType::Number,
        bc_ipc::MetaTypeDto::Boolean => MetaType::Boolean,
        bc_ipc::MetaTypeDto::Date => MetaType::Date,
        bc_ipc::MetaTypeDto::Timestamp => MetaType::Timestamp,
        bc_ipc::MetaTypeDto::Amount => MetaType::Amount,
        bc_ipc::MetaTypeDto::Account => MetaType::Account,
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::currency::Commodity;
    use crate::parser::parse;
    use crate::resolve::resolve;

    /// `(id, path)` pairs as entries.
    fn entries(rows: &[(&str, &[&str])]) -> Vec<PathEntry> {
        rows.iter()
            .map(|(id, path)| PathEntry::new(*id, path.iter().copied()))
            .collect()
    }

    /// A map from id to ending.
    fn endings(rows: &[(&str, &str)]) -> HashMap<String, String> {
        rows.iter()
            .map(|(id, ending)| ((*id).to_owned(), (*ending).to_owned()))
            .collect()
    }

    #[rstest]
    #[case::ambiguous_leaves_take_a_parent(
        &[
            ("a1", &["Expenses", "Food"][..]),
            ("a2", &["Expenses", "Food", "Groceries"][..]),
            ("a3", &["Income", "Food"][..]),
            ("a5", &["Liabilities", "Credit Card"][..]),
        ],
        &[("a1", "Expenses:Food"), ("a2", "Groceries"), ("a3", "Income:Food"), ("a5", "Credit Card")],
    )]
    #[case::a_root_keeps_its_full_path(
        &[("r1", &["Food"][..]), ("r2", &["Expenses", "Food"][..])],
        &[("r1", "Food"), ("r2", "Expenses:Food")],
    )]
    #[case::segments_compare_ignoring_case(
        &[("c1", &["Assets", "bank"][..]), ("c2", &["Liabilities", "Bank"][..])],
        &[("c1", "Assets:bank"), ("c2", "Liabilities:Bank")],
    )]
    fn shortest_endings_are_unique(
        #[case] rows: &[(&str, &[&str])],
        #[case] expected: &[(&str, &str)],
    ) {
        assert_eq!(shortest_endings(&entries(rows)), endings(expected));
    }

    #[test]
    fn every_type_has_a_name() {
        let names: Vec<&str> = [
            MetaType::Text,
            MetaType::Number,
            MetaType::Boolean,
            MetaType::Date,
            MetaType::Timestamp,
            MetaType::Amount,
            MetaType::Account,
        ]
        .into_iter()
        .map(MetaType::name)
        .collect();
        assert_eq!(
            names,
            vec![
                "text",
                "number",
                "boolean",
                "date",
                "timestamp",
                "amount",
                "account"
            ]
        );
    }

    #[test]
    fn a_snapshot_resolves_like_any_catalog() {
        let snapshot = Snapshot::new(
            entries(&[("a2", &["Expenses", "Food", "Groceries"][..])]),
            Vec::new(),
            vec![Commodity::new("AUD", Some("A$"), &[])],
            vec![MetaKey::new("km", MetaType::Number, 2)],
        );
        let resolved = resolve(
            &parse("account:Groceries amount:A$5 @km:>1").expect("parses"),
            &snapshot,
        );
        assert!(!resolved.has_errors(), "{:?}", resolved.diagnostics);
    }

    #[cfg(feature = "ipc")]
    #[test]
    fn an_ipc_catalog_becomes_a_snapshot() {
        let mut dto = bc_ipc::QueryCatalog::new(
            vec![bc_ipc::CatalogPath::new(
                "a1",
                vec!["Assets".to_owned(), "Bank".to_owned()],
            )],
            vec![bc_ipc::CatalogPath::new("t1", vec!["trip".to_owned()])],
            vec![bc_ipc::CatalogCommodity::new(
                "AUD",
                Some("A$".to_owned()),
                vec!["AU$".to_owned()],
            )],
            vec![bc_ipc::CatalogKey::new(
                "km",
                bc_ipc::MetaTypeDto::Number,
                2,
            )],
        );
        dto.archived = vec!["a1".to_owned()];
        assert_eq!(
            Snapshot::from(dto),
            Snapshot::new(
                entries(&[("a1", &["Assets", "Bank"][..])]),
                entries(&[("t1", &["trip"][..])]),
                vec![Commodity::new("AUD", Some("A$"), &["AU$"])],
                vec![MetaKey::new("km", MetaType::Number, 2)],
            )
            .with_archived(vec!["a1".to_owned()])
        );
    }
}
