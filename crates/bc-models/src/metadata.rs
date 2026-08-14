//! Typed key-value metadata attachable to transactions and postings.

use core::fmt;
use core::str::FromStr;

use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::AccountId;
use crate::money::Amount;

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
    /// Normalises `key` to lowercase and validates it.
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
        let normalised = key.into().to_lowercase();
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
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct MetaEntry {
    /// The key this value is filed under. Normalised to lowercase.
    key: MetaKey,

    /// The value. When `mismatched` is set this is always
    /// [`MetaValue::Text`], holding the value's canonical string form.
    value: MetaValue,

    /// `true` when the incoming value could not be represented in the key's
    /// registered type. The value is kept as text and flagged rather than
    /// rejected, so nothing is lost and no import blocks. Defaults to `false`.
    #[builder(default)]
    mismatched: bool,
}

impl MetaEntry {
    /// Creates an entry whose value fits its key's registered type.
    ///
    /// Use [`MetaEntry::builder`] to construct an entry flagged as
    /// `mismatched`.
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
    use core::str::FromStr as _;

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
    fn meta_entry_builder_defaults_to_not_mismatched() {
        let entry = MetaEntry::builder()
            .key(key("payee"))
            .value(MetaValue::Text("Generic Grocer".to_owned()))
            .build();
        assert!(!entry.mismatched());
        assert_eq!(entry, text_entry("payee", "Generic Grocer"));
    }

    #[test]
    fn meta_entry_records_a_mismatch() {
        let entry = MetaEntry::builder()
            .key(key("invoice"))
            .value(MetaValue::Text("not-a-number".to_owned()))
            .mismatched(true)
            .build();
        assert!(entry.mismatched());
    }

    #[test]
    fn meta_entry_round_trips_through_json() {
        let entry = text_entry("note", "docter's appointment");
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
        meta.push(
            MetaEntry::builder()
                .key(key("invoice"))
                .value(MetaValue::Text("not-a-number".to_owned()))
                .mismatched(true)
                .build(),
        );
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
}
