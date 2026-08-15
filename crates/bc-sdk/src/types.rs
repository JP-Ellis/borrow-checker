//! WIT wire types for the BorrowChecker plugin SDK.
//!
//! These types are a simplified, WASM-portable representation of the domain
//! types in `bc_core` / `bc_models`. They are intentionally different: dates
//! are plain `{year, month, day}` integers; amounts carry their commodity as
//! a source-supplied string rather than a validated commodity code. The
//! conversion layer in `bc_plugins::translate` bridges between these wire
//! types and the host's domain types.
//!
//! If you change a type here, verify that `bc_plugins::translate` still compiles
//! and produces correct values.

use rust_decimal::Decimal;
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

/// A monetary amount, carried as an exact decimal rather than minor units.
///
/// Example: AUD 10.50 → `value = dec!(10.50)`, `commodity = "AUD"`.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amount {
    /// The exact decimal value. Negative for debits.
    pub value: Decimal,
    /// Commodity code as the source names it, e.g. `"AUD"`, `"BTC"`.
    pub commodity: String,
}

impl Amount {
    /// Creates a new [`Amount`].
    ///
    /// # Arguments
    ///
    /// * `value` - The exact decimal value. Negative for debits.
    /// * `commodity` - Commodity code as the source names it, e.g. `"AUD"`.
    ///
    /// # Returns
    ///
    /// A new [`Amount`] with the given fields.
    #[inline]
    #[must_use]
    pub fn new(value: Decimal, commodity: impl Into<String>) -> Self {
        Self {
            value,
            commodity: commodity.into(),
        }
    }
}

/// A typed metadata value an importer states.
///
/// The host decides what a key's type actually is. A value whose type differs
/// from the key's registered type is coerced where it can be and kept as
/// flagged text where it cannot, so nothing an importer states is lost.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetaValue {
    /// Free text.
    Text(String),
    /// An exact decimal number.
    Number(Decimal),
    /// A boolean flag.
    Boolean(bool),
    /// A calendar date.
    Date(Date),
    /// An instant in time, in RFC 3339 form.
    ///
    /// Carried as text because `jiff` is not a plugin dependency, and the wire
    /// format is RFC 3339 either way.
    Timestamp(String),
    /// A value paired with a commodity code.
    Amount(Amount),
    /// An account path, e.g. `"Assets:Bank:Checking"`.
    ///
    /// The host resolves it against the account tree at persist time. A path
    /// naming no account is kept as text.
    Account(String),
}

/// One metadata key-value pair on a raw transaction or posting.
///
/// A key crosses the boundary exactly as an importer writes it; the host is
/// what normalises. It lowercases first, so `Payee` and `payee` are one key,
/// then requires `[a-z][a-z0-9_-]*` and at most 64 bytes. A key that still
/// fails costs its own entry and nothing else — the rest of the row is kept.
/// Repeated keys are permitted, and entries stay in the order stated.
///
/// # Example
///
/// ```rust
/// use bc_sdk::MetaEntry;
///
/// let entry = MetaEntry::text("payee", "Generic Grocer");
/// assert_eq!(entry.key, "payee");
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaEntry {
    /// The key this value is filed under.
    pub key: String,
    /// The value.
    pub value: MetaValue,
}

impl MetaEntry {
    /// Files a free-text value under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `text` - The value.
    #[inline]
    #[must_use]
    pub fn text(key: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Text(text.into()),
        }
    }

    /// Files a decimal number under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `number` - The value. Trailing zeros survive to the host.
    #[inline]
    #[must_use]
    pub fn number(key: impl Into<String>, number: Decimal) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Number(number),
        }
    }

    /// Files a boolean flag under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `flag` - The value.
    #[inline]
    #[must_use]
    pub fn boolean(key: impl Into<String>, flag: bool) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Boolean(flag),
        }
    }

    /// Files a calendar date under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `date` - The value.
    #[inline]
    #[must_use]
    pub fn date(key: impl Into<String>, date: Date) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Date(date),
        }
    }

    /// Files an RFC 3339 timestamp under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `timestamp` - The value, e.g. `"2026-01-15T09:30:00Z"`. The host
    ///   rejects text it cannot read as RFC 3339.
    #[inline]
    #[must_use]
    pub fn timestamp(key: impl Into<String>, timestamp: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Timestamp(timestamp.into()),
        }
    }

    /// Files a monetary amount under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `amount` - The value.
    #[inline]
    #[must_use]
    pub fn amount(key: impl Into<String>, amount: Amount) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Amount(amount),
        }
    }

    /// Files an account path under `key`.
    ///
    /// # Arguments
    ///
    /// * `key` - The metadata key.
    /// * `path` - The account path, e.g. `"Assets:Bank:Checking"`.
    #[inline]
    #[must_use]
    pub fn account(key: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: MetaValue::Account(path.into()),
        }
    }
}

