//! Commodity entity — the rich registry of tradeable assets.

use jiff::civil::Date;

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
///     .code("AUD")
///     .name("Australian Dollar")
///     .symbol("$")
///     .decimals(2)
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
    /// registration. Use this ID — not `code` — as the durable reference across
    /// renames.
    #[builder(default)]
    id: CommodityId,

    /// Ticker or currency code (e.g. `"AUD"`, `"BTC"`, `"AAPL"`). Must be
    /// non-empty. Multiple commodities may share a code when they trade on
    /// different exchanges; use [`CommodityId`] to distinguish them
    /// unambiguously.
    #[builder(into)]
    code: String,

    /// Exchange or market where this commodity trades (e.g. `"ASX"`,
    /// `"NASDAQ"`). `None` for fiat currencies and other globally-traded assets
    /// with no single exchange.
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

    /// Acceptable input markers besides `code`/`symbol` (e.g. `"A$"`, `"AU$"`).
    /// Resolution matches against `{code} ∪ {symbol} ∪ aliases`. Defaults to empty.
    #[builder(default)]
    aliases: Vec<String>,

    /// Number of minor-unit digits shown when formatting amounts (e.g. `2` for
    /// cents, `0` for yen, `8` for satoshis). Required — there is no universally
    /// safe default, so every commodity must state its own precision.
    decimals: u8,

    /// Whether this is an ISO 4217 currency. When `true`, display delegates
    /// symbol and grouping to `Intl.NumberFormat`'s currency style; when
    /// `false` (e.g. crypto), the symbol is placed manually. Defaults to `true`.
    #[builder(default = true)]
    is_iso: bool,

    /// Whether the display symbol follows the amount (e.g. `100 ETH`) rather than
    /// preceding it (`$100`). Ignored for ISO currencies. Defaults to `false`.
    #[builder(default)]
    symbol_after: bool,

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

    /// Returns the alternate input markers for this commodity.
    #[inline]
    #[must_use]
    pub fn aliases(&self) -> &[String] {
        &self.aliases
    }

    /// Returns the number of minor-unit digits used when formatting amounts.
    #[inline]
    #[must_use]
    pub fn decimals(&self) -> u8 {
        self.decimals
    }

    /// Returns whether this is an ISO 4217 currency.
    #[inline]
    #[must_use]
    pub fn is_iso(&self) -> bool {
        self.is_iso
    }

    /// Returns whether the display symbol follows the amount.
    #[inline]
    #[must_use]
    pub fn symbol_after(&self) -> bool {
        self.symbol_after
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
        let c = Commodity::builder().code("AUD").decimals(2).build();
        assert_eq!(c.code(), "AUD");
        assert!(c.name().is_none());
    }

    #[test]
    fn commodity_optional_fields() {
        let c = Commodity::builder()
            .code("BTC")
            .name("Bitcoin".to_owned())
            .decimals(2)
            .build();
        assert_eq!(c.name(), Some("Bitcoin"));
    }

    #[test]
    fn commodity_all_optional_fields_set() {
        use jiff::civil::date;

        let c = Commodity::builder()
            .code("AAPL")
            .exchange("NASDAQ")
            .name("Apple Inc.")
            .description("US large-cap technology stock")
            .symbol("$")
            .active_from(date(2000, 1, 1))
            .active_until(date(2099, 12, 31))
            .decimals(2)
            .build();

        assert_eq!(c.code(), "AAPL");
        assert_eq!(c.exchange(), Some("NASDAQ"));
        assert_eq!(c.name(), Some("Apple Inc."));
        assert_eq!(c.description(), Some("US large-cap technology stock"));
        assert_eq!(c.symbol(), Some("$"));
        assert_eq!(c.active_from(), Some(date(2000, 1, 1)));
        assert_eq!(c.active_until(), Some(date(2099, 12, 31)));
    }

    #[test]
    fn commodity_no_optional_fields_returns_none() {
        let c = Commodity::builder().code("EUR").decimals(2).build();

        assert_eq!(c.code(), "EUR");
        assert!(c.exchange().is_none());
        assert!(c.name().is_none());
        assert!(c.description().is_none());
        assert!(c.symbol().is_none());
        assert!(c.active_from().is_none());
        assert!(c.active_until().is_none());
    }

    #[test]
    fn aliases_default_empty_and_settable() {
        let c = Commodity::builder().code("AUD").decimals(2).build();
        assert!(c.aliases().is_empty());
        let c2 = Commodity::builder()
            .code("AUD")
            .aliases(vec!["A$".to_owned(), "AU$".to_owned()])
            .decimals(2)
            .build();
        assert_eq!(c2.aliases(), &["A$".to_owned(), "AU$".to_owned()]);
    }

    #[test]
    fn display_metadata_defaults_and_overrides() {
        let default = Commodity::builder().code("AUD").decimals(2).build();
        assert_eq!(default.decimals(), 2);
        assert!(default.is_iso());
        assert!(!default.symbol_after());

        let eth = Commodity::builder()
            .code("ETH")
            .decimals(9)
            .is_iso(false)
            .symbol_after(true)
            .build();
        assert_eq!(eth.decimals(), 9);
        assert!(!eth.is_iso());
        assert!(eth.symbol_after());
    }
}
