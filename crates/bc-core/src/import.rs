//! Shared import types for the BorrowChecker import pipeline.
//!
//! These types form the contract between `bc-core` and the format-specific parser
//! crates (`bc-format-csv`, `bc-format-ledger`, `bc-format-beancount`,
//! `bc-format-ofx`).  Each format crate produces [`RawTransaction`] values and
//! implements the [`Importer`] trait; the core engine drives the import via
//! [`ImportConfig`].

use bc_models::Amount;
use jiff::civil::Date;

/// A parsed transaction prior to account binding.
///
/// Format-specific parser crates construct these directly from raw input.
/// The core engine then resolves accounts and persists the results.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct RawTransaction {
    /// The transaction date.
    pub date: Date,
    /// The transaction amount (quantity + commodity).
    pub amount: Amount,
    /// The running balance after this transaction, if available.
    pub balance: Option<Amount>,
    /// The payee or merchant name, if available.
    pub payee: Option<String>,
    /// A free-text description or memo for the transaction.
    pub description: String,
    /// An institution-provided reference or check number, if available.
    pub reference: Option<String>,
}

impl RawTransaction {
    /// Constructs a new [`RawTransaction`].
    ///
    /// # Arguments
    ///
    /// * `date` - The transaction date.
    /// * `amount` - The transaction amount (quantity + commodity).
    /// * `balance` - The running balance after this transaction, if available.
    /// * `payee` - The payee or merchant name, if available.
    /// * `description` - A free-text description or memo for the transaction.
    /// * `reference` - An institution-provided reference or check number, if available.
    ///
    /// # Returns
    ///
    /// A new [`RawTransaction`] with the provided fields.
    #[inline]
    #[must_use]
    pub fn new(
        date: jiff::civil::Date,
        amount: Amount,
        balance: Option<Amount>,
        payee: Option<String>,
        description: String,
        reference: Option<String>,
    ) -> Self {
        Self {
            date,
            amount,
            balance,
            payee,
            description,
            reference,
        }
    }
}

/// Opaque JSON configuration blob passed to an [`Importer`].
///
/// Format crates define their own typed configuration structs and use
/// [`ImportConfig::from_typed`] / [`ImportConfig::into_typed`] to convert.
/// The core engine stores and retrieves the raw [`serde_json::Value`] without
/// needing to know the format-specific schema.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::module_name_repetitions,
    reason = "types are exported at the crate root as ImportConfig / ImportError; the module-prefixed names are intentional for API clarity"
)]
pub struct ImportConfig(serde_json::Value);

impl ImportConfig {
    /// Serialises a typed configuration value into an [`ImportConfig`].
    ///
    /// # Arguments
    ///
    /// * `value` - Any value that implements [`serde::Serialize`].
    ///
    /// # Returns
    ///
    /// An [`ImportConfig`] wrapping the serialised JSON representation.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if serialisation fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::ImportConfig;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyCfg { delimiter: char }
    ///
    /// let cfg = ImportConfig::from_typed(&MyCfg { delimiter: ',' }).expect("serialisation is infallible for this type");
    /// ```
    #[inline]
    pub fn from_typed<T: serde::Serialize>(value: &T) -> Result<Self, serde_json::Error> {
        let v = serde_json::to_value(value)?;
        Ok(Self(v))
    }

    /// Deserialises this config into a typed value.
    ///
    /// # Returns
    ///
    /// The deserialised value of type `T`.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if the stored JSON does not match `T`'s schema.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::ImportConfig;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Debug, PartialEq, Serialize, Deserialize)]
    /// struct MyCfg { delimiter: char }
    ///
    /// let original = MyCfg { delimiter: ',' };
    /// let cfg = ImportConfig::from_typed(&original).expect("serialisation is infallible for this type");
    /// let back: MyCfg = cfg.into_typed().expect("deserialisation should succeed");
    /// assert_eq!(back, original);
    /// ```
    #[inline]
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.0)
    }

    /// Returns a reference to the underlying [`serde_json::Value`].
    ///
    /// # Returns
    ///
    /// A reference to the raw JSON value.
    #[must_use]
    #[inline]
    pub fn as_value(&self) -> &serde_json::Value {
        &self.0
    }
}

impl Default for ImportConfig {
    /// Returns an [`ImportConfig`] wrapping an empty JSON object (`{}`).
    #[inline]
    fn default() -> Self {
        Self(serde_json::Value::Object(serde_json::Map::new()))
    }
}

/// Errors produced during an import operation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "types are exported at the crate root as ImportConfig / ImportError; the module-prefixed names are intentional for API clarity"
)]
pub enum ImportError {
    /// The supplied configuration could not be deserialised.
    #[error("invalid import configuration: {0}")]
    InvalidConfig(#[from] serde_json::Error),
    /// A parse error with a human-readable message.
    #[error("parse error: {0}")]
    Parse(String),
    /// A required field was absent in the input.
    #[error("missing required field: {0}")]
    MissingField(String),
    /// A field contained an unexpected or out-of-range value.
    #[error("bad value for field '{field}': {detail}")]
    BadValue {
        /// The name of the field that contained the bad value.
        field: String,
        /// A human-readable explanation of why the value was rejected.
        detail: String,
    },
}

/// An object-safe trait implemented by every format-specific importer.
///
/// Implementors are expected to be `Send + Sync + 'static` so they can be
/// stored in `Arc<dyn Importer>` and used across async tasks.
///
/// # Example
///
/// ```rust,ignore
/// struct CsvImporter;
///
/// impl bc_core::Importer for CsvImporter {
///     fn name(&self) -> &str { "csv" }
///
///     fn detect(&self, bytes: &[u8]) -> bool {
///         // heuristic: first non-whitespace byte is ASCII text
///         bytes.iter().any(|b| b.is_ascii_graphic())
///     }
///
///     fn import(
///         &self,
///         bytes: &[u8],
///         config: &bc_core::ImportConfig,
///     ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
///         todo!()
///     }
/// }
/// ```
pub trait Importer: Send + Sync + 'static {
    /// A short, stable identifier for this importer (e.g. `"csv"`, `"ofx"`).
    fn name(&self) -> &str;

