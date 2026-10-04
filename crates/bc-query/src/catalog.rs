//! What resolution needs to know about the ledger.

use crate::currency::MarkerSource;

/// The ledger facts that `resolve` types terms against.
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
