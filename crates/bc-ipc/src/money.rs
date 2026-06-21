//! Monetary value type for use at the IPC boundary.

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;

/// Errors that can occur during [`Amount`] arithmetic.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AmountError {
    /// The two operands carry different currency codes.
    #[error("commodity mismatch: {left} vs {right}")]
    CommodityMismatch {
        /// Currency code of the left-hand operand.
        left: String,
        /// Currency code of the right-hand operand.
        right: String,
    },
    /// The decimal result exceeded [`Decimal`]'s representable range.
    #[error("decimal overflow in amount arithmetic")]
    Overflow,
}

/// A monetary amount at the IPC boundary: a decimal value plus a currency code.
///
/// `value` carries arbitrary precision and its own scale (e.g. `10.50` keeps
/// two fraction digits); it serializes as a string via `rust_decimal`'s
/// `serde-with-str`. `currency_code` is the ISO 4217 code (or informal crypto
/// code, e.g. `"BTC"`). Positive = credit, negative = debit.
///
/// # Example
///
/// ```
/// use bc_ipc::Amount;
/// use rust_decimal::Decimal;
///
/// let price = Amount::new(Decimal::new(-123_456, 2), "AUD");  // −$1,234.56 AUD
/// assert_eq!(price.value, Decimal::new(-123_456, 2));
/// assert_eq!(price.currency_code, "AUD");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Amount {
    /// The decimal value. Positive = credit; negative = debit.
    pub value: Decimal,
    /// ISO 4217 code or informal code (e.g. `"BTC"`).
    pub currency_code: String,
}

impl Amount {
    /// Creates a new [`Amount`] from a [`Decimal`] value.
    ///
    /// # Arguments
    ///
    /// * `value` - The decimal value.
    /// * `currency_code` - ISO 4217 or informal code.
    #[must_use]
    #[inline]
    pub fn new(value: Decimal, currency_code: impl Into<String>) -> Self {
        Self {
            value,
            currency_code: currency_code.into(),
        }
    }

    /// Returns the decimal value of this amount.
    #[must_use]
    #[inline]
    pub fn value(&self) -> Decimal {
        self.value
    }

