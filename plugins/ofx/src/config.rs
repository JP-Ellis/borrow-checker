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
    #[serde(default)]
    pub account: String,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn default_config_has_empty_account() {
        let cfg = Config::default();
        assert_eq!(cfg.account, "");
    }
}
