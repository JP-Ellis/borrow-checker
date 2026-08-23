//! Typed key-value metadata attachable to transactions and postings.

use core::fmt;
use core::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::AccountId;
use crate::money::Amount;
use crate::money::CommodityCode;

/// Maximum length of a metadata key, in bytes.
///
/// Keys are restricted to ASCII, so this is also a character count.
const MAX_KEY_BYTES: usize = 64;

/// Error returned when a string cannot be normalised into a [`MetaKey`].
///
/// Re-exported from the crate root as [`crate::MetaKeyError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum KeyError {
    /// The key was empty; a metadata key requires at least one character.
    #[error("metadata key must not be empty")]
    Empty,
    /// The first character was not an ASCII letter.
    #[error("metadata key must start with a letter, found '{found}'")]
    LeadingChar {
        /// The offending first character, after lowercasing.
        found: char,
    },
    /// A character outside `[a-z0-9_-]` appeared after the first.
    #[error("metadata key may only contain letters, digits, '_' and '-', found '{found}'")]
    InvalidChar {
        /// The offending character, after lowercasing.
        found: char,
    },
    /// The key exceeded [`MAX_KEY_BYTES`].
    #[error("metadata key must be at most 64 bytes, found {len}")]
    TooLong {
        /// Length of the offending key in bytes, after lowercasing.
        len: usize,
    },
}

/// A validated metadata key, normalised to lowercase.
///
/// Keys match `[a-z][a-z0-9_-]*` and are at most 64 bytes. The charset is
/// ASCII, so bytes and characters count the same and the distinction never
/// bites. Lowercasing is deliberate and silent: keys live in one global
/// registry, where `Payee` and `payee` as separate keys would be a permanent
/// papercut.
///
/// Normalisation is ASCII-only. Unicode case folding would map codepoints
/// outside the charset onto it — `U+212A` KELVIN SIGN folds to `k` — so a
/// key no reader would call ASCII would pass validation and collide with an
/// ASCII one.
///
/// Re-exported from the crate root as [`crate::MetaKey`].
///
/// # Example
///
/// ```
/// use bc_models::MetaKey;
///
/// let key = MetaKey::new("Payee").expect("valid key");
/// assert_eq!(key.as_str(), "payee");
/// assert!(MetaKey::new("1nvoice").is_err());
/// ```
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(try_from = "String", into = "String")]
#[non_exhaustive]
pub struct MetaKey(String);

impl MetaKey {
    /// Normalises `key` to ASCII lowercase and validates it.
    ///
    /// # Arguments
    ///
    /// * `key` - The raw key, in any case.
    ///
    /// # Returns
    ///
    /// The normalised key.
    ///
    /// # Errors
    ///
    /// Returns [`KeyError::Empty`] for an empty key, [`KeyError::LeadingChar`]
    /// when the first character is not an ASCII letter,
    /// [`KeyError::InvalidChar`] for any later character outside
    /// `[a-z0-9_-]`, and [`KeyError::TooLong`] beyond 64 bytes.
    #[inline]
    pub fn new(key: impl Into<String>) -> Result<Self, KeyError> {
        let mut normalised = key.into();
        normalised.make_ascii_lowercase();
        let mut chars = normalised.chars();
        let Some(first) = chars.next() else {
            return Err(KeyError::Empty);
        };
        if !first.is_ascii_lowercase() {
            return Err(KeyError::LeadingChar { found: first });
        }
        if let Some(found) = chars
            .find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'))
        {
            return Err(KeyError::InvalidChar { found });
        }
        if normalised.len() > MAX_KEY_BYTES {
            return Err(KeyError::TooLong {
                len: normalised.len(),
            });
        }
        Ok(Self(normalised))
    }

    /// Returns the normalised key as a string slice.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for MetaKey {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for MetaKey {
    type Err = KeyError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<String> for MetaKey {
    type Error = KeyError;

    #[inline]
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<MetaKey> for String {
    #[inline]
    fn from(key: MetaKey) -> Self {
        key.0
    }
}

/// The type of a metadata value, without the value.
///
/// Every registered key carries one of these; [`MetaValue::ty`] projects a
/// value onto its type. The serde representation (`text`, `number`, …) is the
/// form stored in the key registry.
///
/// Re-exported from the crate root as [`crate::MetaType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[expect(
    clippy::exhaustive_enums,
    reason = "the seven metadata types are closed by design, deviating from the \
              #[non_exhaustive] convention elsewhere in this crate. \
              #[non_exhaustive] buys semver-compatible variant addition for \
              consumers outside the crate; bc-models is unpublished and every \
              consumer is in this workspace, so that buys nothing. It would cost \
              the coercion matrix in bc-core its exhaustiveness check, where a \
              wildcard arm would turn an eighth type into a silent mismatch \
              instead of a build failure."
)]
pub enum MetaType {
    /// Free text.
    Text,
    /// An arbitrary-precision decimal number.
    Number,
    /// A boolean flag.
    Boolean,
    /// A calendar date with no time-of-day component.
    Date,
    /// An instant in time.
    Timestamp,
    /// A monetary amount: a value paired with a commodity code.
    Amount,
    /// A reference to an account.
    Account,
}