    /// Returns `true` if `bytes` look like input this importer can handle.
    ///
    /// Implementations should be fast and non-panicking; they receive the raw
    /// file bytes and return a best-guess answer without consuming the data.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    #[must_use]
    fn detect(&self, bytes: &[u8]) -> bool;

    /// Parses `bytes` into a list of [`RawTransaction`] values.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to parse.
    /// * `config` - Format-specific configuration.
    ///
    /// # Returns
    ///
    /// A list of parsed transactions in the order they appear in the input.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError`] on configuration, parse, or field errors.
    fn import(
        &self,
        bytes: &[u8],
        config: &ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError>;
}

#[cfg(test)]
mod tests {
    use bc_models::CommodityCode;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;
    use serde::Deserialize;
    use serde::Serialize;

    use super::*;

    /// Helper that constructs a minimal [`RawTransaction`].
    fn make_raw_transaction() -> RawTransaction {
        RawTransaction {
            date: date(2024, 3, 15),
            amount: Amount::new(dec!(42.50), CommodityCode::new("USD")),
            balance: Some(Amount::new(dec!(1_000.00), CommodityCode::new("USD"))),
            payee: Some("Coffee Shop".to_owned()),
            description: "Morning coffee".to_owned(),
            reference: Some("REF001".to_owned()),
        }
    }

    #[test]
    fn raw_transaction_fields_are_accessible() {
        let tx = make_raw_transaction();

        assert_eq!(tx.date, date(2024, 3, 15));
        assert_eq!(
            tx.amount,
            Amount::new(dec!(42.50), CommodityCode::new("USD"))
        );
        assert_eq!(
            tx.balance,
            Some(Amount::new(dec!(1_000.00), CommodityCode::new("USD")))
        );
        assert_eq!(tx.payee.as_deref(), Some("Coffee Shop"));
        assert_eq!(tx.description, "Morning coffee");
        assert_eq!(tx.reference.as_deref(), Some("REF001"));
    }

    #[test]
    fn raw_transaction_optional_fields_can_be_none() {
        let tx = RawTransaction {
            date: date(2024, 1, 1),
            amount: Amount::new(dec!(10.00), CommodityCode::new("EUR")),
            balance: None,
            payee: None,
            description: "Unknown".to_owned(),
            reference: None,
        };

        assert!(tx.balance.is_none());
        assert!(tx.payee.is_none());
        assert!(tx.reference.is_none());
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct TestConfig {
        delimiter: char,
        skip_rows: u32,
    }

    #[test]
    fn import_config_round_trips_through_typed() {
        let original = TestConfig {
            delimiter: ';',
            skip_rows: 2,
        };

        let cfg = ImportConfig::from_typed(&original)
            .expect("serialisation of TestConfig should succeed");
        let back: TestConfig = cfg.into_typed().expect("deserialisation should succeed");

        assert_eq!(back, original);
    }

    #[test]
    fn import_config_default_is_empty_object() {
        let cfg = ImportConfig::default();
        assert_eq!(
            cfg.as_value(),
            &serde_json::Value::Object(serde_json::Map::default())
        );
    }

    #[test]
    fn import_config_as_value_returns_inner_json() {
        let original = TestConfig {
            delimiter: ',',
            skip_rows: 0,
        };
        let cfg = ImportConfig::from_typed(&original).expect("serialisation should succeed");
        let value = cfg.as_value();
        assert_eq!(
            value.get("delimiter").and_then(serde_json::Value::as_str),
            Some(",")
        );
    }

    #[test]
    fn import_error_invalid_config_displays() {
        // Deserialising a JSON string as TestConfig should fail.
        let cfg = ImportConfig(serde_json::Value::String("bad".to_owned()));
        let err: Result<TestConfig, _> = cfg.into_typed();
        let import_err = ImportError::InvalidConfig(err.expect_err("should fail"));
        assert!(!import_err.to_string().is_empty());
    }

    #[test]
    fn import_error_parse_displays() {
        let err = ImportError::Parse("unexpected token".to_owned());
        assert!(err.to_string().contains("unexpected token"));
    }

    #[test]
    fn import_error_missing_field_displays() {
        let err = ImportError::MissingField("date".to_owned());
        assert!(err.to_string().contains("date"));
    }

    #[test]
    fn import_error_bad_value_displays() {
        let err = ImportError::BadValue {
            field: "amount".to_owned(),
            detail: "must be positive".to_owned(),
        };
        assert!(err.to_string().contains("amount"));
        assert!(err.to_string().contains("must be positive"));
    }
}
