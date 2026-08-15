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

    use super::MetaEntryDto;
    use super::MetaKeyDefDto;
    use super::MetaTypeDto;
    use super::MetaValueDto;
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
    fn key_def_round_trips_through_json() {
        let def = MetaKeyDefDto::new("invoice", MetaTypeDto::Number);
        let json = serde_json::to_string(&def).expect("serialize");
        let back: MetaKeyDefDto = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(def, back);
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
        use crate::MetaEntryDto;
        use crate::MetaKeyDefDto;
        use crate::MetaTypeDto;
        use crate::MetaValueDto;

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
