//! Integration test: verify #[importer] compiles on native targets.

use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::Importer;
use bc_sdk::RawTransaction;

#[derive(Default)]
struct NullImporter;

#[bc_sdk::importer]
impl Importer for NullImporter {
    fn name(&self) -> &'static str {
        "null"
    }

    fn detect(&self, _bytes: &[u8]) -> bool {
        false
    }

    fn import(
        &self,
        _bytes: &[u8],
        _config: ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError> {
        Ok(vec![])
    }
}

#[cfg(test)]
mod tests {
    use bc_sdk::Importer as _;
    use pretty_assertions::assert_eq;

    use super::NullImporter;

    #[test]
    fn null_importer_compiles_with_importer_macro() {
        let imp = NullImporter;
        assert_eq!(imp.name(), "null");
        assert!(!imp.detect(b"anything"));
    }
}
