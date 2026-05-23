//! Monetary value type for use at the IPC boundary.

use serde::Deserialize;
use serde::Serialize;

/// A monetary amount at the IPC boundary: minor units plus a currency code.
///
/// `minor_units` is the amount in the currency's smallest unit — e.g. cents
/// for USD/AUD, satoshis for BTC, the integer itself for JPY (0 decimals).
/// Positive = credit, negative = debit.
///
/// `currency_code` is the ISO 4217 code (or informal code for crypto, e.g.
/// `"BTC"`).  The UI resolves display metadata via
/// [`bc_ipc::currency::currency_from_code`].
///
/// # Example
///
/// ```
/// use bc_ipc::Money;
///
/// let price = Money::new(-123_456, "AUD");  // −$1,234.56 AUD
/// assert_eq!(price.minor_units, -123_456);
/// assert_eq!(price.currency_code, "AUD");
/// ```
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Money {
    /// Amount in the currency's smallest unit (cents, satoshis, etc.).
    /// Positive = credit; negative = debit.
    pub minor_units: i64,
    /// ISO 4217 code or informal code (e.g. `"BTC"`).
    pub currency_code: String,
}

impl Money {
    /// Creates a new [`Money`] value.
    #[must_use]
    #[inline]
    pub fn new(minor_units: i64, currency_code: impl Into<String>) -> Self {
        Self {
            minor_units,
            currency_code: currency_code.into(),
        }
    }
}
