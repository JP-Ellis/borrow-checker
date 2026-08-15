//! Typed key-value metadata at the IPC boundary.
//!
//! These four types mirror `bc_models`'s metadata model one for one, minus the
//! parts a serde contract cannot carry: a key is a plain [`String`] here, and
//! an account is its id string rather than a resolved path. Resolving that path
//! for display is the frontend's, from the account tree it already holds —
//! carrying it here would make the conversion walk the domain, which this
//! crate's rules place in a `bc-core` extension trait.

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;

use crate::Amount;

// MARK: Key validation

/// Maximum length of a metadata key, in bytes.
///
/// Keys are restricted to ASCII, so this is also a character count and serves
/// directly as an input's `maxlength`.
pub const META_KEY_MAX_BYTES: usize = 64;

/// Why a string cannot be used as a metadata key.
///
/// Carried instead of a bare `false` so a frontend can say which rule the input
/// broke while the user is still typing. [`core::fmt::Display`] renders each
/// variant as a complete sentence, so a caller needs no match arm of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum MetaKeyProblem {
    /// The key was empty; a metadata key requires at least one character.
    #[error("a metadata key must not be empty")]
    Empty,
    /// The first character was not an ASCII letter.
    #[error("a metadata key must start with a letter, found '{found}'")]
    LeadingChar {
        /// The offending first character, after lowercasing.
        found: char,
    },
    /// A character outside `[a-z0-9_-]` appeared after the first.
    #[error("a metadata key may only contain letters, digits, '_' and '-', found '{found}'")]
    InvalidChar {
        /// The offending character, after lowercasing.
        found: char,
    },
    /// The key exceeded [`META_KEY_MAX_BYTES`].
    #[error(
        "a metadata key must be at most {} bytes, found {len}",
        META_KEY_MAX_BYTES
    )]
    TooLong {
        /// Length of the offending key in bytes, after lowercasing.
        len: usize,
    },
}

/// Lowercases `key`, without judging whether the result is usable.
///
/// This is the normalisation half of [`validate_meta_key`], split out because a
/// frontend wants it on every keystroke: the backend lowercases silently, so an
/// input that does not show the user what will actually be stored lies to them.
/// Folding is ASCII-only, matching the backend — Unicode folding would map
/// codepoints outside the charset onto it, and `U+212A` KELVIN SIGN would reach
/// the registry as a colliding `k`.
///
/// # Arguments
///
/// * `key` - The raw key, in any case.
///
/// # Returns
///
/// The key with every ASCII letter lowercased.
///
/// # Example
///
/// ```
/// # use bc_ipc::normalise_meta_key;
/// assert_eq!(normalise_meta_key("Payee"), "payee");
/// ```
#[must_use]
#[inline]
pub fn normalise_meta_key(key: &str) -> String {
    key.to_ascii_lowercase()
}