impl MetaType {
    /// Parses `text` as a value of this type.
    ///
    /// The inverse of [`MetaValue::canonical`]. [`MetaType::Text`] accepts
    /// anything; every other type accepts exactly the form `canonical`
    /// produces.
    ///
    /// # Arguments
    ///
    /// * `text` - The canonical string form to read.
    ///
    /// # Returns
    ///
    /// The parsed value.
    ///
    /// # Errors
    ///
    /// Returns the [`ValueError`] variant naming this type when `text` does
    /// not carry it.
    #[inline]
    pub fn parse_value(self, text: &str) -> Result<MetaValue, ValueError> {
        match self {
            Self::Text => Ok(MetaValue::Text(text.to_owned())),
            Self::Number => text
                .parse::<Decimal>()
                .map(MetaValue::Number)
                .map_err(|_err| ValueError::Number {
                    text: text.to_owned(),
                }),
            Self::Boolean => match text {
                "true" => Ok(MetaValue::Boolean(true)),
                "false" => Ok(MetaValue::Boolean(false)),
                _other => Err(ValueError::Boolean {
                    text: text.to_owned(),
                }),
            },
            Self::Date => {
                text.parse::<Date>()
                    .map(MetaValue::Date)
                    .map_err(|_err| ValueError::Date {
                        text: text.to_owned(),
                    })
            }
            Self::Timestamp => {
                text.parse::<Timestamp>()
                    .map(MetaValue::Timestamp)
                    .map_err(|_err| ValueError::Timestamp {
                        text: text.to_owned(),
                    })
            }
            Self::Amount => {
                // Split on the *first* space: a decimal never contains one, and
                // a commodity code is unvalidated free text that may.
                let (value, commodity) =
                    text.split_once(' ').ok_or_else(|| ValueError::Amount {
                        text: text.to_owned(),
                    })?;
                if commodity.is_empty() {
                    return Err(ValueError::Amount {
                        text: text.to_owned(),
                    });
                }
                let parsed = value
                    .parse::<Decimal>()
                    .map_err(|_err| ValueError::Amount {
                        text: text.to_owned(),
                    })?;
                Ok(MetaValue::Amount(Amount::new(
                    parsed,
                    CommodityCode::new(commodity),
                )))
            }
            Self::Account => text
                .parse::<AccountId>()
                .map(MetaValue::Account)
                .map_err(|_err| ValueError::Account {
                    text: text.to_owned(),
                }),
        }
    }
}

/// Error returned when a string cannot be read as a given [`MetaType`].
///
/// Re-exported from the crate root as [`crate::MetaValueError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ValueError {
    /// The text was not a decimal number.
    #[error("'{text}' is not a number")]
    Number {
        /// The text that failed to parse.
        text: String,
    },
    /// The text was neither `true` nor `false`.
    #[error("'{text}' is not a boolean; expected 'true' or 'false'")]
    Boolean {
        /// The text that failed to parse.
        text: String,
    },
    /// The text was not a `YYYY-MM-DD` date.
    #[error("'{text}' is not a date")]
    Date {
        /// The text that failed to parse.
        text: String,
    },
    /// The text was not an RFC 3339 timestamp.
    #[error("'{text}' is not a timestamp")]
    Timestamp {
        /// The text that failed to parse.
        text: String,
    },
    /// The text was not a decimal value followed by a commodity code.
    #[error("'{text}' is not an amount; expected a value and a commodity, e.g. '42.00 AUD'")]
    Amount {
        /// The text that failed to parse.
        text: String,
    },
    /// The text was not an account id.
    #[error("'{text}' is not an account id")]
    Account {
        /// The text that failed to parse.
        text: String,
    },
}

/// A typed metadata value.
///
/// Re-exported from the crate root as [`crate::MetaValue`].
///
/// # Example
///
/// ```
/// use bc_models::{MetaType, MetaValue};
///
/// let value = MetaValue::Boolean(true);
/// assert_eq!(value.ty(), MetaType::Boolean);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[expect(
    clippy::exhaustive_enums,
    reason = "the seven metadata types are closed by design; see MetaType for the \
              full reasoning"
)]
pub enum MetaValue {
    /// Free text.
    Text(String),
    /// An arbitrary-precision decimal number.
    Number(Decimal),
    /// A boolean flag.
    Boolean(bool),
    /// A calendar date with no time-of-day component.
    Date(Date),
    /// An instant in time.
    Timestamp(Timestamp),
    /// A monetary amount: a value paired with a commodity code.
    Amount(Amount),
    /// A reference to an account.
    Account(AccountId),
}

impl MetaValue {
    /// Returns the type of this value.
    #[inline]
    #[must_use]
    pub fn ty(&self) -> MetaType {
        match *self {
            Self::Text(_) => MetaType::Text,
            Self::Number(_) => MetaType::Number,
            Self::Boolean(_) => MetaType::Boolean,
            Self::Date(_) => MetaType::Date,
            Self::Timestamp(_) => MetaType::Timestamp,
            Self::Amount(_) => MetaType::Amount,
            Self::Account(_) => MetaType::Account,
        }
    }

