//! Shared import types for the BorrowChecker import pipeline.
//!
//! These types form the contract between `bc-core` and the format-specific parser
//! crates (`bc-format-csv`, `bc-format-ledger`, `bc-format-beancount`,
//! `bc-format-ofx`).  Each format crate produces [`RawTransaction`] values and
//! implements the [`Importer`] trait; the core engine drives the import via
//! [`Config`].

pub(crate) mod account_path;
pub(crate) mod batch;
pub(crate) mod profile;
pub(crate) mod registry;

use bc_models::Amount;
use jiff::civil::Date;

/// A single posting leg of a raw transaction, prior to account-id resolution.
///
/// Format-specific parser crates construct these directly from raw input.
/// The core engine then resolves the account path to an [`bc_models::AccountId`]
/// and persists the result.
#[non_exhaustive]
#[derive(bon::Builder, Debug, Clone, PartialEq)]
pub struct RawPosting {
    /// Account path this leg credits/debits, e.g. `"Assets:Bank:Checking"`.
    #[builder(into)]
    pub account: String,
    /// Leg amount. `None` means elided — the residual that balances the transaction.
    pub amount: Option<Amount>,
    /// Per-account running balance after this leg, if the source reports it.
    pub balance: Option<Amount>,
    /// Optional free-text note for this leg.
    #[builder(into)]
    pub note: Option<String>,
    /// Tag names applied to this leg.
    #[builder(default)]
    pub tags: Vec<String>,
}

/// Where a [`RawTransaction`] came from, for diagnostics.
///
/// A source is not always a file — a plugin may call an API or read a database —
/// so `display` carries free-form human-facing text and `uri` carries an
/// optional addressable form. Import diagnostics print `display` verbatim.
#[non_exhaustive]
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
pub struct SourceLocation {
    /// Human-readable location, e.g. `"statements/june.csv row 14"`.
    #[builder(into)]
    pub display: String,
    /// Optional machine-addressable form, e.g. `"file:///june.csv#row=14"`.
    #[builder(into)]
    pub uri: Option<String>,
}

/// A parsed transaction prior to account binding.
///
/// Format-specific parser crates construct these directly from raw input.
/// The core engine then resolves accounts and persists the results.
#[non_exhaustive]
#[derive(bon::Builder, Debug, Clone, PartialEq)]
pub struct RawTransaction {
    /// The transaction date.
    pub date: Date,
    /// The payee or merchant name, if available.
    #[builder(into)]
    pub payee: Option<String>,
    /// A free-text description or memo for the transaction.
    #[builder(into)]
    pub description: String,
    /// Optional user annotation, distinct from `description`.
    #[builder(into)]
    pub note: Option<String>,
    /// An institution-provided reference or check number, if available.
    #[builder(into)]
    pub reference: Option<String>,
    /// Transaction-level tag names (e.g. Beancount `#josh`).
    #[builder(default)]
    pub tags: Vec<String>,
    /// Free-form labelled dates (e.g. `("cleared", …)`).
    #[builder(default)]
    pub extra_dates: Vec<(String, Date)>,
    /// Where this transaction came from, if the importer reported it.
    pub source_location: Option<SourceLocation>,
    /// One or more posting legs. Single-account importers emit exactly one.
    ///
    /// Required: the WIT→core boundary rejects a transaction with no legs, and
    /// every constructed [`RawTransaction`] must carry at least one posting.
    pub postings: Vec<RawPosting>,
}

/// Opaque JSON configuration blob passed to an [`Importer`].
///
/// Format crates define their own typed configuration structs and use
/// [`Config::from_typed`] / [`Config::into_typed`] to convert.
/// The core engine stores and retrieves the raw [`serde_json::Value`] without
/// needing to know the format-specific schema.
///
/// Re-exported from the crate root as [`crate::ImportConfig`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct Config(serde_json::Value);