/// Normalises `key` and reports whether it is a usable metadata key.
///
/// Mirrors `bc_models::MetaKey::new`, which this crate cannot call: the domain
/// crate is unreachable from the default build, and that build is the one the
/// WASM frontend compiles. The rule is duplicated rather than shared, and the
/// `models`-gated test `mirror_agrees_with_the_domain_validator` fails the
/// moment the two disagree.
///
/// The backend validates again and answers an invalid key with
/// `BcError::Validation`. That safeguard is not weakened by this check; it is
/// simply too late to be the only one, since it fails a whole transaction save
/// over one malformed key.
///
/// # Arguments
///
/// * `key` - The raw key, in any case.
///
/// # Returns
///
/// The normalised key, ready to send.
///
/// # Errors
///
/// Returns the [`MetaKeyProblem`] naming the first rule the key broke. Rules
/// are checked in the backend's order — empty, leading character, charset,
/// length — so both sides reject a key for the same reason, not merely reject
/// it.
///
/// # Example
///
/// ```
/// # use bc_ipc::{validate_meta_key, MetaKeyProblem};
/// assert_eq!(validate_meta_key("Invoice"), Ok("invoice".to_owned()));
/// assert_eq!(
///     validate_meta_key("1nvoice"),
///     Err(MetaKeyProblem::LeadingChar { found: '1' })
/// );
/// ```
#[inline]
pub fn validate_meta_key(key: &str) -> Result<String, MetaKeyProblem> {
    let normalised = normalise_meta_key(key);
    let mut chars = normalised.chars();
    let Some(first) = chars.next() else {
        return Err(MetaKeyProblem::Empty);
    };
    if !first.is_ascii_lowercase() {
        return Err(MetaKeyProblem::LeadingChar { found: first });
    }
    if let Some(found) =
        chars.find(|c| !(c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_' || *c == '-'))
    {
        return Err(MetaKeyProblem::InvalidChar { found });
    }
    if normalised.len() > META_KEY_MAX_BYTES {
        return Err(MetaKeyProblem::TooLong {
            len: normalised.len(),
        });
    }
    Ok(normalised)
}

/// The type of a metadata value, without the value.
///
/// Every registered key carries one of these. The serde form (`text`,
/// `number`, …) matches `bc_models::MetaType`'s.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[expect(
    clippy::exhaustive_enums,
    reason = "the seven metadata types are closed by design, deviating from this \
              crate's #[non_exhaustive] convention. That convention buys \
              semver-compatible variant addition for consumers outside the \
              crate, and every consumer is in this workspace. The cost lands on \
              the frontend editor, which must offer an input per type: \
              #[non_exhaustive] would force a wildcard arm there that renders an \
              eighth type wrongly instead of failing the build."
)]
pub enum MetaTypeDto {
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

impl MetaTypeDto {
    /// Returns the lowercase display label for this type.
    ///
    /// # Example
    ///
    /// ```
    /// # use bc_ipc::MetaTypeDto;
    /// assert_eq!(MetaTypeDto::Timestamp.label(), "timestamp");
    /// ```
    #[must_use]
    #[inline]
    pub fn label(self) -> &'static str {
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
}

/// A typed metadata value.
///
/// [`Self::Account`] carries the account's id string. Rendering it as a path
/// needs the account tree, which the frontend holds and this crate does not.
///
/// # Example
///
/// ```
/// # use bc_ipc::{MetaTypeDto, MetaValueDto};
/// assert_eq!(MetaValueDto::Boolean(true).ty(), MetaTypeDto::Boolean);
/// ```
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[expect(
    clippy::exhaustive_enums,
    reason = "the seven metadata types are closed by design; see MetaTypeDto for \
              the full reasoning"
)]
pub enum MetaValueDto {
    /// Free text.
    Text(String),
    /// An arbitrary-precision decimal number.
    Number(Decimal),
    /// A boolean flag.
    Boolean(bool),
    /// A calendar date with no time-of-day component.
    Date(jiff::civil::Date),
    /// An instant in time.
    Timestamp(jiff::Timestamp),
    /// A monetary amount: a value paired with a currency code.
    Amount(Amount),
    /// An account id string.
    Account(String),
}

impl MetaValueDto {
    /// Returns the type of this value.
    #[must_use]
    #[inline]
    pub fn ty(&self) -> MetaTypeDto {
        match *self {
            Self::Text(_) => MetaTypeDto::Text,
            Self::Number(_) => MetaTypeDto::Number,
            Self::Boolean(_) => MetaTypeDto::Boolean,
            Self::Date(_) => MetaTypeDto::Date,
            Self::Timestamp(_) => MetaTypeDto::Timestamp,
            Self::Amount(_) => MetaTypeDto::Amount,
            Self::Account(_) => MetaTypeDto::Account,
        }
    }
}

/// One metadata key-value pair attached to a transaction or posting.
///
/// Repeated keys are legal and the list order is the display order, so an entry
/// is identified by its position rather than by its key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MetaEntryDto {
    /// The key, normalised to lowercase by the backend.
    pub key: String,
    /// The value.
    pub value: MetaValueDto,
    /// `true` when the stored value did not fit its key's registered type, as
    /// of the last write.
    ///
    /// Badge these entries as needing repair. This is an output only: the
    /// backend derives the flag afresh on every write and discards whatever an
    /// incoming entry claims, so an entry whose value has been repaired stops
    /// being flagged and one sent back flagged does not stay that way.
    pub mismatched: bool,
}

