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

// MARK: models conversions

#[cfg(feature = "models")]
impl From<&bc_models::Commodity> for CommodityInfo {
    /// Converts a [`bc_models::Commodity`] to an IPC [`CommodityInfo`].
    ///
    /// The reverse of [`TryFrom<CommodityInfo>`]. Kept as a `From` impl now
    /// that bc-ipc depends on bc-models via the `models` feature.
    #[inline]
    fn from(c: &bc_models::Commodity) -> Self {
        Self::new(
            c.id().to_string(),
            c.code().to_owned(),
            c.symbol().map(ToOwned::to_owned),
            c.aliases().to_vec(),
            c.decimals(),
            c.is_iso(),
            c.symbol_after(),
        )
    }
}

#[cfg(feature = "models")]
impl TryFrom<CommodityInfo> for bc_models::Commodity {
    type Error = crate::BcError;

    /// Builds a [`bc_models::Commodity`] from an IPC [`CommodityInfo`]. A blank
    /// id yields a fresh commodity (create); a populated id round-trips
    /// (update).
    #[inline]
    fn try_from(info: CommodityInfo) -> Result<Self, Self::Error> {
        let id =
            if info.id.is_empty() {
                None
            } else {
                Some(info.id.parse::<bc_models::CommodityId>().map_err(|e| {
                    crate::BcError::Validation(format!("invalid commodity id: {e}"))
                })?)
            };
        Ok(bc_models::Commodity::builder()
            .code(info.code)
            .aliases(info.aliases)
            .decimals(info.decimals)
            .is_iso(info.is_iso)
            .symbol_after(info.symbol_after)
            .maybe_symbol(info.symbol)
            .maybe_id(id)
            .build())
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

#[cfg(test)]
#[cfg(feature = "models")]
mod models_tests {
    use pretty_assertions::assert_eq;

    use super::CommodityInfo;

    #[test]
    fn try_from_valid_id_round_trips() {
        let id = bc_models::CommodityId::new();
        let info = CommodityInfo::new(
            id.to_string(),
            "AUD",
            Some("A$".to_owned()),
            vec![],
            2,
            true,
            false,
        );
        let commodity = bc_models::Commodity::try_from(info).expect("valid id parses");
        assert_eq!(commodity.id(), &id);
        assert_eq!(commodity.code(), "AUD");
    }

    #[test]
    fn try_from_blank_id_creates_new() {
        let info = CommodityInfo::new(String::new(), "USD", None, vec![], 2, true, false);
        let commodity = bc_models::Commodity::try_from(info).expect("blank id creates");
        assert_eq!(commodity.code(), "USD");
    }

    #[test]
    fn try_from_invalid_id_errors() {
        let info = CommodityInfo::new("not-an-id", "USD", None, vec![], 2, true, false);
        let err = bc_models::Commodity::try_from(info).expect_err("invalid id must fail");
        assert!(matches!(err, crate::BcError::Validation(_)));
    }

    #[test]
    fn commodity_round_trips_all_fields() {
        let original = bc_models::Commodity::builder()
            .code("BTC")
            .symbol("₿")
            .aliases(vec!["XBT".to_owned(), "BTC".to_owned()])
            .decimals(8)
            .is_iso(false)
            .symbol_after(true)
            .build();

        let info = CommodityInfo::from(&original);
        assert_eq!(info.id, original.id().to_string());
        assert_eq!(info.code, "BTC");
        assert_eq!(info.symbol.as_deref(), Some("₿"));
        assert_eq!(info.aliases, vec!["XBT".to_owned(), "BTC".to_owned()]);
        assert_eq!(info.decimals, 8);
        assert!(!info.is_iso);
        assert!(info.symbol_after);

        let back = bc_models::Commodity::try_from(info).expect("round-trips back to model");
        assert_eq!(back.id(), original.id());
        assert_eq!(back.code(), original.code());
        assert_eq!(back.symbol(), original.symbol());
        assert_eq!(back.aliases(), original.aliases());
        assert_eq!(back.decimals(), original.decimals());
        assert_eq!(back.is_iso(), original.is_iso());
        assert_eq!(back.symbol_after(), original.symbol_after());
    }
}
