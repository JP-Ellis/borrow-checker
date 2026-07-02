//! Currency metadata shared between the native backend and WASM frontend.
//!
//! Currency/commodity metadata ([`CommodityInfo`]) is served over IPC by
//! `bc-app` rather than hardcoded here.

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
    /// Minor-unit digits shown when formatting (e.g. `2`, `0`, `8`).
    pub decimals: u8,
    /// Whether this is an ISO 4217 currency (drives `Intl.NumberFormat` style).
    pub is_iso: bool,
    /// Whether the symbol follows the amount (ignored for ISO currencies).
    pub symbol_after: bool,
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
        decimals: u8,
        is_iso: bool,
        symbol_after: bool,
    ) -> Self {
        Self {
            id: id.into(),
            code: code.into(),
            symbol,
            aliases,
            decimals,
            is_iso,
            symbol_after,
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
            2,
            true,
            false,
        );
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<CommodityInfo>(&json).unwrap(), c);
    }
}
