//! Ergonomic SDK types that mirror the WIT interface.
//!
//! These types are the public API for plugin authors. They are distinct from
//! `bc-models` types — `bc-sdk` has no workspace dependencies.

use serde::de::DeserializeOwned;

/// A calendar date.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Date {
    /// Full year, e.g. 2025.
    pub year: i32,
    /// Month 1–12.
    pub month: u8,
    /// Day 1–31.
    pub day: u8,
}

impl Date {
    /// Creates a new [`Date`].
    ///
    /// # Arguments
    ///
    /// * `year` - Full year, e.g. `2025`.
    /// * `month` - Month 1–12.
    /// * `day` - Day 1–31.
    ///
    /// # Returns
    ///
    /// A new [`Date`] with the given fields.
    #[inline]
    #[must_use]
    pub fn new(year: i32, month: u8, day: u8) -> Self {
        Self { year, month, day }
    }
}

/// A monetary amount represented in minor units to avoid floating-point imprecision.
///
/// Example: AUD 10.50 → `minor_units = 1050`, `currency = "AUD"`, `scale = 2`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
    /// Minor units (e.g. cents). Negative for debits.
    pub minor_units: i64,
    /// ISO 4217 currency code, e.g. `"AUD"`.
    pub currency: String,
    /// Number of decimal places, e.g. `2` for AUD cents.
    pub scale: u8,
}

impl Amount {
    /// Creates a new [`Amount`].
    ///
    /// # Arguments
    ///
    /// * `minor_units` - Minor units (e.g. cents). Negative for debits.
    /// * `currency` - ISO 4217 currency code, e.g. `"AUD"`.
    /// * `scale` - Number of decimal places, e.g. `2` for AUD cents.
    ///
    /// # Returns
    ///
    /// A new [`Amount`] with the given fields.
    #[inline]
    #[must_use]
    pub fn new(minor_units: i64, currency: impl Into<String>, scale: u8) -> Self {
        Self {
            minor_units,
            currency: currency.into(),
            scale,
        }
    }
}

/// A parsed transaction prior to account binding.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct RawTransaction {
    /// The transaction date.
    pub date: Date,
    /// The transaction amount.
    pub amount: Amount,
    /// The running balance after this transaction, if available.
    pub balance: Option<Amount>,
    /// The payee or merchant name, if available.
    pub payee: Option<String>,
    /// A free-text description or memo.
    pub description: String,
    /// An institution-provided reference, if available.
    pub reference: Option<String>,
}

impl RawTransaction {
    /// Creates a new [`RawTransaction`].
    ///
    /// # Arguments
    ///
    /// * `date` - The transaction date.
    /// * `amount` - The transaction amount.
    /// * `balance` - The running balance after this transaction, if available.
    /// * `payee` - The payee or merchant name, if available.
    /// * `description` - A free-text description or memo.
    /// * `reference` - An institution-provided reference, if available.
    ///
    /// # Returns
    ///
    /// A new [`RawTransaction`] with the given fields.
    #[inline]
    #[must_use]
    pub fn new(
        date: Date,
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

/// Opaque JSON configuration blob passed to an importer from the import profile.
///
/// Use [`ImportConfig::as_typed`] to deserialize into a format-specific struct.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ImportConfig(String);

impl ImportConfig {
    /// Constructs an [`ImportConfig`] from a raw JSON string.
    ///
    /// This is called by the generated WASM export glue — plugin authors
    /// should not need to call this directly.
    ///
    /// # Arguments
    ///
    /// * `s` - A raw JSON string.
    ///
    /// # Returns
    ///
    /// A new [`ImportConfig`] wrapping the given string.
    #[inline]
    #[must_use]
    pub fn from_json_string(s: String) -> Self {
        Self(s)
    }

    /// Deserialises this config into a typed value.
    ///
    /// # Errors
    ///
    /// Returns [`serde_json::Error`] if the stored JSON does not match `T`'s schema.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_sdk::ImportConfig;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct MyCfg { delimiter: char }
    ///
    /// let cfg = ImportConfig::from_json_string(r#"{"delimiter":","}"#.to_owned());
    /// let typed: MyCfg = cfg.as_typed().expect("valid config");
    /// assert_eq!(typed.delimiter, ',');
    /// ```
    #[inline]
    pub fn as_typed<T: DeserializeOwned>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.0)
    }

    /// Returns the raw JSON string.
    ///
    /// # Returns
    ///
    /// The raw JSON string stored in this config.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ImportConfig {
    /// Returns an [`ImportConfig`] wrapping an empty JSON object (`{}`).
    #[inline]
    fn default() -> Self {
        Self("{}".to_owned())
    }
}

/// Errors produced during an import operation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The supplied configuration could not be deserialised.
    #[error("invalid import configuration: {0}")]
    InvalidConfig(String),
    /// A parse error with a human-readable message.
    #[error("parse error: {0}")]
    Parse(String),
    /// A required field was absent in the input.
    #[error("missing required field: {0}")]
    MissingField(String),
    /// A field contained an unexpected or out-of-range value.
    #[error("bad value for field '{field}': {detail}")]
    BadValue {
        /// The name of the field.
        field: String,
        /// A human-readable explanation.
        detail: String,
    },
}

impl From<serde_json::Error> for ImportError {
    #[inline]
    fn from(e: serde_json::Error) -> Self {
        Self::InvalidConfig(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use serde::Deserialize;

    use super::*;

    #[test]
    fn import_config_as_typed_round_trips() {
        #[derive(Debug, PartialEq, Deserialize)]
        struct Cfg {
            delimiter: char,
        }
        let cfg = ImportConfig::from_json_string(r#"{"delimiter":","}"#.to_owned());
        let typed: Cfg = cfg.as_typed().expect("valid config");
        assert_eq!(typed.delimiter, ',');
    }

    #[test]
    fn import_config_as_typed_errors_on_wrong_schema() {
        let cfg = ImportConfig::from_json_string("not-json".to_owned());
        let result: Result<serde_json::Value, _> = cfg.as_typed();
        result.expect_err("invalid JSON should fail to parse");
    }

    #[test]
    fn import_config_default_is_empty_object() {
        let cfg = ImportConfig::default();
        assert_eq!(cfg.as_str(), "{}");
    }

    #[test]
    fn amount_new_stores_fields() {
        let a = Amount::new(1050_i64, "AUD", 2_u8);
        assert_eq!(a.minor_units, 1050);
        assert_eq!(a.currency, "AUD");
        assert_eq!(a.scale, 2);
    }

    #[test]
    fn date_new_stores_fields() {
        let d = Date::new(2025_i32, 3_u8, 15_u8);
        assert_eq!(d.year, 2025_i32);
        assert_eq!(d.month, 3_u8);
        assert_eq!(d.day, 15_u8);
    }

    #[test]
    fn raw_transaction_new_stores_fields() {
        let date = Date::new(2025_i32, 1_u8, 15_u8);
        let amount = Amount::new(1050_i64, "AUD", 2_u8);
        let tx = RawTransaction::new(
            date.clone(),
            amount.clone(),
            None,
            Some("Payee".to_owned()),
            "Description".to_owned(),
            None,
        );
        assert_eq!(tx.date, date);
        assert_eq!(tx.amount, amount);
        assert_eq!(tx.balance, None);
        assert_eq!(tx.payee.as_deref(), Some("Payee"));
        assert_eq!(tx.description, "Description");
        assert_eq!(tx.reference, None);
    }
}