impl Config {
    /// Serialises a typed configuration value into a [`Config`].
    ///
    /// # Arguments
    ///
    /// * `value` - Any value that implements [`serde::Serialize`].
    ///
    /// # Returns
    ///
    /// A [`Config`] wrapping the serialised JSON representation.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if serialisation fails.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::ImportConfig as Config;
    /// use serde::Serialize;
    ///
    /// #[derive(Serialize)]
    /// struct MyCfg { delimiter: char }
    ///
    /// let cfg = Config::from_typed(&MyCfg { delimiter: ',' }).expect("serialisation is infallible for this type");
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
    /// use bc_core::ImportConfig as Config;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Debug, PartialEq, Serialize, Deserialize)]
    /// struct MyCfg { delimiter: char }
    ///
    /// let original = MyCfg { delimiter: ',' };
    /// let cfg = Config::from_typed(&original).expect("serialisation is infallible for this type");
    /// let back: MyCfg = cfg.into_typed().expect("deserialisation should succeed");
    /// assert_eq!(back, original);
    /// ```
    #[inline]
    pub fn into_typed<T: serde::de::DeserializeOwned>(self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.0)
    }

    /// Constructs a [`Config`] directly from a [`serde_json::Value`].
    ///
    /// # Arguments
    ///
    /// * `value` - The JSON value to wrap.
    ///
    /// # Returns
    ///
    /// A [`Config`] wrapping the given value.
    #[must_use]
    #[inline]
    pub fn from_value(value: serde_json::Value) -> Self {
        Self(value)
    }

    /// Deserialises this config into a typed value without consuming it.
    ///
    /// Unlike [`into_typed`](Self::into_typed), this method borrows `self` and
    /// clones the inner [`serde_json::Value`] to perform the conversion.
    /// Prefer this over `config.clone().into_typed()` at call sites that need
    /// to retain the original config.
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
    /// use bc_core::ImportConfig as Config;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Debug, PartialEq, Serialize, Deserialize)]
    /// struct MyCfg { delimiter: char }
    ///
    /// let original = MyCfg { delimiter: ',' };
    /// let cfg = Config::from_typed(&original).expect("serialisation should succeed");
    /// let back: MyCfg = cfg.as_typed().expect("deserialisation should succeed");
    /// assert_eq!(back, original);
    /// ```
    #[inline]
    pub fn as_typed<T: serde::de::DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_value(self.0.clone())
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

impl Default for Config {
    /// Returns a [`Config`] wrapping an empty JSON object (`{}`).
    #[inline]
    fn default() -> Self {
        Self(serde_json::Value::Object(serde_json::Map::new()))
    }
}

/// Errors produced during an import operation.
///
/// Re-exported from the crate root as [`crate::ImportError`].
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
#[expect(
    clippy::error_impl_error,
    reason = "re-exported as ImportError; the name is unambiguous at the crate root"
)]
pub enum Error {
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
/// ## Compatibility invariant
///
/// This trait is intentionally parallel to [`bc_sdk::Importer`] in method
/// shape. If you change either trait's method signatures (names, parameter
/// types, return types), you must update the other and update the translation
/// layer in `bc_plugins::translate`. The two traits use different types
/// (`bc_core` domain types vs. `bc_sdk` WIT wire types) and cannot share a
/// definition; `bc_plugins::translate` is the authoritative bridge.
///
/// # Example
///
/// ```rust,ignore
/// struct CsvImporter;
///
/// impl bc_core::Importer for CsvImporter {
///     fn name(&self) -> &str { "csv" }
///
///     fn import(
///         &self,
///         config: &bc_core::ImportConfig,
///     ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
///         todo!()
///     }
///
///     fn validate(&self, config: &bc_core::ImportConfig) -> Result<(), bc_core::ImportError> {
///         Ok(())
///     }
/// }
/// ```
pub trait Importer: Send + Sync + 'static {
    /// A short, stable identifier for this importer (e.g. `"csv"`, `"ofx"`).
    fn name(&self) -> &str;

    /// Reads and parses this importer's configured sources.
    ///
    /// The importer resolves its own file paths from `config` and reads them
    /// from the host-preopened documents root.
    ///
    /// # Arguments
    ///
    /// * `config` - Format-specific configuration.
    ///
    /// # Returns
    ///
    /// A list of parsed transactions in source order.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] on configuration, I/O, parse, or field errors.
    fn import(&self, config: &Config) -> Result<Vec<RawTransaction>, Error>;