/// A single posting leg of a raw transaction, prior to account-id resolution.
#[derive(bon::Builder, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RawPosting {
    /// Account path this leg credits/debits, e.g. `"Assets:Bank:Checking"`.
    #[builder(into)]
    pub account: String,
    /// Leg amount. `None` means elided — the residual that balances the transaction.
    pub amount: Option<Amount>,
    /// Per-account running balance after this leg, if the source reports it.
    pub balance: Option<Amount>,
    /// Tag names applied to this leg.
    #[builder(default)]
    pub tags: Vec<String>,
    /// Typed key-value metadata for this leg, in display order.
    #[builder(default)]
    pub metadata: Vec<MetaEntry>,
}

/// Where a [`RawTransaction`] came from, for diagnostics.
///
/// A source is not always a file — a plugin may call an API or read a database —
/// so `display` carries free-form human-facing text and `uri` carries an
/// optional addressable form. The host prints `display` verbatim in diagnostics.
#[derive(bon::Builder, Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourceLocation {
    /// Human-readable location, e.g. `"statements/june.csv row 14"`.
    #[builder(into)]
    pub display: String,
    /// Optional machine-addressable form, e.g. `"file:///june.csv#row=14"`.
    #[builder(into)]
    pub uri: Option<String>,
}

/// A parsed transaction prior to account binding. Carries one or more postings.
#[derive(bon::Builder, Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct RawTransaction {
    /// The transaction date.
    pub date: Date,
    /// A free-text description or memo.
    #[builder(into)]
    pub description: String,
    /// An institution-provided reference, if available (a dedup input).
    #[builder(into)]
    pub reference: Option<String>,
    /// Transaction-level tag names (e.g. Beancount `#josh`).
    #[builder(default)]
    pub tags: Vec<String>,
    /// Typed key-value metadata for this transaction, in display order.
    ///
    /// Everything an importer knows beyond the structural fields belongs here:
    /// a payee under `payee`, a memo under `note`, a settlement date under a
    /// key of its own.
    #[builder(default)]
    pub metadata: Vec<MetaEntry>,
    /// Where this transaction came from, if the importer can report it.
    pub source_location: Option<SourceLocation>,
    /// One or more posting legs. Single-account importers emit exactly one.
    ///
    /// Required: a transaction with no legs is meaningless, so plugin authors
    /// must supply at least one posting.
    pub postings: Vec<RawPosting>,
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
    pub fn as_typed<T>(&self) -> Result<T, serde_json::Error>
    where
        T: DeserializeOwned,
    {
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
use crate::__bindings::borrow_checker::sdk::types::MetaEntry as WitMetaEntry;
use crate::__bindings::borrow_checker::sdk::types::MetaValue as WitMetaValue;
use crate::__bindings::borrow_checker::sdk::types::RawPosting as WitRawPosting;
use crate::__bindings::borrow_checker::sdk::types::SourceLocation as WitSourceLocation;
use crate::__bindings::exports::borrow_checker::sdk::importer::ImportError as WitImportError;
use crate::__bindings::exports::borrow_checker::sdk::importer::RawTransaction as WitRawTransaction;

#[doc(hidden)]
impl From<MetaValue> for WitMetaValue {
    #[inline]
    fn from(v: MetaValue) -> Self {
        match v {
            MetaValue::Text(text) => Self::Text(text),
            MetaValue::Number(number) => Self::Number(number.to_string()),
            MetaValue::Boolean(flag) => Self::Boolean(flag),
            MetaValue::Date(date) => Self::Date(WitDate {
                year: date.year,
                month: date.month,
                day: date.day,
            }),
            MetaValue::Timestamp(stamp) => Self::Timestamp(stamp),
            MetaValue::Amount(amount) => Self::Amount(amount.into()),
            MetaValue::Account(path) => Self::Account(path),
        }
    }
}

#[doc(hidden)]
impl From<MetaEntry> for WitMetaEntry {
    #[inline]
    fn from(e: MetaEntry) -> Self {
        Self {
            key: e.key,
            value: e.value.into(),
        }
    }
}

#[doc(hidden)]
impl From<RawPosting> for WitRawPosting {
    #[inline]
    fn from(p: RawPosting) -> Self {
        Self {
            account: p.account,
            amount: p.amount.map(Into::into),
            balance: p.balance.map(Into::into),
            tags: p.tags,
            metadata: p.metadata.into_iter().map(Into::into).collect(),
        }
    }
}

#[doc(hidden)]
impl From<SourceLocation> for WitSourceLocation {
    #[inline]
    fn from(l: SourceLocation) -> Self {
        Self {
            display: l.display,
            uri: l.uri,
        }
    }
}

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
            description: t.description,
            reference: t.reference,
            tags: t.tags,
            metadata: t.metadata.into_iter().map(Into::into).collect(),
            source_location: t.source_location.map(Into::into),
            postings: t.postings.into_iter().map(Into::into).collect(),
        }
    }
}