    /// Returns this value's canonical string form.
    ///
    /// Every value has one, because both metadata tables store `value_text`
    /// `NOT NULL`. The form is exactly what [`MetaType::parse_value`] reads
    /// back, so the two are inverses for all seven types. It is a storage
    /// contract, not presentation: nothing here may be reshaped to look better
    /// on screen.
    ///
    /// [`MetaValue::Account`] yields the account's id. Rendering the account's
    /// path instead needs the account tree, which this crate cannot see; the
    /// storage layer substitutes the path where it has one.
    ///
    /// # Example
    ///
    /// ```
    /// use bc_models::{MetaType, MetaValue};
    ///
    /// let value = MetaValue::Boolean(true);
    /// assert_eq!(value.canonical(), "true");
    /// assert_eq!(MetaType::Boolean.parse_value("true"), Ok(value));
    /// ```
    #[inline]
    #[must_use]
    pub fn canonical(&self) -> String {
        match *self {
            Self::Text(ref text) => text.clone(),
            Self::Number(number) => number.to_string(),
            Self::Boolean(flag) => flag.to_string(),
            Self::Date(day) => day.to_string(),
            Self::Timestamp(stamp) => stamp.to_string(),
            Self::Amount(ref amount) => format!("{} {}", amount.value(), amount.commodity()),
            Self::Account(ref id) => id.to_string(),
        }
    }
}

/// One metadata key-value pair attached to a transaction or posting.
///
/// Re-exported from the crate root as [`crate::MetaEntry`].
///
/// # Example
///
/// ```
/// use bc_models::{MetaEntry, MetaKey, MetaValue};
///
/// let entry = MetaEntry::new(
///     MetaKey::new("payee").expect("valid key"),
///     MetaValue::Text("Generic Grocer".to_owned()),
/// );
///
/// assert_eq!(entry.key().as_str(), "payee");
/// assert!(!entry.mismatched());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(try_from = "Repr")]
#[non_exhaustive]
pub struct MetaEntry {
    /// The key this value is filed under. Normalised to lowercase.
    key: MetaKey,

    /// The value. When `mismatched` is set this is always
    /// [`MetaValue::Text`], holding the value's canonical string form.
    value: MetaValue,

    /// `true` when the value could not be represented in the key's registered
    /// type. The value is kept as text and flagged rather than rejected, so
    /// nothing is lost and no import blocks.
    ///
    /// This is an output, read on entries that came out of storage. A write
    /// derives the stored flag from the value against the key's registered
    /// type and discards whatever the incoming entry claims, so an entry whose
    /// value has since been repaired stops being flagged.
    mismatched: bool,
}

/// Deserialisation shim for [`MetaEntry`].
///
/// The flagged-implies-text invariant is a property of the pair, so it cannot
/// be checked field by field as serde fills them in. This mirrors the wire
/// shape, and [`MetaEntry`]'s `TryFrom` rejects the pairs the type forbids.
#[derive(serde::Deserialize)]
struct Repr {
    /// The key this value is filed under.
    key: MetaKey,
    /// The value, before the flagged-implies-text check.
    value: MetaValue,
    /// Whether the value was flagged as not fitting its key's type.
    #[serde(default)]
    mismatched: bool,
}

/// Error returned when a serialised [`MetaEntry`] breaks the flagged-value
/// invariant.
///
/// Re-exported from the crate root as [`crate::MetaEntryError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EntryError {
    /// A flagged entry carried a value other than [`MetaValue::Text`].
    #[error("a mismatched metadata entry must hold text, found {found:?}")]
    FlaggedNonText {
        /// Type of the offending value.
        found: MetaType,
    },
}

impl TryFrom<Repr> for MetaEntry {
    type Error = EntryError;

    #[inline]
    fn try_from(repr: Repr) -> Result<Self, Self::Error> {
        if repr.mismatched && repr.value.ty() != MetaType::Text {
            return Err(EntryError::FlaggedNonText {
                found: repr.value.ty(),
            });
        }
        Ok(Self {
            key: repr.key,
            value: repr.value,
            mismatched: repr.mismatched,
        })
    }
}

impl MetaEntry {
    /// Creates an entry whose value fits its key's registered type.
    ///
    /// Use [`MetaEntry::mismatch`] for a value that did not fit. A write
    /// derives the stored flag afresh, so neither constructor can force an
    /// entry to store as mismatched.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to file the value under.
    /// * `value` - The value.
    #[inline]
    #[must_use]
    pub fn new(key: MetaKey, value: MetaValue) -> Self {
        Self {
            key,
            value,
            mismatched: false,
        }
    }

    /// Creates an entry flagged as not fitting its key's registered type.
    ///
    /// The raw text is preserved verbatim, so nothing is lost and no import
    /// blocks. Taking the text rather than a [`MetaValue`] is what makes the
    /// flagged-implies-text invariant hold by construction.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to file the value under.
    /// * `raw` - The value's text, as it arrived.
    #[inline]
    #[must_use]
    pub fn mismatch(key: MetaKey, raw: impl Into<String>) -> Self {
        Self {
            key,
            value: MetaValue::Text(raw.into()),
            mismatched: true,
        }
    }