impl MetaEntryDto {
    /// Creates an unflagged entry.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to file the value under.
    /// * `value` - The value.
    #[must_use]
    #[inline]
    pub fn new(key: impl Into<String>, value: MetaValueDto) -> Self {
        Self {
            key: key.into(),
            value,
            mismatched: false,
        }
    }

    /// Creates a flagged entry, whose value is always the raw text that would
    /// not fit its key's registered type.
    ///
    /// Mirrors `bc_models::MetaEntry::mismatch`, and exists because
    /// [`MetaEntryDto`] is `#[non_exhaustive]`: no other crate can build a
    /// flagged entry by struct literal, which the frontend's editor tests and QA
    /// fixtures both need.
    ///
    /// # Arguments
    ///
    /// * `key` - The key to file the text under.
    /// * `text` - The raw text the store could not fit.
    #[must_use]
    #[inline]
    pub fn flagged(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: MetaValueDto::Text(text.into()),
            mismatched: true,
        }
    }
}

/// A registry entry binding a metadata key to its value type.
///
/// The registry's registration timestamp is deliberately absent: nothing in the
/// UI displays it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct MetaKeyDefDto {
    /// The registered key, normalised to lowercase.
    pub key: String,
    /// The type every value under this key is coerced towards.
    pub ty: MetaTypeDto,
}

impl MetaKeyDefDto {
    /// Creates a new [`MetaKeyDefDto`].
    ///
    /// # Arguments
    ///
    /// * `key` - The registered key.
    /// * `ty` - The registered value type.
    #[must_use]
    #[inline]
    pub fn new(key: impl Into<String>, ty: MetaTypeDto) -> Self {
        Self {
            key: key.into(),
            ty,
        }
    }
}

// MARK: models conversions

#[cfg(feature = "models")]
impl From<bc_models::MetaType> for MetaTypeDto {
    /// Converts a domain metadata type to its DTO. Both enums are exhaustive,
    /// so an eighth type fails to compile here.
    #[inline]
    fn from(ty: bc_models::MetaType) -> Self {
        match ty {
            bc_models::MetaType::Text => Self::Text,
            bc_models::MetaType::Number => Self::Number,
            bc_models::MetaType::Boolean => Self::Boolean,
            bc_models::MetaType::Date => Self::Date,
            bc_models::MetaType::Timestamp => Self::Timestamp,
            bc_models::MetaType::Amount => Self::Amount,
            bc_models::MetaType::Account => Self::Account,
        }
    }
}

#[cfg(feature = "models")]
impl From<MetaTypeDto> for bc_models::MetaType {
    /// Converts a metadata type DTO to its domain counterpart.
    #[inline]
    fn from(ty: MetaTypeDto) -> Self {
        match ty {
            MetaTypeDto::Text => Self::Text,
            MetaTypeDto::Number => Self::Number,
            MetaTypeDto::Boolean => Self::Boolean,
            MetaTypeDto::Date => Self::Date,
            MetaTypeDto::Timestamp => Self::Timestamp,
            MetaTypeDto::Amount => Self::Amount,
            MetaTypeDto::Account => Self::Account,
        }
    }
}

#[cfg(feature = "models")]
impl From<&bc_models::MetaValue> for MetaValueDto {
    /// Converts a domain metadata value to its DTO, rendering an account as its
    /// bare id.
    #[inline]
    fn from(value: &bc_models::MetaValue) -> Self {
        match *value {
            bc_models::MetaValue::Text(ref text) => Self::Text(text.clone()),
            bc_models::MetaValue::Number(number) => Self::Number(number),
            bc_models::MetaValue::Boolean(flag) => Self::Boolean(flag),
            bc_models::MetaValue::Date(day) => Self::Date(day),
            bc_models::MetaValue::Timestamp(stamp) => Self::Timestamp(stamp),
            bc_models::MetaValue::Amount(ref amount) => Self::Amount(Amount::from(amount)),
            bc_models::MetaValue::Account(ref id) => Self::Account(id.to_string()),
        }
    }
}

#[cfg(feature = "models")]
impl TryFrom<&MetaValueDto> for bc_models::MetaValue {
    type Error = crate::BcError;

