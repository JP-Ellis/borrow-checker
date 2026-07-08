//! Integration tests for bc-plugins loading WASM plugins.

#[cfg(test)]
mod tests {
    use std::env;
    use std::path::PathBuf;

    use bc_core::ImportConfig;
    use bc_core::ImporterRegistry;
    use bc_plugins::PluginRegistry;
    use pretty_assertions::assert_eq;

    /// Returns the directory containing compiled plugin WASM artifacts.
    ///
    /// **Prerequisite:** Plugin WASMs must be built before running these tests.
    /// Run `mise run build-plugins` (or `cargo xtask build-plugins`) from the
    /// workspace root to compile all plugin crates to `target/plugins/`.
    ///
    /// To point at a custom directory, set `BORROW_CHECKER_PLUGIN_DIR`.
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
        let registry =
            PluginRegistry::load(&[plugin_dir], None).expect("Failed to load plugin registry");
        registry.into_importer_registry()
    }

    // NOTE: these tests exercise real on-disk plugin `.wasm` files built by
    // `mise run build-plugins`. Until those plugins are rebuilt against the
    // `parse(config)` ABI (Tasks 3-5), the registry loads empty and every
    // `create_for_name` lookup below returns `None`. This is expected
    // transient breakage, not a regression — see the task-2 brief.

    #[test]
    fn csv_plugin_import() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("csv")
            .expect("CSV plugin not found in registry");

        assert_eq!(importer.name(), "csv");

        let config_json = r#"{
            "account": "Assets:Test",
            "commodity": "AUD",
            "date_column": "Date",
            "date_format": "%Y-%m-%d",
            "amount_columns": {"style": "single", "column": "Amount"},
            "description_column": "Description"
        }"#;
        let value: serde_json::Value =
            serde_json::from_str(config_json).expect("hardcoded JSON is valid");
        let config = ImportConfig::from_value(value);

        let txns = importer.import(&config).expect("Import failed");
        assert_eq!(txns.len(), 1);
        #[expect(
            clippy::indexing_slicing,
            reason = "test: length asserted on the line above"
        )]
        let first = &txns[0];
        assert_eq!(first.description, "Test");
        let posting = first.postings.first().expect("one posting emitted");
        assert_eq!(posting.account, "Assets:Test");
    }

    #[test]
    fn ledger_plugin_loads() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("ledger")
            .expect("Ledger plugin not found in registry");

        assert_eq!(importer.name(), "ledger");
    }

    #[test]
    fn beancount_plugin_loads() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("beancount")
            .expect("Beancount plugin not found in registry");

        assert_eq!(importer.name(), "beancount");
    }

    #[test]
    fn ofx_plugin_loads() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("ofx")
            .expect("OFX plugin not found in registry");

        assert_eq!(importer.name(), "ofx");
    }

    #[test]
    fn malformed_config_is_handled_gracefully() {
        let registry = load_registry();
        let importer = registry
            .create_for_name("csv")
            .expect("CSV plugin not found");

        let config = ImportConfig::default();
        importer
            .import(&config)
            .expect_err("import with an incomplete config should return an error");
    }
}
