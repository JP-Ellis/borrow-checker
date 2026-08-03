//! Resolution of imported commodity codes against the registry.
//!
//! Mirrors [`crate::AccountResolver`]: one snapshot per import run, so
//! resolving thousands of legs costs no further queries.

use std::collections::HashMap;

use bc_models::Commodity;
use bc_models::CommodityCode;

use crate::BcResult;

/// Resolves a commodity code as an import states it to a registered commodity.
///
/// Matching mirrors `commodity::check_ambiguity` exactly: **codes match
/// case-insensitively; symbols and aliases match exactly**. The registry cannot
/// hold two commodities whose codes differ only in case — `create` rejects the
/// second as a marker conflict — so folding case on codes cannot merge two
/// distinct commodities. Aliases stay exact, leaving an explicitly
/// case-insensitive alias open as a future per-alias option.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct CommodityResolver {
    /// Upper-cased code → the registered spelling of that code.
    by_code: HashMap<String, String>,
    /// Symbol or alias, verbatim → the registered code it denotes.
    by_marker: HashMap<String, String>,
}

impl CommodityResolver {
    /// Loads every registered commodity into an in-memory resolution map.
    ///
    /// # Arguments
    ///
    /// * `commodities` - The commodity service to snapshot.
    ///
    /// # Returns
    ///
    /// A resolver over every registered commodity.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn load(commodities: &crate::CommodityService) -> BcResult<Self> {
        Ok(Self::from_commodities(&commodities.list_all().await?))
    }

    /// Builds a resolver from an already-loaded set of commodities.
    ///
    /// # Arguments
    ///
    /// * `all` - Every registered commodity.
    ///
    /// # Returns
    ///
    /// A resolver over `all`.
    #[must_use]
    #[inline]
    pub fn from_commodities(all: &[Commodity]) -> Self {
        let mut by_code = HashMap::with_capacity(all.len());
        let mut by_marker = HashMap::new();
        for c in all {
            by_code.insert(c.code().to_uppercase(), c.code().to_owned());
            if let Some(symbol) = c.symbol() {
                by_marker.insert(symbol.to_owned(), c.code().to_owned());
            }
            for alias in c.aliases() {
                by_marker.insert(alias.clone(), c.code().to_owned());
            }
        }
        Self { by_code, by_marker }
    }

    /// Resolves `code` to the registered spelling of the commodity it names.
    ///
    /// # Arguments
    ///
    /// * `code` - The code as the imported document states it.
    ///
    /// # Returns
    ///
    /// The registered code, or `None` when nothing matches.
    #[must_use]
    #[inline]
    pub fn resolve(&self, code: &CommodityCode) -> Option<&str> {
        let raw = code.as_str();
        if raw.is_empty() {
            return None;
        }
        self.by_code
            .get(&raw.to_uppercase())
            .or_else(|| self.by_marker.get(raw))
            .map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use bc_models::Commodity;
    use bc_models::CommodityCode;
    use pretty_assertions::assert_eq;

    use super::CommodityResolver;

    fn registry() -> Vec<Commodity> {
        vec![
            Commodity::builder()
                .code("AUD")
                .symbol("A$")
                .aliases(vec!["AU$".to_owned()])
                .decimals(2)
                .build(),
            Commodity::builder()
                .code("BTC")
                .symbol("B")
                .decimals(8)
                .is_iso(false)
                .build(),
        ]
    }

    #[test]
    fn resolves_an_exact_code() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("BTC")), Some("BTC"));
    }

    /// The registry cannot hold both `BTC` and `btc` — `create` rejects the
    /// second as a marker conflict — so folding case on *codes* loses nothing.
    #[test]
    fn resolves_a_code_case_insensitively_to_its_registered_spelling() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("btc")), Some("BTC"));
        assert_eq!(r.resolve(&CommodityCode::new("Btc")), Some("BTC"));
    }

    #[test]
    fn resolves_an_alias_to_the_canonical_code() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("AU$")), Some("AUD"));
    }

    #[test]
    fn resolves_a_symbol_to_the_canonical_code() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("A$")), Some("AUD"));
    }

    /// Aliases match exactly, mirroring `check_ambiguity`. Only codes fold case.
    #[test]
    fn does_not_resolve_an_alias_differing_in_case() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("au$")), None);
    }

    #[test]
    fn does_not_resolve_an_unregistered_code() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("DOGE")), None);
    }

    #[test]
    fn does_not_resolve_an_empty_code() {
        let r = CommodityResolver::from_commodities(&registry());
        assert_eq!(r.resolve(&CommodityCode::new("")), None);
    }
}
