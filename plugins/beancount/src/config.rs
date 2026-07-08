//! Configuration for the Beancount importer.

/// Beancount importer configuration.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Ledger file to import, relative to the host-preopened documents root.
    pub source_file: String,
}
