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
}