    /// Returns the key.
    #[inline]
    #[must_use]
    pub fn key(&self) -> &MetaKey {
        &self.key
    }

    /// Returns the value.
    #[inline]
    #[must_use]
    pub fn value(&self) -> &MetaValue {
        &self.value
    }

    /// Returns `true` when the value did not fit its key's registered type.
    #[inline]
    #[must_use]
    pub fn mismatched(&self) -> bool {
        self.mismatched
    }
}

/// An ordered list of metadata entries.
///
/// Repeated keys are permitted; insertion order is preserved and is the
/// display order. No uniqueness constraint applies anywhere.
///
/// Re-exported from the crate root as [`crate::Metadata`].
///
/// # Example
///
/// ```
/// use bc_models::{MetaEntry, MetaKey, MetaValue, Metadata};
///
/// let note = MetaKey::new("note").expect("valid key");
/// let mut meta = Metadata::default();
/// meta.push(MetaEntry::new(note.clone(), MetaValue::Text("first".to_owned())));
/// meta.push(MetaEntry::new(note.clone(), MetaValue::Text("second".to_owned())));
///
/// assert_eq!(meta.get_all(&note).count(), 2);
/// assert_eq!(meta.get_first_text(&note), Some("first"));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct Metadata(Vec<MetaEntry>);

impl Metadata {
    /// Wraps a list of entries, preserving their order.
    #[inline]
    #[must_use]
    pub fn new(entries: Vec<MetaEntry>) -> Self {
        Self(entries)
    }

    /// Returns every entry in display order.
    #[inline]
    #[must_use]
    pub fn entries(&self) -> &[MetaEntry] {
        &self.0
    }

    /// Returns the number of entries, counting repeated keys separately.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` when there are no entries.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterates over the entries in display order.
    ///
    /// This is the only view that reaches [`MetaEntry::mismatched`]; the
    /// value-level accessors below drop it.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = &MetaEntry> {
        self.0.iter()
    }

    /// Iterates over every value filed under `key`, in display order.
    #[inline]
    pub fn get_all<'meta>(&'meta self, key: &MetaKey) -> impl Iterator<Item = &'meta MetaValue> {
        self.0
            .iter()
            .filter(move |e| e.key() == key)
            .map(MetaEntry::value)
    }

    /// Returns the first value filed under `key`, if any.
    #[inline]
    #[must_use]
    pub fn get_first(&self, key: &MetaKey) -> Option<&MetaValue> {
        self.0.iter().find(|e| e.key() == key).map(MetaEntry::value)
    }

    /// Returns the first value filed under `key` when it is
    /// [`MetaValue::Text`], and `None` when the key is absent or its first
    /// value carries another type.
    #[inline]
    #[must_use]
    pub fn get_first_text(&self, key: &MetaKey) -> Option<&str> {
        match self.get_first(key) {
            Some(MetaValue::Text(text)) => Some(text),
            Some(
                MetaValue::Number(_)
                | MetaValue::Boolean(_)
                | MetaValue::Date(_)
                | MetaValue::Timestamp(_)
                | MetaValue::Amount(_)
                | MetaValue::Account(_),
            )
            | None => None,
        }
    }

    /// Compares two lists by key, by value and by position, ignoring
    /// [`MetaEntry::mismatched`].
    ///
    /// The derived [`PartialEq`] includes the flag, which is rarely what a
    /// caller wants: the flag is the store's verdict at load time, derived by
    /// the write path from the value against the key's registered type and
    /// overwritten on every write. Two lists differing in it alone therefore
    /// describe the same edit.
    ///
    /// # Arguments
    ///
    /// * `other` - The list to compare against.
    ///
    /// # Returns
    ///
    /// `true` when the two lists agree in everything an edit can express.
    ///
    /// # Example
    ///
    /// ```
    /// use bc_models::{MetaEntry, MetaKey, MetaValue, Metadata};
    ///
    /// let invoice = MetaKey::new("invoice").expect("valid key");
    /// let flagged = Metadata::new(vec![MetaEntry::mismatch(invoice.clone(), "1502")]);
    /// let plain = Metadata::new(vec![MetaEntry::new(
    ///     invoice,
    ///     MetaValue::Text("1502".to_owned()),
    /// )]);
    ///
    /// assert!(flagged.eq_ignoring_mismatched(&plain));
    /// assert!(flagged != plain);
    /// ```
    #[inline]
    #[must_use]
    pub fn eq_ignoring_mismatched(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .all(|(mine, theirs)| mine.key() == theirs.key() && mine.value() == theirs.value())
    }

    /// Appends `entry` after every existing entry.
    #[inline]
    pub fn push(&mut self, entry: MetaEntry) {
        self.0.push(entry);
    }

    /// Removes every entry filed under `key` and returns them in display
    /// order. The surviving entries keep their relative order.
    #[inline]
    pub fn remove_all(&mut self, key: &MetaKey) -> Vec<MetaEntry> {
        let mut removed = Vec::new();
        let mut kept = Vec::with_capacity(self.0.len());
        for entry in self.0.drain(..) {
            if entry.key() == key {
                removed.push(entry);
            } else {
                kept.push(entry);
            }
        }
        self.0 = kept;
        removed
    }
}