#[doc(hidden)]
impl From<Amount> for WitAmount {
    #[inline]
    fn from(a: Amount) -> Self {
        Self {
            value: a.value.to_string(),
            commodity: a.commodity,
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
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use serde::Deserialize;

    use super::*;
    use crate::__bindings::borrow_checker::sdk::types::Amount as WitAmount;

    #[test]
    fn amount_carries_a_decimal_and_a_commodity() {
        let a = Amount::new(dec!(10.50), "AUD");
        assert_eq!(a.value, dec!(10.50));
        assert_eq!(a.commodity, "AUD");
    }

    /// The old i64 minor-units wire format capped an 18-decimal value at ±9.223.
    /// Real exchange exports exceed that, so the string form must carry it intact.
    #[test]
    fn amount_survives_eighteen_decimal_places_above_the_old_i64_ceiling() {
        let big = Decimal::from_str_exact("123.456789012345678").expect("valid decimal");
        let wit: WitAmount = Amount::new(big, "ETH").into();
        assert_eq!(wit.value, "123.456789012345678");
        assert_eq!(wit.commodity, "ETH");
    }

    /// Trailing zeros are significant: they are the source's stated precision.
    #[test]
    fn amount_preserves_trailing_zeros_on_the_wire() {
        let wit: WitAmount = Amount::new(dec!(50.00), "AUD").into();
        assert_eq!(wit.value, "50.00");
    }

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
    fn raw_transaction_builder_stores_fields() {
        let tx = RawTransaction::builder()
            .date(Date::new(2025, 6, 27))
            .description("Coffee")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank:Checking")
                    .maybe_amount(Some(Amount::new(dec!(-5.00), "AUD")))
                    .build(),
            ])
            .build();
        assert_eq!(tx.date, Date::new(2025, 6, 27));
        assert_eq!(tx.description, "Coffee");
        assert_eq!(tx.postings.len(), 1);
        let posting = tx.postings.first().expect("one posting");
        assert_eq!(posting.account, "Assets:Bank:Checking");
        assert_eq!(posting.amount, Some(Amount::new(dec!(-5.00), "AUD")));
    }

    #[test]
    fn meta_entry_constructors_cover_every_type() {
        let built = [
            MetaEntry::text("payee", "Generic Grocer"),
            MetaEntry::number("invoice", dec!(1502)),
            MetaEntry::boolean("reimbursable", true),
            MetaEntry::date("settled", Date::new(2026, 1, 15)),
            MetaEntry::timestamp("posted-at", "2026-01-15T09:30:00Z"),
            MetaEntry::amount("fee", Amount::new(dec!(1.50), "AUD")),
            MetaEntry::account("counterparty", "Assets:Bank:Savings"),
        ];

        let values: Vec<MetaValue> = built.iter().map(|e| e.value.clone()).collect();
        assert_eq!(
            values,
            vec![
                MetaValue::Text("Generic Grocer".to_owned()),
                MetaValue::Number(dec!(1502)),
                MetaValue::Boolean(true),
                MetaValue::Date(Date::new(2026, 1, 15)),
                MetaValue::Timestamp("2026-01-15T09:30:00Z".to_owned()),
                MetaValue::Amount(Amount::new(dec!(1.50), "AUD")),
                MetaValue::Account("Assets:Bank:Savings".to_owned()),
            ]
        );
        let keys: Vec<&str> = built.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "payee",
                "invoice",
                "reimbursable",
                "settled",
                "posted-at",
                "fee",
                "counterparty"
            ]
        );
    }

    /// A number crosses the boundary as text, so the scale the source stated
    /// must survive the conversion the same way an amount's does.
    #[test]
    fn meta_number_keeps_trailing_zeros_on_the_wire() {
        let wit: WitMetaValue = MetaValue::Number(dec!(1.500)).into();
        let WitMetaValue::Number(text) = wit else {
            panic!("a number must reach the wire as a number");
        };
        assert_eq!(text, "1.500");
    }

    #[test]
    fn a_transaction_states_no_metadata_by_default() {
        let tx = RawTransaction::builder()
            .date(Date::new(2026, 1, 15))
            .description("Coffee")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank:Checking")
                    .build(),
            ])
            .build();
        assert_eq!(tx.metadata, vec![]);
        assert_eq!(tx.postings.first().expect("one posting").metadata, vec![]);
    }

    #[test]
    fn metadata_reaches_the_wire_in_the_stated_order() {
        let tx = RawTransaction::builder()
            .date(Date::new(2026, 1, 15))
            .description("Coffee")
            .metadata(vec![
                MetaEntry::text("note", "first"),
                MetaEntry::text("payee", "Generic Grocer"),
                MetaEntry::text("note", "second"),
            ])
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank:Checking")
                    .build(),
            ])
            .build();

        let wit: WitRawTransaction = tx.into();
        let keys: Vec<&str> = wit.metadata.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["note", "payee", "note"]);
    }
}
