//! WIT wire types for the BorrowChecker plugin SDK.
//!
//! These types are a simplified, WASM-portable representation of the domain
//! types in `bc_core` / `bc_models`. They are intentionally different: dates
//! are plain `{year, month, day}` integers; amounts use minor units rather
//! than `rust_decimal::Decimal`. The conversion layer in `bc_plugins::translate`
//! bridges between these wire types and the host's domain types.
//!
//! If you change a type here, verify that `bc_plugins::translate` still compiles
//! and produces correct values.

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

    /// Creates a new [`Date`], returning an error if the values are out of range.
    ///
    /// Validates that `month` is 1–12 and `day` is valid for the given month and year
    /// (including leap-year handling for February).
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
    ///
    /// # Errors
    ///
    /// Returns a [`String`] error message if `month` or `day` is out of range.
    #[inline]
    pub fn try_new(year: i32, month: u8, day: u8) -> Result<Self, String> {
        if month == 0 || month > 12 {
            return Err(format!("month {month} is out of range (must be 1–12)"));
        }
        let max_day = days_in_month(year, month);
        if day == 0 || day > max_day {
            return Err(format!(
                "day {day} is out of range for {year}-{month:02} (max {max_day})"
            ));
        }
        Ok(Self { year, month, day })
    }
}

/// Returns the number of days in the given month for the given year.
#[inline]
#[expect(
    clippy::integer_division_remainder_used,
    reason = "Gregorian leap-year rule requires modulo arithmetic"
)]
fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 0,
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

// These `From` impls convert bc_sdk ergonomic types → WIT-generated types.
// They are used by the #[importer] proc-macro generated code.
// Bring generated types into scope to avoid absolute paths (clippy::absolute_paths).
use crate::__bindings::borrow_checker::sdk::types::Amount as WitAmount;
use crate::__bindings::borrow_checker::sdk::types::Date as WitDate;
use crate::__bindings::exports::borrow_checker::sdk::importer::ImportError as WitImportError;
use crate::__bindings::exports::borrow_checker::sdk::importer::RawTransaction as WitRawTransaction;

#[doc(hidden)]
impl From<RawTransaction> for WitRawTransaction {
    #[inline]
    fn from(t: RawTransaction) -> Self {
        Self {
            date: WitDate {
                year: t.date.year,
                month: t.date.month,
                day: t.date.day,
            },
            amount: t.amount.into(),
            balance: t.balance.map(::core::convert::Into::into),
            payee: t.payee,
            description: t.description,
            reference: t.reference,
        }
    }
}

#[doc(hidden)]
impl From<Amount> for WitAmount {
    #[inline]
    fn from(a: Amount) -> Self {
        Self {
            minor_units: a.minor_units,
            currency: a.currency,
            scale: a.scale,
        }
    }
}

#[doc(hidden)]
impl From<ImportError> for WitImportError {
    #[inline]
    fn from(e: ImportError) -> Self {
        match e {
            ImportError::InvalidConfig(s) => Self::InvalidConfig(s),
            ImportError::Parse(s) => Self::Parse(s),
            ImportError::MissingField(s) => Self::MissingField(s),
            ImportError::BadValue { field, detail } => Self::BadValue(format!("{field}: {detail}")),
        }
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
    fn date_try_new_accepts_valid_dates() {
        Date::try_new(2025, 1, 1).expect("2025-01-01 is valid");
        Date::try_new(2025, 12, 31).expect("2025-12-31 is valid");
        Date::try_new(2024, 2, 29).expect("2024-02-29 is valid (leap year)");
        Date::try_new(2025, 2, 28).expect("2025-02-28 is valid");
    }

    #[test]
    fn date_try_new_rejects_invalid_dates() {
        Date::try_new(2025, 0, 15).expect_err("month 0 is invalid");
        Date::try_new(2025, 13, 15).expect_err("month 13 is invalid");
        Date::try_new(2025, 1, 0).expect_err("day 0 is invalid");
        Date::try_new(2025, 1, 32).expect_err("day 32 is invalid");
        Date::try_new(2025, 4, 31).expect_err("April has 30 days");
        Date::try_new(2025, 2, 29).expect_err("2025 is not a leap year");
        Date::try_new(2025, 2, 31).expect_err("February never has 31 days");
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