    /// Checks a configuration for coherence without reading any files.
    ///
    /// Importers whose configuration admits combinations that are syntactically
    /// valid but meaningless should reject them here, so a profile can be
    /// checked when it is saved rather than when an import runs. An importer
    /// with nothing to check yet should still implement this explicitly,
    /// returning `Ok(())` — there is no default, so a delegate wrapper that
    /// forgets to forward this method fails to compile rather than silently
    /// skipping validation.
    ///
    /// Implementations must not perform I/O.
    ///
    /// # Arguments
    ///
    /// * `config` - Format-specific configuration.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the configuration is coherent.
    ///
    /// # Errors
    ///
    /// Returns [`Error`] describing why the configuration is incoherent.
    fn validate(&self, config: &Config) -> Result<(), Error>;
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

    /// Helper that constructs a minimal [`RawTransaction`] with a single posting.
    fn make_raw_transaction() -> RawTransaction {
        RawTransaction::builder()
            .date(date(2024, 3, 15))
            .payee("Coffee Shop")
            .description("Morning coffee")
            .reference("REF001")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(dec!(42.50), CommodityCode::new("USD"))))
                    .maybe_balance(Some(Amount::new(dec!(1_000.00), CommodityCode::new("USD"))))
                    .build(),
            ])
            .build()
    }

    #[test]
    fn raw_transaction_fields_are_accessible() {
        let tx = make_raw_transaction();

        assert_eq!(tx.date, date(2024, 3, 15));
        assert_eq!(tx.postings.len(), 1);
        let posting = tx.postings.first().expect("one posting");
        assert_eq!(
            posting.amount,
            Some(Amount::new(dec!(42.50), CommodityCode::new("USD")))
        );
        assert_eq!(
            posting.balance,
            Some(Amount::new(dec!(1_000.00), CommodityCode::new("USD")))
        );
        assert_eq!(tx.payee.as_deref(), Some("Coffee Shop"));
        assert_eq!(tx.description, "Morning coffee");
        assert_eq!(tx.reference.as_deref(), Some("REF001"));
    }

    #[test]
    fn raw_transaction_optional_fields_can_be_none() {
        let tx = RawTransaction::builder()
            .date(date(2024, 1, 1))
            .description("Unknown")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(dec!(10.00), CommodityCode::new("EUR"))))
                    .build(),
            ])
            .build();

        assert!(tx.payee.is_none());
        assert!(tx.reference.is_none());
        assert!(tx.postings.first().expect("one posting").balance.is_none());
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

        let cfg =
            Config::from_typed(&original).expect("serialisation of TestConfig should succeed");
        let back: TestConfig = cfg.into_typed().expect("deserialisation should succeed");

        assert_eq!(back, original);
    }

    #[test]
    fn import_config_default_is_empty_object() {
        let cfg = Config::default();
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
        let cfg = Config::from_typed(&original).expect("serialisation should succeed");
        let value = cfg.as_value();
        assert_eq!(
            value.get("delimiter").and_then(serde_json::Value::as_str),
            Some(",")
        );
    }

    #[test]
    fn import_error_invalid_config_displays() {
        // Deserialising a JSON string as TestConfig should fail.
        let cfg = Config(serde_json::Value::String("bad".to_owned()));
        let err: Result<TestConfig, _> = cfg.into_typed();
        let import_err = Error::InvalidConfig(err.expect_err("should fail"));
        assert!(!import_err.to_string().is_empty());
    }

    #[test]
    fn import_error_parse_displays() {
        let err = Error::Parse("unexpected token".to_owned());
        assert!(err.to_string().contains("unexpected token"));
    }

    #[test]
    fn import_error_missing_field_displays() {
        let err = Error::MissingField("date".to_owned());
        assert!(err.to_string().contains("date"));
    }

    #[test]
    fn import_error_bad_value_displays() {
        let err = Error::BadValue {
            field: "amount".to_owned(),
            detail: "must be positive".to_owned(),
        };
        assert!(err.to_string().contains("amount"));
        assert!(err.to_string().contains("must be positive"));
    }
}
