//! Configuration for the Beancount importer.

/// Beancount importer configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Ledger file to import, relative to the host-preopened documents root.
    pub source_file: String,
    /// Renames applied to every commodity code the importer emits, each at
    /// its directive's date.
    #[serde(default)]
    pub commodity_aliases: Vec<bc_sdk::CommodityAlias>,
}