    /// Adds `other` to `self`, returning a new [`Amount`] in the shared currency.
    ///
    /// # Arguments
    ///
    /// * `other` - The amount to add; must carry the same currency code as `self`.
    ///
    /// # Returns
    ///
    /// The sum as a new [`Amount`].
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::CommodityMismatch`] if the currency codes differ,
    /// or [`AmountError::Overflow`] if the decimal sum exceeds [`Decimal`]'s range.
    #[inline]
    pub fn add(&self, other: &Self) -> Result<Self, AmountError> {
        if self.currency_code != other.currency_code {
            return Err(AmountError::CommodityMismatch {
                left: self.currency_code.clone(),
                right: other.currency_code.clone(),
            });
        }
        let sum = self
            .value
            .checked_add(other.value)
            .ok_or(AmountError::Overflow)?;
        Ok(Self::new(sum, self.currency_code.clone()))
    }

    /// Subtracts `other` from `self`, returning a new [`Amount`] in the shared currency.
    ///
    /// # Arguments
    ///
    /// * `other` - The amount to subtract; must carry the same currency code as `self`.
    ///
    /// # Returns
    ///
    /// The difference as a new [`Amount`].
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::CommodityMismatch`] if the currency codes differ,
    /// or [`AmountError::Overflow`] if the decimal difference exceeds [`Decimal`]'s range.
    #[inline]
    pub fn sub(&self, other: &Self) -> Result<Self, AmountError> {
        if self.currency_code != other.currency_code {
            return Err(AmountError::CommodityMismatch {
                left: self.currency_code.clone(),
                right: other.currency_code.clone(),
            });
        }
        let diff = self
            .value
            .checked_sub(other.value)
            .ok_or(AmountError::Overflow)?;
        Ok(Self::new(diff, self.currency_code.clone()))
    }

    /// Adds `other` to `self`, panicking on currency mismatch or overflow.
    ///
    /// For call sites that have already guaranteed a shared currency.
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
    /// Panics if the currency codes differ or the decimal sum overflows.
    #[inline]
    #[must_use]
    #[expect(
        clippy::panic,
        reason = "panics are intentional for unchecked arithmetic"
    )]
    pub fn add_unchecked(&self, other: &Self) -> Self {
        match self.add(other) {
            Ok(amount) => amount,
            Err(error) => panic!("Amount::add_unchecked: {error}"),
        }
    }

    /// Subtracts `other` from `self`, panicking on currency mismatch or overflow.
    ///
    /// For call sites that have already guaranteed a shared currency.
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
    /// Panics if the currency codes differ or the decimal difference overflows.
    #[inline]
    #[must_use]
    #[expect(
        clippy::panic,
        reason = "panics are intentional for unchecked arithmetic"
    )]
    pub fn sub_unchecked(&self, other: &Self) -> Self {
        match self.sub(other) {
            Ok(amount) => amount,
            Err(error) => panic!("Amount::sub_unchecked: {error}"),
        }
    }

    /// Returns a compact display string. Large amounts are abbreviated (`64k`, `1m`). Small
    /// amounts show the currency symbol. Returns `"—"` when no currency is set.
    ///
    /// Fraction digits follow the value's own [`Decimal`] scale. For currency-canonical
    /// fraction digits used by the main UI money display, see bc-ui's `format_amount`.
    #[must_use]
    #[inline]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "display approximation — Decimal division by non-zero constants for k/m thresholds cannot overflow or panic"
    )]
    pub fn format_short(&self) -> String {
        if self.currency_code.is_empty() {
            return "\u{2014}".into();
        }
        let abs = self.value.abs();
        let prefix = if self.value.is_sign_negative() {
            "\u{2212}"
        } else {
            ""
        };
        let million = Decimal::from(1_000_000_i64);
        let thousand = Decimal::from(1_000_i64);
        if abs >= million {
            format!("{prefix}{}m", (abs / million).trunc())
        } else if abs >= thousand {
            format!("{prefix}{}k", (abs / thousand).trunc())
        } else {
            let sign = match self.value.cmp(&Decimal::ZERO) {
                core::cmp::Ordering::Greater => "+",
                core::cmp::Ordering::Less => "\u{2212}",
                core::cmp::Ordering::Equal => "",
            };
            let decimal = abs.to_string();
            match crate::currency_from_code(&self.currency_code) {
                Some(c) if c.symbol_after => format!("{sign}{decimal}\u{00a0}{}", c.symbol),
                Some(c) => format!("{sign}{}{decimal}", c.symbol),
                None => format!("{sign}{} {decimal}", self.currency_code),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rust_decimal::Decimal;

    use super::Amount;

    #[test]
    fn balance_short_thousands() {
        assert_eq!(
            Amount::new(Decimal::new(6_400_000, 2), "USD").format_short(),
            "64k"
        );
    }

    #[test]
    fn balance_short_millions() {
        assert_eq!(
            Amount::new(Decimal::new(120_000_000, 2), "USD").format_short(),
            "1m"
        );
    }

    #[test]
    fn balance_short_negative() {
        assert_eq!(
            Amount::new(Decimal::new(-244_000, 2), "USD").format_short(),
            "\u{2212}2k"
        );
    }

    #[test]
    fn balance_short_small() {
        assert_eq!(
            Amount::new(Decimal::new(42_100, 2), "USD").format_short(),
            "+$421.00"
        );
    }

    #[test]
    fn balance_short_jpy_millions() {
        assert_eq!(
            Amount::new(Decimal::new(1_500_000, 0), "JPY").format_short(),
            "1m"
        );
    }

    #[test]
    fn balance_short_negative_thousands() {
        assert_eq!(
            Amount::new(Decimal::new(-150_000, 2), "USD").format_short(),
            "\u{2212}1k"
        );
    }

    #[test]
    fn amount_value_roundtrips_through_json() {
        let a = Amount::new(Decimal::new(1050, 2), "AUD"); // 10.50
        let json = serde_json::to_string(&a).expect("ser");
        assert_eq!(json, r#"{"value":"10.50","currency_code":"AUD"}"#);
        let back: Amount = serde_json::from_str(&json).expect("de");
        assert_eq!(a, back);
        assert_eq!(back.value, Decimal::new(1050, 2));
    }

    #[test]
    fn large_high_precision_value_survives() {
        // ~1e20 mantissa: would overflow i64 minor-units under the old representation.
        let big = Decimal::from_i128_with_scale(100_000_000_000_000_000_000_i128, 8);
        let a = Amount::new(big, "BTC");
        let json = serde_json::to_string(&a).expect("ser");
        let back: Amount = serde_json::from_str(&json).expect("de");
        assert_eq!(back.value, big);
        assert_ne!(back.value, Decimal::ZERO);
    }

    #[test]
    fn add_same_commodity_sums_value() {
        let a = Amount::new(Decimal::new(1050, 2), "AUD"); // 10.50
        let b = Amount::new(Decimal::new(425, 2), "AUD"); // 4.25
        let sum = a.add(&b).expect("same commodity");
        assert_eq!(sum.value, Decimal::new(1475, 2));
        assert_eq!(sum.currency_code, "AUD");
    }

    #[test]
    fn sub_same_commodity_subtracts_value() {
        let a = Amount::new(Decimal::new(10, 0), "USD");
        let b = Amount::new(Decimal::new(3, 0), "USD");
        assert_eq!(a.sub(&b).expect("same commodity").value, Decimal::new(7, 0));
    }

    #[test]
    fn add_commodity_mismatch_errors() {
        let a = Amount::new(Decimal::new(10, 0), "AUD");
        let b = Amount::new(Decimal::new(10, 0), "USD");
        assert_eq!(
            a.add(&b),
            Err(crate::AmountError::CommodityMismatch {
                left: "AUD".to_owned(),
                right: "USD".to_owned(),
            })
        );
    }

    #[test]
    #[should_panic(expected = "commodity mismatch")]
    fn add_unchecked_panics_on_mismatch() {
        let a = Amount::new(Decimal::new(10, 0), "AUD");
        let b = Amount::new(Decimal::new(10, 0), "USD");
        drop(a.add_unchecked(&b));
    }

    #[test]
    fn add_overflow_errors() {
        let max = Amount::new(Decimal::MAX, "AUD");
        let one = Amount::new(Decimal::new(1, 0), "AUD");
        assert_eq!(max.add(&one), Err(crate::AmountError::Overflow));
    }

    #[test]
    #[should_panic(expected = "commodity mismatch")]
    fn sub_unchecked_panics_on_mismatch() {
        let a = Amount::new(Decimal::new(10, 0), "AUD");
        let b = Amount::new(Decimal::new(10, 0), "USD");
        drop(a.sub_unchecked(&b));
    }

    #[test]
    fn sub_unchecked_returns_difference() {
        let a = Amount::new(Decimal::new(10, 0), "USD");
        let b = Amount::new(Decimal::new(3, 0), "USD");
        assert_eq!(a.sub_unchecked(&b).value, Decimal::new(7, 0));
    }
}
