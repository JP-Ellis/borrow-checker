//! Monetary amount types and commodity codes.
//!
//! [`Amount`] pairs a [`rust_decimal::Decimal`] with a [`CommodityCode`],
//! allowing arbitrary commodity denominations (currencies, securities, etc.).
//!
//! # `CommodityCode` vs `Commodity`
//!
//! This crate also defines a rich [`Commodity`] struct (with a stable
//! [`CommodityId`], exchange, description, etc.). [`CommodityCode`] is a
//! deliberately simpler, complementary type — the two serve different layers:
//!
//! - **`CommodityCode` is unresolved.** It is the raw ticker or currency string
//!   as it appears in external formats (Beancount, OFX, CSV, Ledger). It can be
//!   constructed anywhere, with no registry or database access required.
//!
//! - **Codes are not unique identifiers.** The same code (e.g. `"AAPL"`) may
//!   refer to different [`Commodity`] entries on different exchanges.
//!   [`CommodityCode`] intentionally preserves that ambiguity; [`CommodityId`]
//!   is the stable, unambiguous reference once the commodity is registered.
//!
//! - **`Amount` stays lightweight.** By using [`CommodityCode`] rather than
//!   [`CommodityId`], [`Amount`] is a plain value type — no registry lookup is
//!   needed to construct one. This is essential for the import/parsing pipeline,
//!   where amounts are assembled before commodities are resolved.
//!
//! Resolution from [`CommodityCode`] to a [`Commodity`] / [`CommodityId`]
//! happens at a higher layer (bc-core) once the exchange context is known.
//!
//! [`Commodity`]: crate::Commodity
//! [`CommodityId`]: crate::CommodityId

use core::fmt;

pub use rust_decimal::Decimal;

/// A commodity code string (e.g. `"USD"`, `"AUD"`, `"BTC"`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
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

impl From<String> for CommodityCode {
    #[inline]
    fn from(code: String) -> Self {
        Self(code)
    }
}

impl From<&str> for CommodityCode {
    #[inline]
    fn from(code: &str) -> Self {
        Self(code.to_owned())
    }
}

/// Errors that can occur during [`Amount`] arithmetic.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AmountError {
    /// The two operands carry different commodities.
    #[error("commodity mismatch: {left} vs {right}")]
    CommodityMismatch {
        /// Commodity of the left-hand operand.
        left: String,
        /// Commodity of the right-hand operand.
        right: String,
    },
    /// The decimal result exceeded [`Decimal`]'s representable range.
    #[error("decimal overflow in amount arithmetic")]
    Overflow,
}

