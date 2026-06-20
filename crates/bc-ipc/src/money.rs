//! Monetary value type for use at the IPC boundary.

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;

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
///
/// let price = Amount::from_minor(-123_456, "AUD", 2);  // −$1,234.56 AUD
/// assert_eq!(price.value, rust_decimal::Decimal::new(-123_456, 2));
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

    /// Creates an [`Amount`] from minor units and a scale.
    ///
    /// Convenience for tests and fixtures expressed in a currency's smallest
    /// unit (e.g. cents). `from_minor(1050, "AUD", 2)` is `10.50 AUD`.
    ///
    /// # Arguments
    ///
    /// * `minor_units` - Amount in the smallest unit.
    /// * `currency_code` - ISO 4217 or informal code.
    /// * `scale` - Number of decimal places.
    #[must_use]
    #[inline]
    pub fn from_minor(minor_units: i64, currency_code: impl Into<String>, scale: u8) -> Self {
        Self::new(Decimal::new(minor_units, u32::from(scale)), currency_code)
    }

    /// Returns the decimal value of this amount.
    #[must_use]
    #[inline]
    pub fn value(&self) -> Decimal {
        self.value
    }

    /// Returns a compact display string. Large amounts are abbreviated (`64k`, `1m`). Small
    /// amounts show the currency symbol. Returns `"—"` when no currency is set.
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
            Amount::from_minor(6_400_000, "USD", 2).format_short(),
            "64k"
        );
    }

    #[test]
    fn balance_short_millions() {
        assert_eq!(
            Amount::from_minor(120_000_000, "USD", 2).format_short(),
            "1m"
        );
    }

    #[test]
    fn balance_short_negative() {
        assert_eq!(
            Amount::from_minor(-244_000, "USD", 2).format_short(),
            "\u{2212}2k"
        );
    }

    #[test]
    fn balance_short_small() {
        assert_eq!(
            Amount::from_minor(42_100, "USD", 2).format_short(),
            "+$421.00"
        );
    }

    #[test]
    fn balance_short_jpy_millions() {
        assert_eq!(Amount::from_minor(1_500_000, "JPY", 0).format_short(), "1m");
    }

    #[test]
    fn balance_short_negative_thousands() {
        assert_eq!(
            Amount::from_minor(-150_000, "USD", 2).format_short(),
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
    fn from_minor_builds_scaled_decimal() {
        assert_eq!(
            Amount::from_minor(12_345, "BTC", 8).value,
            Decimal::new(12_345, 8)
        );
        assert_eq!(
            Amount::from_minor(1_234, "JPY", 0).value,
            Decimal::new(1_234, 0)
        );
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
}
