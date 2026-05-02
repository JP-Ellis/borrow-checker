//! Integration tests for bc-plugins loading WASM plugins.

use std::env;
use std::path::PathBuf;

use bc_core::ImportConfig;
use bc_core::ImporterRegistry;
use bc_plugins::PluginRegistry;

fn get_plugin_dir() -> PathBuf {
    if let Ok(val) = env::var("BORROW_CHECKER_PLUGIN_DIR") {
        PathBuf::from(val)
    } else {
        // Fallback to workspace root -> target/plugins
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.pop(); // pop bc-plugins
        path.pop(); // pop crates
        path.join("target").join("plugins")
    }
}

fn load_registry() -> ImporterRegistry {
    let plugin_dir = get_plugin_dir();
    if !plugin_dir.exists() {
        panic!(
            "Plugin directory does not exist: {}. Please run `cargo xtask build-plugins` first.",
            plugin_dir.display()
        );
    }

    let registry = PluginRegistry::load(&[plugin_dir]).expect("Failed to load plugin registry");
    registry.into_importer_registry()
}

#[test]
fn test_csv_plugin_detect_and_import() {
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
    let value: serde_json::Value = serde_json::from_str(config_json).unwrap();
    let config = ImportConfig::from_value(value);

    let txns = importer
        .import(csv_content, &config)
        .expect("Import failed");
    assert_eq!(txns.len(), 1);
    assert_eq!(txns[0].description, "Test");
}

#[test]
fn test_ledger_plugin_detect() {
    let registry = load_registry();
    let importer = registry
        .create_for_name("ledger")
        .expect("Ledger plugin not found in registry");

    assert_eq!(importer.name(), "ledger");

    let ledger_content = b"2025-01-01 * Grocery\n  Expenses:Food  10.00 AUD\n  Assets:Cash";
    assert!(importer.detect(ledger_content));
}

#[test]
fn test_beancount_plugin_detect() {
    let registry = load_registry();
    let importer = registry
        .create_for_name("beancount")
        .expect("Beancount plugin not found in registry");

    assert_eq!(importer.name(), "beancount");

    let beancount_content = b"2025-01-01 * \"Grocery\"\n  Expenses:Food  10.00 AUD\n  Assets:Cash";
    assert!(importer.detect(beancount_content));
}

#[test]
fn test_ofx_plugin_detect() {
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
fn test_malformed_input_is_handled_gracefully() {
    let registry = load_registry();
    let importer = registry.create_for_name("csv").unwrap();

    // detect() should safely return false on binary garbage
    let garbage = b"\x00\xFF\xFE\x00BinaryGarbage";
    assert!(!importer.detect(garbage));

    let config = ImportConfig::default();
    let result = importer.import(garbage, &config);
    // import() should return a proper ImportError
    assert!(result.is_err());
}