impl FromIterator<MetaEntry> for Metadata {
    #[inline]
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = MetaEntry>,
    {
        Self(iter.into_iter().collect())
    }
}

/// A registry entry binding a metadata key to its value type.
///
/// Every key is a user key: there are no reserved or built-in keys, and every
/// key can be renamed and retyped. A key enters the registry on first write,
/// with its type inferred from the value.
///
/// Re-exported from the crate root as [`crate::MetaKeyDef`].
///
/// # Example
///
/// ```
/// use bc_models::{MetaKey, MetaKeyDef, MetaType};
/// use jiff::Timestamp;
///
/// let def = MetaKeyDef::builder()
///     .key(MetaKey::new("invoice").expect("valid key"))
///     .ty(MetaType::Number)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(def.ty(), MetaType::Number);
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct MetaKeyDef {
    /// The registered key, normalised to lowercase.
    key: MetaKey,

    /// The type every value under this key is coerced towards. A value that
    /// will not coerce is stored as text and flagged, never rejected.
    ty: MetaType,

    /// Timestamp recorded when this key was first registered. Callers
    /// registering a new key should pass [`jiff::Timestamp::now()`].
    created_at: Timestamp,
}

impl MetaKeyDef {
    /// Returns the registered key.
    #[inline]
    #[must_use]
    pub fn key(&self) -> &MetaKey {
        &self.key
    }

    /// Returns the registered value type.
    #[inline]
    #[must_use]
    pub fn ty(&self) -> MetaType {
        self.ty
    }

    /// Returns the registration timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

impl<'meta> IntoIterator for &'meta Metadata {
    type IntoIter = core::slice::Iter<'meta, MetaEntry>;
    type Item = &'meta MetaEntry;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::AccountId;
    use crate::money::Amount;
    use crate::money::CommodityCode;

    #[rstest]
    #[case("payee")]
    #[case("invoice")]
    #[case("a")]
    #[case("a-b_c")]
    #[case("x9")]
    fn meta_key_accepts_valid_keys(#[case] input: &str) {
        let key = MetaKey::new(input).expect("key should be valid");
        assert_eq!(key.as_str(), input);
    }

    #[rstest]
    #[case("Payee", "payee")]
    #[case("INVOICE", "invoice")]
    #[case("Due-Date", "due-date")]
    fn meta_key_normalises_to_lowercase(#[case] input: &str, #[case] expected: &str) {
        let key = MetaKey::new(input).expect("key should be valid");
        assert_eq!(key.as_str(), expected);
        assert_eq!(key, MetaKey::new(expected).expect("key should be valid"));
    }

    #[test]
    fn meta_key_rejects_empty() {
        assert_eq!(MetaKey::new(""), Err(KeyError::Empty));
    }

    #[rstest]
    #[case("1abc", '1')]
    #[case("_abc", '_')]
    #[case("-abc", '-')]
    #[case("9", '9')]
    fn meta_key_rejects_non_letter_first_char(#[case] input: &str, #[case] found: char) {
        assert_eq!(MetaKey::new(input), Err(KeyError::LeadingChar { found }));
    }

    #[rstest]
    #[case("pay ee", ' ')]
    #[case("pay:ee", ':')]
    #[case("pay.ee", '.')]
    #[case("payée", 'é')]
    fn meta_key_rejects_bad_charset(#[case] input: &str, #[case] found: char) {
        assert_eq!(MetaKey::new(input), Err(KeyError::InvalidChar { found }));
    }

    /// `U+212A` KELVIN SIGN folds to ASCII `k` under Unicode case mapping, so
    /// Unicode lowercasing here would admit a key outside the documented
    /// charset and collide it with the ASCII spelling.
    #[test]
    fn meta_key_rejects_unicode_folding_onto_the_ascii_charset() {
        assert_eq!(
            MetaKey::new("\u{212A}elvin"),
            Err(KeyError::LeadingChar { found: '\u{212A}' })
        );
        assert_eq!(
            MetaKey::new("in\u{212A}oice"),
            Err(KeyError::InvalidChar { found: '\u{212A}' })
        );
    }

    #[test]
    fn meta_key_accepts_64_bytes() {
        let input = "a".repeat(64);
        assert_eq!(
            MetaKey::new(input.clone())
                .expect("64 bytes should be valid")
                .as_str(),
            input
        );
    }

    #[test]
    fn meta_key_rejects_65_bytes() {
        assert_eq!(
            MetaKey::new("a".repeat(65)),
            Err(KeyError::TooLong { len: 65 })
        );
    }

    #[test]
    fn meta_key_displays_normalised_form() {
        let key = MetaKey::new("Payee").expect("key should be valid");
        assert_eq!(key.to_string(), "payee");
    }