    /// Converts a metadata value DTO to its domain counterpart.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::Validation`] when an account variant does not
    /// carry a parsable account id. Every other variant is infallible.
    #[inline]
    fn try_from(value: &MetaValueDto) -> Result<Self, Self::Error> {
        Ok(match *value {
            MetaValueDto::Text(ref text) => Self::Text(text.clone()),
            MetaValueDto::Number(number) => Self::Number(number),
            MetaValueDto::Boolean(flag) => Self::Boolean(flag),
            MetaValueDto::Date(day) => Self::Date(day),
            MetaValueDto::Timestamp(stamp) => Self::Timestamp(stamp),
            MetaValueDto::Amount(ref amount) => Self::Amount(bc_models::Amount::from(amount)),
            MetaValueDto::Account(ref id) => {
                Self::Account(id.parse::<bc_models::AccountId>().map_err(|e| {
                    crate::BcError::Validation(format!("invalid account id '{id}': {e}"))
                })?)
            }
        })
    }
}

#[cfg(feature = "models")]
impl From<&bc_models::MetaEntry> for MetaEntryDto {
    /// Converts a domain entry to its DTO, carrying the mismatch flag out so
    /// the frontend can badge the entry.
    #[inline]
    fn from(entry: &bc_models::MetaEntry) -> Self {
        Self {
            key: entry.key().as_str().to_owned(),
            value: MetaValueDto::from(entry.value()),
            mismatched: entry.mismatched(),
        }
    }
}

#[cfg(feature = "models")]
impl TryFrom<&MetaEntryDto> for bc_models::MetaEntry {
    type Error = crate::BcError;

    /// Converts an entry DTO to its domain counterpart, **discarding
    /// `mismatched`**.
    ///
    /// The flag is the store's verdict at load time, and the write path derives
    /// it afresh against the key's registered type. An incoming DTO does not
    /// get to assert it, exactly as an incoming domain entry does not.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::Validation`] when the key fails validation or
    /// the value carries an unparsable account id.
    #[inline]
    fn try_from(entry: &MetaEntryDto) -> Result<Self, Self::Error> {
        let key = bc_models::MetaKey::new(entry.key.clone()).map_err(|e| {
            crate::BcError::Validation(format!("invalid metadata key '{}': {e}", entry.key))
        })?;
        Ok(Self::new(
            key,
            bc_models::MetaValue::try_from(&entry.value)?,
        ))
    }
}

