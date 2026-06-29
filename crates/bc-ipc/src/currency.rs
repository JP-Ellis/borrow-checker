//! Currency metadata shared between the native backend and WASM frontend.
//!
//! # Stop-gap
//!
//! The constants here (`USD`, `AUD`, …) are hardcoded until the backend
//! serves currency metadata over IPC.
//!
//! TODO(ipc): replace constant registry with data from IPC once currency
//! metadata is served by bc-app.

use serde::Deserialize;
use serde::Serialize;

/// A registered commodity/currency for the UI: canonical code, optional display
/// symbol, and acceptable input-marker aliases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CommodityInfo {
    /// Stable commodity id.
    pub id: String,
    /// Canonical code (e.g. `"AUD"`).
    pub code: String,
    /// Display symbol, if recorded.
    pub symbol: Option<String>,
    /// Alternate input markers (e.g. `["AU$"]`).
    pub aliases: Vec<String>,
}

impl CommodityInfo {
    /// Creates a [`CommodityInfo`].
    #[must_use]
    #[inline]
    pub fn new(
        id: impl Into<String>,
        code: impl Into<String>,
        symbol: Option<String>,
        aliases: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            code: code.into(),
            symbol,
            aliases,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::CommodityInfo;

    #[test]
    #[expect(clippy::unwrap_used, reason = "unwrap is acceptable in test code")]
    fn commodity_info_round_trips() {
        let c = CommodityInfo::new(
            "commodity_x",
            "AUD",
            Some("A$".to_owned()),
            vec!["AU$".to_owned()],
        );
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<CommodityInfo>(&json).unwrap(), c);
    }
}

/// Metadata describing how a currency's amounts are displayed.
///
/// Formatting on the UI side delegates to the browser's `Intl.NumberFormat`
/// API, which handles locale-aware grouping, decimal separators, and symbol
/// placement.  The `symbol` and `symbol_after` fields are used only for
/// non-ISO currencies (e.g. crypto) where `Intl` cannot look up the symbol.
///
/// Use the provided constants ([`USD`], [`AUD`], etc.) rather than
/// constructing directly; the struct is `#[non_exhaustive]` to allow new
/// fields without a breaking change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Currency {
    /// ISO 4217 code (or informal code for crypto, e.g. `"BTC"`).
    pub code: &'static str,
    /// Display symbol (e.g. `"$"`, `"€"`, `"₿"`).  Ignored for ISO
    /// currencies; `Intl.NumberFormat` supplies the locale-correct symbol.
    pub symbol: &'static str,
    /// If `true` the symbol follows the amount; otherwise it precedes it.
    /// Ignored for ISO currencies (locale determines placement).
    pub symbol_after: bool,
    /// Number of minor-unit digits (e.g. 2 for cents, 8 for satoshis, 0 for yen).
    pub decimals: u8,
    /// Whether this is an ISO 4217 currency.
    ///
    /// `true`  → `Intl.NumberFormat` `style:"currency"` handles symbol + grouping.
    /// `false` → `Intl.NumberFormat` `style:"decimal"` + manual symbol placement.
    pub is_iso: bool,
}

// TODO(ipc): replace the constants and lookup table below with data served by
// the backend once IPC currency metadata is implemented.

/// US Dollar.
pub const USD: Currency = Currency {
    code: "USD",
    symbol: "$",
    symbol_after: false,
    decimals: 2,
    is_iso: true,
};

/// Australian Dollar.
pub const AUD: Currency = Currency {
    code: "AUD",
    symbol: "A$",
    symbol_after: false,
    decimals: 2,
    is_iso: true,
};

/// Euro.
pub const EUR: Currency = Currency {
    code: "EUR",
    symbol: "€",
    symbol_after: false,
    decimals: 2,
    is_iso: true,
};

/// British Pound.
pub const GBP: Currency = Currency {
    code: "GBP",
    symbol: "£",
    symbol_after: false,
    decimals: 2,
    is_iso: true,
};

/// Japanese Yen (no decimal places).
pub const JPY: Currency = Currency {
    code: "JPY",
    symbol: "¥",
    symbol_after: false,
    decimals: 0,
    is_iso: true,
};

/// Korean Won (no decimal places).
pub const KRW: Currency = Currency {
    code: "KRW",
    symbol: "₩",
    symbol_after: false,
    decimals: 0,
    is_iso: true,
};

/// Indian Rupee (South Asian grouping handled by `Intl.NumberFormat`).
pub const INR: Currency = Currency {
    code: "INR",
    symbol: "₹",
    symbol_after: false,
    decimals: 2,
    is_iso: true,
};

/// Bitcoin (minor unit: satoshi = 10⁻⁸ BTC).
pub const BTC: Currency = Currency {
    code: "BTC",
    symbol: "₿",
    symbol_after: false,
    decimals: 8,
    is_iso: false,
};

/// Ethereum (minor unit: nanoether = 10⁻⁹ ETH).
///
/// Full wei precision (10⁻¹⁸) doesn't fit in `i64` for useful amounts;
/// nanoether is used as the minor unit for practical range.
pub const ETH: Currency = Currency {
    code: "ETH",
    symbol: "ETH",
    symbol_after: true,
    decimals: 9,
    is_iso: false,
};

// TODO(ipc): replace with a server-provided registry once currency metadata
// is served by bc-app over IPC.
/// Look up a [`Currency`] by its ISO 4217 code (or informal code for crypto).
///
/// Returns `None` if the code is not in the stop-gap registry.
#[must_use]
pub fn currency_from_code(code: &str) -> Option<&'static Currency> {
    match code {
        "USD" => Some(&USD),
        "AUD" => Some(&AUD),
        "EUR" => Some(&EUR),
        "GBP" => Some(&GBP),
        "JPY" => Some(&JPY),
        "KRW" => Some(&KRW),
        "INR" => Some(&INR),
        "BTC" => Some(&BTC),
        "ETH" => Some(&ETH),
        _ => None,
    }
}
