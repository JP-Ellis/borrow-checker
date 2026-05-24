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
}