#[cfg(feature = "models")]
impl From<&bc_models::MetaKeyDef> for MetaKeyDefDto {
    /// Converts a registry entry to its DTO, dropping the registration
    /// timestamp.
    #[inline]
    fn from(def: &bc_models::MetaKeyDef) -> Self {
        Self::new(def.key().as_str(), MetaTypeDto::from(def.ty()))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::META_KEY_MAX_BYTES;
    use super::MetaEntryDto;
    use super::MetaKeyDefDto;
    use super::MetaKeyProblem;
    use super::MetaTypeDto;
    use super::MetaValueDto;
    use super::normalise_meta_key;
    use super::validate_meta_key;
    use crate::Amount;

    /// One sample of each of the seven value types, in variant order.
    fn sample_values() -> Vec<MetaValueDto> {
        vec![
            MetaValueDto::Text("Generic Grocer".to_owned()),
            MetaValueDto::Number(Decimal::new(150_250, 2)),
            MetaValueDto::Boolean(true),
            MetaValueDto::Date(jiff::civil::date(2026, 1, 15)),
            MetaValueDto::Timestamp(
                jiff::Timestamp::from_second(1_700_000_000).expect("valid timestamp"),
            ),
            MetaValueDto::Amount(Amount::new(Decimal::new(4200, 2), "AUD")),
            MetaValueDto::Account("account_00000000000000000000000000".to_owned()),
        ]
    }

    #[test]
    fn value_ty_projects_every_variant() {
        let actual: Vec<MetaTypeDto> = sample_values().iter().map(MetaValueDto::ty).collect();
        assert_eq!(
            actual,
            vec![
                MetaTypeDto::Text,
                MetaTypeDto::Number,
                MetaTypeDto::Boolean,
                MetaTypeDto::Date,
                MetaTypeDto::Timestamp,
                MetaTypeDto::Amount,
                MetaTypeDto::Account,
            ]
        );
    }

    #[test]
    fn value_is_externally_tagged_snake_case() {
        let json = serde_json::to_string(&MetaValueDto::Text("hi".to_owned())).expect("serialize");
        assert_eq!(json, "{\"text\":\"hi\"}");
    }

    #[test]
    fn every_value_round_trips_through_json() {
        for value in sample_values() {
            let json = serde_json::to_string(&value).expect("serialize");
            let back: MetaValueDto = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(value, back);
        }
    }

    #[rstest]
    #[case(MetaTypeDto::Text, "text")]
    #[case(MetaTypeDto::Number, "number")]
    #[case(MetaTypeDto::Boolean, "boolean")]
    #[case(MetaTypeDto::Date, "date")]
    #[case(MetaTypeDto::Timestamp, "timestamp")]
    #[case(MetaTypeDto::Amount, "amount")]
    #[case(MetaTypeDto::Account, "account")]
    fn type_serialises_to_its_label(#[case] ty: MetaTypeDto, #[case] expected: &str) {
        let json = serde_json::to_string(&ty).expect("serialize");
        assert_eq!(json, format!("\"{expected}\""));
        assert_eq!(ty.label(), expected);
        let back: MetaTypeDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(ty, back);
    }

    #[test]
    fn entry_defaults_to_unflagged() {
        let entry = MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned()));
        assert_eq!(entry.key, "payee");
        assert!(!entry.mismatched);
    }

    #[test]
    fn a_flagged_entry_carries_its_text() {
        let entry = MetaEntryDto::flagged("cleared", "sometime in May");
        assert!(entry.mismatched);
        assert_eq!(entry.key, "cleared");
        assert_eq!(
            entry.value,
            MetaValueDto::Text("sometime in May".to_owned()),
            "a flagged value is always the raw text that would not fit"
        );
    }

    #[test]
    fn key_def_round_trips_through_json() {
        let def = MetaKeyDefDto::new("invoice", MetaTypeDto::Number);
        let json = serde_json::to_string(&def).expect("serialize");
        let back: MetaKeyDefDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(def, back);
    }

    #[rstest]
    #[case("Payee", "payee")]
    #[case("INVOICE", "invoice")]
    #[case("due date", "due date")]
    #[case("é", "é")]
    fn normalise_lowercases_ascii_and_leaves_the_rest(#[case] raw: &str, #[case] expected: &str) {
        assert_eq!(normalise_meta_key(raw), expected);
    }

    #[rstest]
    #[case("payee", "payee")]
    #[case("Payee", "payee")]
    #[case("inVoice", "invoice")]
    #[case("due_date-2", "due_date-2")]
    fn validate_accepts_and_normalises(#[case] raw: &str, #[case] expected: &str) {
        assert_eq!(validate_meta_key(raw), Ok(expected.to_owned()));
    }

    #[rstest]
    #[case("", MetaKeyProblem::Empty)]
    #[case("1nvoice", MetaKeyProblem::LeadingChar { found: '1' })]
    #[case("_leading", MetaKeyProblem::LeadingChar { found: '_' })]
    #[case("é", MetaKeyProblem::LeadingChar { found: 'é' })]
    #[case("due date", MetaKeyProblem::InvalidChar { found: ' ' })]
    #[case("payee:name", MetaKeyProblem::InvalidChar { found: ':' })]
    fn validate_names_the_rule_that_was_broken(
        #[case] raw: &str,
        #[case] expected: MetaKeyProblem,
    ) {
        assert_eq!(validate_meta_key(raw), Err(expected));
    }

    #[test]
    fn validate_admits_the_byte_cap_and_rejects_one_past_it() {
        let at_cap = "a".repeat(META_KEY_MAX_BYTES);
        assert_eq!(validate_meta_key(&at_cap), Ok(at_cap.clone()));
        assert_eq!(
            validate_meta_key(&format!("{at_cap}a")),
            Err(MetaKeyProblem::TooLong {
                len: META_KEY_MAX_BYTES + 1
            })
        );
    }

    #[test]
    fn charset_is_judged_before_length() {
        let over_cap_with_a_space = format!("{} ", "a".repeat(META_KEY_MAX_BYTES));
        assert_eq!(
            validate_meta_key(&over_cap_with_a_space),
            Err(MetaKeyProblem::InvalidChar { found: ' ' }),
            "a key breaking both rules is reported the same way on both sides"
        );
    }

    #[cfg(feature = "models")]
    mod models {
        use bc_models::MetaEntry;
        use bc_models::MetaKey;
        use bc_models::MetaKeyDef;
        use bc_models::MetaType;
        use bc_models::MetaValue;
        use pretty_assertions::assert_eq;
        use rstest::rstest;

        use super::sample_values;
        use crate::BcError;
        use crate::META_KEY_MAX_BYTES;
        use crate::MetaEntryDto;
        use crate::MetaKeyDefDto;
        use crate::MetaKeyProblem;
        use crate::MetaTypeDto;
        use crate::MetaValueDto;
        use crate::validate_meta_key;

        #[test]
        fn every_value_round_trips_through_the_domain() {
            for dto in sample_values() {
                let domain = MetaValue::try_from(&dto).expect("convert to domain");
                assert_eq!(MetaValueDto::from(&domain), dto);
            }
        }

        #[rstest]
        #[case(MetaType::Text, MetaTypeDto::Text)]
        #[case(MetaType::Number, MetaTypeDto::Number)]
        #[case(MetaType::Boolean, MetaTypeDto::Boolean)]
        #[case(MetaType::Date, MetaTypeDto::Date)]
        #[case(MetaType::Timestamp, MetaTypeDto::Timestamp)]
        #[case(MetaType::Amount, MetaTypeDto::Amount)]
        #[case(MetaType::Account, MetaTypeDto::Account)]
        fn every_type_round_trips_through_the_domain(
            #[case] domain: MetaType,
            #[case] dto: MetaTypeDto,
        ) {
            assert_eq!(MetaTypeDto::from(domain), dto);
            assert_eq!(MetaType::from(dto), domain);
        }

        /// Every key the two validators are held to agree on.
        ///
        /// Covers each problem variant, the lowercase-first case, the byte cap
        /// in both directions, and keys breaking two rules at once, where only
        /// a shared check order gives a shared answer.
        fn candidate_keys() -> Vec<String> {
            let long = "a".repeat(META_KEY_MAX_BYTES);
            vec![
                "payee".to_owned(),
                "Payee".to_owned(),
                "INVOICE".to_owned(),
                "inVoice".to_owned(),
                "due_date-2".to_owned(),
                "a".to_owned(),
                String::new(),
                "1nvoice".to_owned(),
                "_leading".to_owned(),
                "-leading".to_owned(),
                "é".to_owned(),
                "due date".to_owned(),
                "due.date".to_owned(),
                "duée".to_owned(),
                "payee:name".to_owned(),
                long.clone(),
                format!("{long}a"),
                format!("{long} "),
                format!("{long}é"),
            ]
        }

        /// Restates a domain rejection as its mirror, so the two are comparable.
        ///
        /// A match rather than a string compare: the wording of the two error
        /// types is allowed to differ, and only the rule each names has to
        /// agree.
        fn mirror_of(err: &bc_models::MetaKeyError) -> MetaKeyProblem {
            match *err {
                bc_models::MetaKeyError::Empty => MetaKeyProblem::Empty,
                bc_models::MetaKeyError::LeadingChar { found } => {
                    MetaKeyProblem::LeadingChar { found }
                }
                bc_models::MetaKeyError::InvalidChar { found } => {
                    MetaKeyProblem::InvalidChar { found }
                }
                bc_models::MetaKeyError::TooLong { len } => MetaKeyProblem::TooLong { len },
                ref other => panic!("domain rejection {other:?} has no mirror in bc-ipc"),
            }
        }

        #[test]
        fn mirror_agrees_with_the_domain_validator() {
            for raw in candidate_keys() {
                let ours = validate_meta_key(&raw);
                let theirs = MetaKey::new(raw.clone());
                let expected = match theirs {
                    Ok(key) => Ok(key.as_str().to_owned()),
                    Err(ref err) => Err(mirror_of(err)),
                };
                assert_eq!(
                    ours, expected,
                    "the bc-ipc mirror and bc_models::MetaKey::new disagree on '{raw}'"
                );
            }
        }

        #[test]
        fn the_byte_cap_matches_the_domain() {
            let at_cap = "a".repeat(META_KEY_MAX_BYTES);
            MetaKey::new(at_cap.clone()).expect("the domain admits a key at the mirrored cap");
            let over = format!("{at_cap}a");
            assert_eq!(
                MetaKey::new(over.clone()).err().map(|e| mirror_of(&e)),
                Some(MetaKeyProblem::TooLong {
                    len: META_KEY_MAX_BYTES + 1
                }),
                "META_KEY_MAX_BYTES must be the domain's cap, not merely near it"
            );
        }

        #[rstest]
        #[case(MetaType::Text, MetaTypeDto::Text)]
        #[case(MetaType::Number, MetaTypeDto::Number)]
        #[case(MetaType::Boolean, MetaTypeDto::Boolean)]
        #[case(MetaType::Date, MetaTypeDto::Date)]
        #[case(MetaType::Timestamp, MetaTypeDto::Timestamp)]
        #[case(MetaType::Amount, MetaTypeDto::Amount)]
        #[case(MetaType::Account, MetaTypeDto::Account)]
        fn the_type_label_matches_the_domains_registry_name(
            #[case] domain: MetaType,
            #[case] dto: MetaTypeDto,
        ) {
            let registry_name = serde_json::to_string(&domain).expect("serialize");
            assert_eq!(
                format!("\"{}\"", dto.label()),
                registry_name,
                "the display label and the name stored in the registry must not drift apart"
            );
        }

        #[test]
        fn an_account_value_carries_the_bare_id() {
            let id = bc_models::AccountId::new();
            assert_eq!(
                MetaValueDto::from(&MetaValue::Account(id.clone())),
                MetaValueDto::Account(id.to_string()),
                "resolving the path is the frontend's, from the tree it holds"
            );
        }

        #[test]
        fn an_unparsable_account_id_is_rejected() {
            let err = MetaValue::try_from(&MetaValueDto::Account("Assets:Bank".to_owned()))
                .expect_err("an account path is not an id");
            let BcError::Validation(message) = err else {
                panic!("an unparsable account id must be a validation failure");
            };
            assert!(
                message.starts_with("invalid account id 'Assets:Bank': "),
                "the message names the offending id; the tail is the parser's, got: {message}"
            );
        }

        #[test]
        fn an_invalid_key_is_rejected() {
            let dto = MetaEntryDto::new("1nvoice", MetaValueDto::Boolean(true));
            let err = MetaEntry::try_from(&dto).expect_err("a key must start with a letter");
            assert_eq!(
                err,
                BcError::Validation(
                    "invalid metadata key '1nvoice': metadata key must start with a letter, \
                     found '1'"
                        .to_owned()
                )
            );
        }

        #[test]
        fn a_flagged_entry_carries_its_flag_out() {
            let entry =
                MetaEntry::mismatch(MetaKey::new("invoice").expect("valid key"), "not-a-number");
            let dto = MetaEntryDto::from(&entry);
            assert!(dto.mismatched);
            assert_eq!(dto.value, MetaValueDto::Text("not-a-number".to_owned()));
        }

        #[test]
        fn a_flagged_dto_arrives_unflagged() {
            let dto = MetaEntryDto {
                key: "invoice".to_owned(),
                value: MetaValueDto::Text("not-a-number".to_owned()),
                mismatched: true,
            };
            let entry = MetaEntry::try_from(&dto).expect("convert to domain");
            assert!(
                !entry.mismatched(),
                "the write path derives the flag; an incoming DTO does not get to assert it"
            );
        }

        #[test]
        fn a_key_definition_drops_its_timestamp() {
            let def = MetaKeyDef::builder()
                .key(MetaKey::new("invoice").expect("valid key"))
                .ty(MetaType::Number)
                .created_at(jiff::Timestamp::from_second(1_700_000_000).expect("valid timestamp"))
                .build();
            assert_eq!(
                MetaKeyDefDto::from(&def),
                MetaKeyDefDto::new("invoice", MetaTypeDto::Number)
            );
        }
    }
}