/// A precise monetary amount with an associated commodity denomination.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Amount {
    /// The numeric value.
    value: Decimal,
    /// The commodity or currency code.
    commodity: CommodityCode,
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

    /// Returns the numeric value of this amount.
    #[inline]
    #[must_use]
    pub fn value(&self) -> Decimal {
        self.value
    }

    /// Returns the commodity code of this amount.
    #[inline]
    #[must_use]
    pub fn commodity(&self) -> &CommodityCode {
        &self.commodity
    }

    /// Adds `other` to `self`, returning a new [`Amount`] in the shared commodity.
    ///
    /// # Arguments
    ///
    /// * `other` - The amount to add; must carry the same commodity as `self`.
    ///
    /// # Returns
    ///
    /// The sum as a new [`Amount`].
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::CommodityMismatch`] if the commodities differ, or
    /// [`AmountError::Overflow`] if the decimal sum exceeds [`Decimal`]'s range.
    #[inline]
    pub fn add(&self, other: &Self) -> Result<Self, AmountError> {
        if self.commodity != other.commodity {
            return Err(AmountError::CommodityMismatch {
                left: self.commodity.to_string(),
                right: other.commodity.to_string(),
            });
        }
        let sum = self
            .value
            .checked_add(other.value)
            .ok_or(AmountError::Overflow)?;
        Ok(Self::new(sum, self.commodity.clone()))
    }

    /// Subtracts `other` from `self`, returning a new [`Amount`] in the shared commodity.
    ///
    /// # Arguments
    ///
    /// * `other` - The amount to subtract; must carry the same commodity as `self`.
    ///
    /// # Returns
    ///
    /// The difference as a new [`Amount`].
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::CommodityMismatch`] if the commodities differ, or
    /// [`AmountError::Overflow`] if the decimal difference exceeds [`Decimal`]'s range.
    #[inline]
    pub fn sub(&self, other: &Self) -> Result<Self, AmountError> {
        if self.commodity != other.commodity {
            return Err(AmountError::CommodityMismatch {
                left: self.commodity.to_string(),
                right: other.commodity.to_string(),
            });
        }
        let diff = self
            .value
            .checked_sub(other.value)
            .ok_or(AmountError::Overflow)?;
        Ok(Self::new(diff, self.commodity.clone()))
    }

    /// Adds `other` to `self`, panicking on commodity mismatch or overflow.
    ///
    /// For call sites that have already guaranteed a shared commodity.
    ///
    /// # Arguments
    ///
    /// * `other` - The amount to add.
    ///
    /// # Returns
    ///
    /// The sum as a new [`Amount`].
    ///
    /// # Panics
    ///
    /// Panics if the commodities differ or the decimal sum overflows.
    #[inline]
    #[must_use]
    pub fn add_unchecked(&self, other: &Self) -> Self {
        match self.add(other) {
            Ok(amount) => amount,
            #[expect(
                clippy::panic,
                reason = "intentional panic for call sites that have guaranteed shared commodity"
            )]
            Err(error) => panic!("Amount::add_unchecked: {error}"),
        }
    }

    /// Subtracts `other` from `self`, panicking on commodity mismatch or overflow.
    ///
    /// For call sites that have already guaranteed a shared commodity.
    ///
    /// # Arguments
    ///
    /// * `other` - The amount to subtract.
    ///
    /// # Returns
    ///
    /// The difference as a new [`Amount`].
    ///
    /// # Panics
    ///
    /// Panics if the commodities differ or the decimal difference overflows.
    #[inline]
    #[must_use]
    pub fn sub_unchecked(&self, other: &Self) -> Self {
        match self.sub(other) {
            Ok(amount) => amount,
            #[expect(
                clippy::panic,
                reason = "intentional panic for call sites that have guaranteed shared commodity"
            )]
            Err(error) => panic!("Amount::sub_unchecked: {error}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn commodity_code_display() {
        let c = CommodityCode::new("USD");
        assert_eq!(c.to_string(), "USD");
    }

    #[test]
    fn amount_stores_value_and_commodity() {
        let amt = Amount::new(dec!(100.50), CommodityCode::new("USD"));
        assert_eq!(amt.commodity().to_string(), "USD");
    }

    #[test]
    fn add_same_commodity_sums_value() {
        let a = Amount::new(dec!(10.50), "AUD");
        let b = Amount::new(dec!(4.25), "AUD");
        let sum = a.add(&b).expect("same commodity");
        assert_eq!(sum.value(), dec!(14.75));
        assert_eq!(sum.commodity().as_str(), "AUD");
    }

    #[test]
    fn sub_same_commodity_subtracts_value() {
        let a = Amount::new(dec!(10), "USD");
        let b = Amount::new(dec!(3), "USD");
        assert_eq!(a.sub(&b).expect("same commodity").value(), dec!(7));
    }

    #[test]
    fn add_commodity_mismatch_errors() {
        let a = Amount::new(dec!(10), "AUD");
        let b = Amount::new(dec!(10), "USD");
        assert_eq!(
            a.add(&b),
            Err(AmountError::CommodityMismatch {
                left: "AUD".to_owned(),
                right: "USD".to_owned(),
            })
        );
    }

    #[test]
    #[should_panic(expected = "commodity mismatch")]
    fn add_unchecked_panics_on_mismatch() {
        let a = Amount::new(dec!(10), "AUD");
        let b = Amount::new(dec!(10), "USD");
        drop(a.add_unchecked(&b));
    }

    #[test]
    fn add_overflow_errors() {
        let max = Amount::new(Decimal::MAX, "AUD");
        let one = Amount::new(dec!(1), "AUD");
        assert_eq!(max.add(&one), Err(AmountError::Overflow));
    }

    #[test]
    fn commodity_code_from_str_and_string() {
        assert_eq!(CommodityCode::from("EUR").as_str(), "EUR");
        assert_eq!(CommodityCode::from("EUR".to_owned()).as_str(), "EUR");
    }
}
