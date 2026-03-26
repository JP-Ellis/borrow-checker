//! Commodity entity — the rich registry of tradeable assets.

use core::fmt;
use core::str::FromStr;

use jiff::civil::Date;
use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

crate::define_id!(CommodityId, "commodity");

/// A tradeable asset or currency registered in the system.
///
/// Two commodities may share a `code` (e.g. `"AAPL"` on different exchanges).
/// [`CommodityId`] is the stable identity; `code` is display metadata.
///
/// # Example
///
/// ```
/// use bc_models::{Commodity, CommodityId};
///
/// let commodity = Commodity::builder()
///     .id(CommodityId::new())
///     .code("AUD")
///     .name("Australian Dollar")
///     .symbol("$")
///     .build();
///
/// assert_eq!(commodity.code(), "AUD");
/// assert_eq!(commodity.name(), Some("Australian Dollar"));
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Commodity {
    /// Stable, opaque identifier for this commodity. Assigned by `bc-core` on
    /// registration. Use this ID — not `code` — as the durable reference across renames.
    id: CommodityId,

    /// Ticker or currency code (e.g. `"AUD"`, `"BTC"`, `"AAPL"`). Must be non-empty.
    /// Multiple commodities may share a code when they trade on different exchanges;
    /// use [`CommodityId`] to distinguish them unambiguously.
    #[builder(into)]
    code: String,

    /// Exchange or market where this commodity trades (e.g. `"ASX"`, `"NASDAQ"`).
    /// `None` for fiat currencies and other globally-traded assets with no single exchange.
    #[builder(into)]
    exchange: Option<String>,

    /// Human-readable full name (e.g. `"Australian Dollar"`, `"Bitcoin"`).
    /// `None` if a name has not been recorded for this commodity.
    #[builder(into)]
    name: Option<String>,

    /// Optional free-text description providing additional context (e.g. `ISIN`, notes on
    /// the exchange listing). `None` if no description has been set.
    #[builder(into)]
    description: Option<String>,

    /// Display symbol used when formatting amounts (e.g. `"$"`, `"₿"`). `None` if no
    /// symbol has been recorded; in that case the `code` is used as the display fallback.
    #[builder(into)]
    symbol: Option<String>,

    /// First date from which this commodity is considered valid. `None` means there is
    /// no lower bound on validity.
    active_from: Option<Date>,

    /// Last date on which this commodity is considered valid (inclusive). `None` means
    /// there is no upper bound — the commodity is still active.
    active_until: Option<Date>,
}

impl Commodity {
    /// Returns the commodity ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &CommodityId {
        &self.id
    }

    /// Returns the commodity code (e.g. `"AUD"`).
    #[inline]
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns the exchange, if any.
    #[inline]
    #[must_use]
    pub fn exchange(&self) -> Option<&str> {
        self.exchange.as_deref()
    }

    /// Returns the human-readable name, if any.
    #[inline]
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the display symbol, if any.
    #[inline]
    #[must_use]
    pub fn symbol(&self) -> Option<&str> {
        self.symbol.as_deref()
    }

    /// Returns the date from which this commodity is valid.
    #[inline]
    #[must_use]
    pub fn active_from(&self) -> Option<Date> {
        self.active_from
    }

    /// Returns the date until which this commodity is valid.
    #[inline]
    #[must_use]
    pub fn active_until(&self) -> Option<Date> {
        self.active_until
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn commodity_id_has_correct_prefix() {
        assert!(CommodityId::new().to_string().starts_with("commodity_"));
    }

    #[test]
    fn commodity_builder_requires_code() {
        let c = Commodity::builder()
            .id(CommodityId::new())
            .code("AUD")
            .build();
        assert_eq!(c.code(), "AUD");
        assert!(c.name().is_none());
    }

    #[test]
    fn commodity_optional_fields() {
        let c = Commodity::builder()
            .id(CommodityId::new())
            .code("BTC")
            .name("Bitcoin".to_owned())
            .build();
        assert_eq!(c.name(), Some("Bitcoin"));
    }
}
