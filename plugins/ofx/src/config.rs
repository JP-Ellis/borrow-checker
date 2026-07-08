//! Configuration types for the OFX importer.

/// Full configuration for the OFX importer.
///
/// Construct using [`Config::default()`] and then set individual fields,
/// or deserialize from JSON.
#[non_exhaustive]
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Account path this statement's transactions post to, e.g. `"Assets:NAB:Josh"`.
    /// Stamped onto every emitted posting.
    pub account: String,
    /// Statement file to import, relative to the host-preopened documents root.
    pub source_file: String,
}

#[cfg(test)]
mod tests {
    use bc_sdk::ImportConfig;

    use super::*;

    #[test]
    fn missing_account_fails_to_deserialize() {
        let cfg = ImportConfig::from_json_string("{}".to_owned());
        let result: Result<Config, _> = cfg.as_typed();
        assert!(result.is_err());
    }
}
