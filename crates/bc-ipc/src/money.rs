//! Monetary value type for use at the IPC boundary.

use serde::Deserialize;
use serde::Serialize;

/// A monetary amount at the IPC boundary: minor units, a currency code, and an
/// explicit decimal scale.
///
/// `minor_units` is the amount in the currency's smallest unit — e.g. cents
/// for USD/AUD (`scale = 2`), satoshis for BTC (`scale = 8`), the integer
/// itself for JPY (`scale = 0`).  Positive = credit, negative = debit.
///
/// `currency_code` is the ISO 4217 code (or informal code for crypto, e.g.
/// `"BTC"`).  The UI resolves display metadata via
/// [`bc_ipc::currency_from_code`].
///
/// `scale` is the number of decimal places — e.g. `2` for AUD cents,
/// `8` for BTC satoshis. Carried explicitly so that arbitrary commodities
/// (crypto, securities, custom units) round-trip correctly without a lookup
/// table.
///
/// # Example
///
/// ```
/// use bc_ipc::Amount;
///
/// let price = Amount::new(-123_456, "AUD", 2);  // −$1,234.56 AUD
/// assert_eq!(price.minor_units, -123_456);
/// assert_eq!(price.currency_code, "AUD");
/// assert_eq!(price.scale, 2);
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Amount {
    /// Amount in the currency's smallest unit (cents, satoshis, etc.).
    /// Positive = credit; negative = debit.
    pub minor_units: i64,
    /// ISO 4217 code or informal code (e.g. `"BTC"`).
    pub currency_code: String,
    /// Number of decimal places (e.g. `2` for AUD cents, `8` for BTC satoshis).
    pub scale: u8,
}

impl Amount {
    /// Creates a new [`Amount`] value.
    #[must_use]
    #[inline]
    pub fn new(minor_units: i64, currency_code: impl Into<String>, scale: u8) -> Self {
        Self {
            minor_units,
            currency_code: currency_code.into(),
            scale,
        }
    }

    /// Returns a compact display string. Large amounts are abbreviated (`64k`, `1m`). Small
    /// amounts show the currency symbol. Returns `"—"` when no currency is set.
    #[must_use]
    #[inline]
    #[expect(
        clippy::integer_division,
        clippy::integer_division_remainder_used,
        clippy::arithmetic_side_effects,
        reason = "display approximation — integer division for k/m thresholds and decimal formatting cannot overflow or panic"
    )]
    pub fn format_short(&self) -> String {
        if self.currency_code.is_empty() {
            return "\u{2014}".into();
        }
        let abs = self.minor_units.unsigned_abs();
        let scale_factor = 10_u64.saturating_pow(u32::from(self.scale));
        let prefix = if self.minor_units < 0 { "\u{2212}" } else { "" };
        if abs >= 1_000_000 * scale_factor {
            format!("{prefix}{}m", abs / (1_000_000 * scale_factor))
        } else if abs >= 1_000 * scale_factor {
            format!("{prefix}{}k", abs / (1_000 * scale_factor))
        } else {
            let sign = match self.minor_units.cmp(&0) {
                core::cmp::Ordering::Greater => "+",
                core::cmp::Ordering::Less => "\u{2212}",
                core::cmp::Ordering::Equal => "",
            };
            let decimal = if self.scale == 0 {
                abs.to_string()
            } else {
                let integer = abs / scale_factor;
                let frac = abs % scale_factor;
                let width = usize::from(self.scale);
                format!("{integer}.{frac:0>width$}")
            };
            match crate::currency_from_code(&self.currency_code) {
                Some(c) if c.symbol_after => {
                    format!("{sign}{decimal}\u{00a0}{}", c.symbol)
                }
                Some(c) => format!("{sign}{}{decimal}", c.symbol),
                None => format!("{sign}{} {decimal}", self.currency_code),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::Amount;

    #[test]
    fn balance_short_thousands() {
        assert_eq!(Amount::new(6_400_000, "USD", 2).format_short(), "64k");
    }

    #[test]
    fn balance_short_millions() {
        assert_eq!(Amount::new(120_000_000, "USD", 2).format_short(), "1m");
    }

    #[test]
    fn balance_short_negative() {
        assert_eq!(Amount::new(-244_000, "USD", 2).format_short(), "\u{2212}2k");
    }

    #[test]
    fn balance_short_small() {
        assert_eq!(Amount::new(42_100, "USD", 2).format_short(), "+$421.00");
    }

    #[test]
    fn balance_short_jpy_millions() {
        assert_eq!(Amount::new(1_500_000, "JPY", 0).format_short(), "1m");
    }

    #[test]
    fn balance_short_negative_thousands() {
        assert_eq!(Amount::new(-150_000, "USD", 2).format_short(), "\u{2212}1k");
    }
}
