//! Commodity entity — the rich registry of tradeable assets.

use core::{fmt, str::FromStr};

use jiff::civil::Date;
use mti::prelude::*;
use serde::{Deserialize, Serialize};

crate::define_id!(CommodityId, "commodity");

/// A tradeable asset or currency registered in the system.
///
/// Two commodities may share a `code` (e.g. `"AAPL"` on different exchanges).
/// [`CommodityId`] is the stable identity; `code` is display metadata.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Commodity {
    /// The unique identifier for this commodity.
    id: CommodityId,
    /// Non-empty code string, e.g. `"AUD"`, `"BTC"`, `"AAPL"`.
    #[builder(into)]
    code: String,
    /// Exchange or market, e.g. `"ASX"`, `"NASDAQ"`. `None` for fiat/universal.
    #[builder(into)]
    exchange: Option<String>,
    /// Human-readable name, e.g. `"Australian Dollar"`.
    #[builder(into)]
    name: Option<String>,
    /// Optional description.
    #[builder(into)]
    description: Option<String>,
    /// Display symbol, e.g. `"$"`, `"₿"`.
    #[builder(into)]
    symbol: Option<String>,
    /// Date from which this commodity is valid.
    active_from: Option<Date>,
    /// Date until which this commodity is valid.
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