    #[test]
    fn meta_key_parses_from_str() {
        assert_eq!(
            MetaKey::from_str("Invoice").expect("key should be valid"),
            MetaKey::new("invoice").expect("key should be valid")
        );
        assert_eq!(
            MetaKey::from_str("1nvoice"),
            Err(KeyError::LeadingChar { found: '1' })
        );
    }

    #[test]
    fn meta_key_serialises_to_bare_string() {
        let key = MetaKey::new("payee").expect("key should be valid");
        let json = serde_json::to_string(&key).expect("serialize should succeed");
        assert_eq!(json, "\"payee\"");
    }

    #[test]
    fn meta_key_deserialises_normalising_and_validating() {
        let key: MetaKey = serde_json::from_str("\"Payee\"").expect("deserialize should succeed");
        assert_eq!(key.as_str(), "payee");
        let bad = serde_json::from_str::<MetaKey>("\"1bad\"").ok();
        assert_eq!(bad, None);
    }

    fn sample_values() -> Vec<MetaValue> {
        vec![
            MetaValue::Text("hello".to_owned()),
            MetaValue::Number(dec!(1502.50)),
            MetaValue::Boolean(true),
            MetaValue::Date(date(2026, 1, 15)),
            MetaValue::Timestamp(Timestamp::from_second(1_700_000_000).expect("valid timestamp")),
            MetaValue::Amount(Amount::new(dec!(42.00), CommodityCode::new("AUD"))),
            MetaValue::Account(AccountId::new()),
        ]
    }

    #[test]
    fn meta_value_ty_projects_every_variant() {
        let expected = vec![
            MetaType::Text,
            MetaType::Number,
            MetaType::Boolean,
            MetaType::Date,
            MetaType::Timestamp,
            MetaType::Amount,
            MetaType::Account,
        ];
        let actual: Vec<MetaType> = sample_values().iter().map(MetaValue::ty).collect();
        assert_eq!(actual, expected);
    }

    #[test]
    fn meta_value_round_trips_through_json() {
        for value in sample_values() {
            let json = serde_json::to_string(&value).expect("serialize should succeed");
            let back: MetaValue = serde_json::from_str(&json).expect("deserialize should succeed");
            assert_eq!(value, back);
        }
    }

    #[test]
    fn meta_value_is_externally_tagged_snake_case() {
        let json = serde_json::to_string(&MetaValue::Text("hi".to_owned()))
            .expect("serialize should succeed");
        assert_eq!(json, "{\"text\":\"hi\"}");
    }

    #[rstest]
    #[case(MetaType::Text, "\"text\"")]
    #[case(MetaType::Number, "\"number\"")]
    #[case(MetaType::Boolean, "\"boolean\"")]
    #[case(MetaType::Date, "\"date\"")]
    #[case(MetaType::Timestamp, "\"timestamp\"")]
    #[case(MetaType::Amount, "\"amount\"")]
    #[case(MetaType::Account, "\"account\"")]
    fn meta_type_serialises_to_registry_name(#[case] ty: MetaType, #[case] expected: &str) {
        let json = serde_json::to_string(&ty).expect("serialize should succeed");
        assert_eq!(json, expected);
        let back: MetaType = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(ty, back);
    }

    fn key(name: &str) -> MetaKey {
        MetaKey::new(name).expect("key should be valid")
    }

    fn text_entry(name: &str, text: &str) -> MetaEntry {
        MetaEntry::new(key(name), MetaValue::Text(text.to_owned()))
    }

    #[test]
    fn meta_entry_new_is_not_mismatched() {
        let entry = text_entry("payee", "Generic Grocer");
        assert_eq!(entry.key(), &key("payee"));
        assert_eq!(entry.value(), &MetaValue::Text("Generic Grocer".to_owned()));
        assert!(!entry.mismatched());
    }

    #[test]
    fn meta_entry_records_a_mismatch_as_text() {
        let entry = MetaEntry::mismatch(key("invoice"), "not-a-number");
        assert!(entry.mismatched());
        assert_eq!(entry.key(), &key("invoice"));
        assert_eq!(entry.value(), &MetaValue::Text("not-a-number".to_owned()));
    }

