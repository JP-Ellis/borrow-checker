//! Monetary amount types and commodity codes.
//!
//! [`Amount`] pairs a [`rust_decimal::Decimal`] with a [`CommodityCode`],
//! allowing arbitrary commodity denominations (currencies, securities, etc.).
//! Use [`rusty_money::iso`] for ISO-4217 currency operations.

use core::fmt;

pub use rust_decimal::Decimal;
#[expect(
    clippy::module_name_repetitions,
    reason = "re-exporting the upstream type name verbatim for discoverability"
)]
pub use rusty_money::{Money, MoneyError, iso};

/// A commodity code string (e.g. `"USD"`, `"AUD"`, `"BTC"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct CommodityCode(String);

impl CommodityCode {
    /// Creates a new [`CommodityCode`] from a string.
    #[inline]
    #[must_use]
    pub fn new(code: impl Into<String>) -> Self {
        Self(code.into())
    }

    /// Returns the code as a string slice.
    #[inline]
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CommodityCode {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&'static rusty_money::iso::Currency> for CommodityCode {
    #[inline]
    fn from(c: &'static rusty_money::iso::Currency) -> Self {
        Self(c.iso_alpha_code.to_owned())
    }
}

/// A precise monetary amount with an associated commodity denomination.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Amount {
    /// The numeric value.
    pub value: Decimal,
    /// The commodity or currency code.
    pub commodity: CommodityCode,
}

impl Amount {
    /// Creates a new [`Amount`].
    ///
    /// # Arguments
    ///
    /// * `value` - The numeric value.
    /// * `commodity` - The commodity or currency code.
    #[inline]
    #[must_use]
    pub fn new(value: Decimal, commodity: impl Into<CommodityCode>) -> Self {
        Self {
            value,
            commodity: commodity.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn commodity_code_display() {
        let c = CommodityCode::new("USD");
        assert_eq!(c.to_string(), "USD");
    }

    #[test]
    fn amount_stores_value_and_commodity() {
        use rust_decimal_macros::dec;
        let amt = Amount {
            value: dec!(100.50),
            commodity: CommodityCode::new("USD"),
        };
        assert_eq!(amt.commodity.to_string(), "USD");
    }
}
