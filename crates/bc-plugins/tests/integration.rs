//! Integration tests for bc-plugins loading WASM plugins.

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use bc_core::ImportConfig;
    use bc_core::ImporterRegistry;
    use bc_plugins::PluginRegistry;
    use pretty_assertions::assert_eq;

    fn get_plugin_dir() -> PathBuf {
        if let Ok(val) = env::var("BORROW_CHECKER_PLUGIN_DIR") {
            PathBuf::from(val)
        } else {
            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.pop(); // pop bc-plugins
            path.pop(); // pop crates
            path.join("target").join("plugins")
        }
    }

    fn load_registry() -> ImporterRegistry {
        let plugin_dir = get_plugin_dir();
        assert!(
            plugin_dir.exists(),
            "Plugin directory does not exist: {}. Please run `mise run build-plugins` first.",
            plugin_dir.display()
        );
        let registry = PluginRegistry::load(&[plugin_dir]).expect("Failed to load plugin registry");
        registry.into_importer_registry()
    }

    #[test]
    fn csv_plugin_detect_and_import() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("csv")
            .expect("CSV plugin not found in registry");

        assert_eq!(importer.name(), "csv");

        let csv_content = b"Date,Amount,Description\n2025-01-01,10.0,Test";
        assert!(importer.detect(csv_content));

        let config_json = r#"{
            "commodity": "AUD",
            "date_column": "Date",
            "date_format": "%Y-%m-%d",
            "amount_columns": {"style": "single", "column": "Amount"},
            "description_column": "Description"
        }"#;
        let value: serde_json::Value =
            serde_json::from_str(config_json).expect("hardcoded JSON is valid");
        let config = ImportConfig::from_value(value);

        let txns = importer
            .import(csv_content, &config)
            .expect("Import failed");
        assert_eq!(txns.len(), 1);
        #[expect(
            clippy::indexing_slicing,
            reason = "test: length asserted on the line above"
        )]
        let first = &txns[0];
        assert_eq!(first.description, "Test");
    }

    #[test]
    fn ledger_plugin_detect() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("ledger")
            .expect("Ledger plugin not found in registry");

        assert_eq!(importer.name(), "ledger");

        let ledger_content = b"2025-01-01 * Grocery\n  Expenses:Food  10.00 AUD\n  Assets:Cash";
        assert!(importer.detect(ledger_content));
    }

    #[test]
    fn beancount_plugin_detect() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("beancount")
            .expect("Beancount plugin not found in registry");

        assert_eq!(importer.name(), "beancount");

        let beancount_content =
            b"2025-01-01 * \"Grocery\"\n  Expenses:Food  10.00 AUD\n  Assets:Cash";
        assert!(importer.detect(beancount_content));
    }

    #[test]
    fn ofx_plugin_detect() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("ofx")
            .expect("OFX plugin not found in registry");

        assert_eq!(importer.name(), "ofx");

        let ofx_content =
            b"OFXHEADER:100\nDATA:OFXSGML\n\n<OFX>\n<BANKMSGSRSV1>\n</BANKMSGSRSV1>\n</OFX>";
        assert!(importer.detect(ofx_content));
    }

    #[test]
    fn malformed_input_is_handled_gracefully() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("csv")
            .expect("CSV plugin not found");

        let garbage = b"\x00\xFF\xFE\x00BinaryGarbage";
        assert!(!importer.detect(garbage));

        let config = ImportConfig::default();
        importer
            .import(garbage, &config)
            .expect_err("import of binary garbage should return an error");
    }
}