    #[test]
    fn meta_entry_deserialise_rejects_a_flagged_non_text_value() {
        let json = r#"{"key":"invoice","value":{"number":"1502"},"mismatched":true}"#;
        let err = serde_json::from_str::<MetaEntry>(json)
            .expect_err("a flagged non-text value should be rejected");
        assert!(
            err.to_string().contains("must hold text"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn meta_entry_deserialise_accepts_a_flagged_text_value() {
        let json = r#"{"key":"invoice","value":{"text":"not-a-number"},"mismatched":true}"#;
        let entry: MetaEntry = serde_json::from_str(json).expect("deserialize should succeed");
        assert_eq!(entry, MetaEntry::mismatch(key("invoice"), "not-a-number"));
    }

    #[test]
    fn meta_entry_mismatch_round_trips_through_json() {
        let entry = MetaEntry::mismatch(key("invoice"), "not-a-number");
        let json = serde_json::to_string(&entry).expect("serialize should succeed");
        let back: MetaEntry = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(entry, back);
    }

    #[test]
    fn meta_entry_round_trips_through_json() {
        let entry = text_entry("note", "doctor's appointment");
        let json = serde_json::to_string(&entry).expect("serialize should succeed");
        let back: MetaEntry = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(entry, back);
    }

    #[test]
    fn metadata_default_is_empty() {
        let meta = Metadata::default();
        assert!(meta.is_empty());
        assert_eq!(meta.len(), 0);
        assert_eq!(meta.get_first(&key("payee")), None);
        assert_eq!(meta.get_all(&key("payee")).count(), 0);
    }

    #[test]
    fn metadata_preserves_insertion_order_across_keys() {
        let mut meta = Metadata::default();
        meta.push(text_entry("payee", "Generic Grocer"));
        meta.push(text_entry("note", "weekly shop"));
        meta.push(text_entry("channel", "card"));
        let keys: Vec<&str> = meta.iter().map(|e| e.key().as_str()).collect();
        assert_eq!(keys, vec!["payee", "note", "channel"]);
    }

    #[test]
    fn metadata_permits_repeated_keys() {
        let mut meta = Metadata::default();
        meta.push(text_entry("note", "first"));
        meta.push(text_entry("payee", "Generic Grocer"));
        meta.push(text_entry("note", "second"));

        let notes: Vec<&MetaValue> = meta.get_all(&key("note")).collect();
        assert_eq!(
            notes,
            vec![
                &MetaValue::Text("first".to_owned()),
                &MetaValue::Text("second".to_owned()),
            ]
        );
        assert_eq!(
            meta.get_first(&key("note")),
            Some(&MetaValue::Text("first".to_owned()))
        );
    }

    #[test]
    fn metadata_iter_reaches_the_mismatched_flag() {
        let mut meta = Metadata::default();
        meta.push(text_entry("payee", "Generic Grocer"));
        meta.push(MetaEntry::mismatch(key("invoice"), "not-a-number"));
        let flagged: Vec<&str> = meta
            .iter()
            .filter(|e| e.mismatched())
            .map(|e| e.key().as_str())
            .collect();
        assert_eq!(flagged, vec!["invoice"]);
    }

    #[test]
    fn metadata_remove_all_returns_entries_and_keeps_the_rest_ordered() {
        let mut meta = Metadata::default();
        meta.push(text_entry("note", "first"));
        meta.push(text_entry("payee", "Generic Grocer"));
        meta.push(text_entry("note", "second"));
        meta.push(text_entry("channel", "card"));

        let removed = meta.remove_all(&key("note"));
        assert_eq!(
            removed,
            vec![text_entry("note", "first"), text_entry("note", "second")]
        );

        let keys: Vec<&str> = meta.iter().map(|e| e.key().as_str()).collect();
        assert_eq!(keys, vec!["payee", "channel"]);
        assert_eq!(meta.remove_all(&key("absent")), vec![]);
    }

    #[test]
    fn metadata_get_first_text_reads_only_text_values() {
        let mut meta = Metadata::default();
        meta.push(text_entry("payee", "Generic Grocer"));
        meta.push(MetaEntry::new(
            key("invoice"),
            MetaValue::Number(dec!(1502)),
        ));
        assert_eq!(meta.get_first_text(&key("payee")), Some("Generic Grocer"));
        assert_eq!(meta.get_first_text(&key("invoice")), None);
        assert_eq!(meta.get_first_text(&key("absent")), None);
    }

    #[test]
    fn eq_ignoring_mismatched_ignores_only_the_flag() {
        let flagged = Metadata::new(vec![MetaEntry::mismatch(key("invoice"), "1502")]);
        let plain = Metadata::new(vec![text_entry("invoice", "1502")]);

        assert!(flagged.eq_ignoring_mismatched(&plain));
        pretty_assertions::assert_ne!(
            flagged,
            plain,
            "the derived PartialEq still sees the flag; the method is the only way past it"
        );
    }

    #[rstest]
    #[case::value(vec![text_entry("note", "second"), text_entry("payee", "Generic Grocer")])]
    #[case::key(vec![text_entry("memo", "first"), text_entry("payee", "Generic Grocer")])]
    #[case::length(vec![text_entry("note", "first")])]
    #[case::order_across_keys(vec![text_entry("payee", "Generic Grocer"), text_entry("note", "first")])]
    fn eq_ignoring_mismatched_sees_every_other_difference(#[case] other: Vec<MetaEntry>) {
        let base = Metadata::new(vec![
            text_entry("note", "first"),
            text_entry("payee", "Generic Grocer"),
        ]);
        assert!(!base.eq_ignoring_mismatched(&Metadata::new(other)));
    }

    #[test]
    fn metadata_serialises_as_a_bare_array() {
        let meta = Metadata::new(vec![text_entry("payee", "Generic Grocer")]);
        let json = serde_json::to_string(&meta).expect("serialize should succeed");
        assert!(json.starts_with('['));
        let back: Metadata = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(meta, back);
    }

    #[test]
    fn metadata_collects_from_an_iterator_in_order() {
        let meta: Metadata = vec![
            text_entry("payee", "Generic Grocer"),
            text_entry("note", "n"),
        ]
        .into_iter()
        .collect();
        let keys: Vec<&str> = meta.iter().map(|e| e.key().as_str()).collect();
        assert_eq!(keys, vec!["payee", "note"]);
    }

    #[test]
    fn meta_key_def_carries_key_type_and_creation_time() {
        let created_at = Timestamp::from_second(1_700_000_000).expect("valid timestamp");
        let def = MetaKeyDef::builder()
            .key(key("invoice"))
            .ty(MetaType::Number)
            .created_at(created_at)
            .build();

        assert_eq!(def.key(), &key("invoice"));
        assert_eq!(def.ty(), MetaType::Number);
        assert_eq!(def.created_at(), &created_at);
    }

    #[test]
    fn meta_key_def_round_trips_through_json() {
        let def = MetaKeyDef::builder()
            .key(key("payee"))
            .ty(MetaType::Text)
            .created_at(Timestamp::from_second(1_700_000_000).expect("valid timestamp"))
            .build();
        let json = serde_json::to_string(&def).expect("serialize should succeed");
        let back: MetaKeyDef = serde_json::from_str(&json).expect("deserialize should succeed");
        assert_eq!(def, back);
    }

    #[rstest]
    #[case(MetaValue::Text("Generic Grocer".to_owned()), "Generic Grocer")]
    #[case(MetaValue::Number(dec!(1502.50)), "1502.50")]
    #[case(MetaValue::Boolean(true), "true")]
    #[case(MetaValue::Boolean(false), "false")]
    #[case(MetaValue::Date(date(2026, 1, 15)), "2026-01-15")]
    #[case(
        MetaValue::Amount(Amount::new(dec!(42.00), CommodityCode::new("AUD"))),
        "42.00 AUD"
    )]
    fn meta_value_canonical_form_is_stable(#[case] value: MetaValue, #[case] expected: &str) {
        assert_eq!(value.canonical(), expected);
    }

    #[test]
    fn meta_value_canonical_number_keeps_trailing_zeros() {
        assert_eq!(MetaValue::Number(dec!(1.500)).canonical(), "1.500");
    }

    #[test]
    fn meta_value_canonical_timestamp_is_rfc_3339() {
        let stamp = Timestamp::from_second(1_700_000_000).expect("valid timestamp");
        assert_eq!(
            MetaValue::Timestamp(stamp).canonical(),
            "2023-11-14T22:13:20Z"
        );
    }

    #[test]
    fn meta_value_canonical_account_is_the_id() {
        let id = AccountId::new();
        assert_eq!(MetaValue::Account(id.clone()).canonical(), id.to_string());
    }

    #[test]
    fn canonical_and_parse_value_are_inverses() {
        for value in sample_values() {
            let text = value.canonical();
            let back = value
                .ty()
                .parse_value(&text)
                .expect("canonical form must parse back");
            assert_eq!(value, back);
        }
    }

    #[test]
    fn parse_value_accepts_any_text_as_text() {
        assert_eq!(
            MetaType::Text.parse_value("not a number at all"),
            Ok(MetaValue::Text("not a number at all".to_owned()))
        );
    }

    #[rstest]
    #[case(MetaType::Number, "not-a-number")]
    #[case(MetaType::Boolean, "yes")]
    #[case(MetaType::Date, "2026-13-99")]
    #[case(MetaType::Timestamp, "yesterday")]
    #[case(MetaType::Amount, "42.00")]
    #[case(MetaType::Amount, "42.00 ")]
    #[case(MetaType::Account, "not an id")]
    // An account *path* is what a tombstoned entry's `value_text` retains. It
    // is deliberately not an id, so such an entry reads back as flagged text.
    #[case(MetaType::Account, "Assets:Bank:Savings")]
    fn parse_value_rejects_text_that_is_not_the_type(#[case] ty: MetaType, #[case] text: &str) {
        assert_eq!(ty.parse_value(text).ok(), None);
    }

    #[test]
    fn parse_value_boolean_is_exact() {
        assert_eq!(
            MetaType::Boolean.parse_value("true"),
            Ok(MetaValue::Boolean(true))
        );
        assert_eq!(
            MetaType::Boolean.parse_value("false"),
            Ok(MetaValue::Boolean(false))
        );
        assert_eq!(MetaType::Boolean.parse_value("True").ok(), None);
        assert_eq!(MetaType::Boolean.parse_value("1").ok(), None);
    }

    #[test]
    fn parse_value_amount_splits_on_the_first_space() {
        assert_eq!(
            MetaType::Amount.parse_value("42.00 AUD"),
            Ok(MetaValue::Amount(Amount::new(
                dec!(42.00),
                CommodityCode::new("AUD")
            )))
        );
    }

    #[test]
    fn amount_round_trips_a_commodity_holding_a_space() {
        let value = MetaValue::Amount(Amount::new(
            dec!(42.00),
            CommodityCode::new("SHARE CLASS A"),
        ));

        let text = value.canonical();

        assert_eq!(text, "42.00 SHARE CLASS A");
        assert_eq!(
            MetaType::Amount.parse_value(&text),
            Ok(value),
            "a commodity code is unvalidated free text, so the split must not depend on it"
        );
    }
}
