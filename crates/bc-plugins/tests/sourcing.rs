//! End-to-end: the CSV plugin sources its own files from a read-only preopen.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::env;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use bc_core::ImportConfig;
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

#[test]
fn csv_plugin_reads_files_from_preopened_root() {
    let root = env::temp_dir().join("bc-sourcing-e2e");
    let dir = root.join("Assets/NAB/Josh");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&dir).expect("mkdir");

    let header = "Date,Amount,Account Number,,Transaction Type,Transaction Details,Balance,Category,Merchant Name\n";
    let good_row = "27 Jun 25,-4321.00,123456789, ,TRANSFER DEBIT,SMARTBEAR,0.00,Transfers out,\n";
    let mut good = fs::File::create(dir.join("2025-06.csv")).expect("create good csv");
    good.write_all(format!("{header}{good_row}").as_bytes())
        .expect("write good csv");

    let bad_row = "not-a-date,not-a-number,123456789, ,TRANSFER DEBIT,BROKEN,0.00,Transfers out,\n";
    let mut bad = fs::File::create(dir.join("2025-07-broken.csv")).expect("create bad csv");
    bad.write_all(format!("{header}{bad_row}").as_bytes())
        .expect("write bad csv");

    let plugin_dir = get_plugin_dir();
    assert!(
        plugin_dir.exists(),
        "Plugin directory does not exist: {}. Please run `mise run build-plugins` first.",
        plugin_dir.display()
    );
    let registry = PluginRegistry::load(&[plugin_dir], Some(root.as_path()))
        .expect("Failed to load plugin registry");
    let importers = registry.into_importer_registry();
    let importer = importers
        .create_for_name("csv")
        .expect("CSV plugin not found in registry");

    let cfg = serde_json::json!({
        "account": "Assets:NAB:Josh",
        "source_dir": "Assets/NAB/Josh",
        "source_glob": "*.csv",
        "date_column": "Date",
        "date_format": "%d %b %y",
        "amount_columns": { "style": "single", "column": "Amount" },
        "description_column": "Transaction Details",
        "balance_column": "Balance",
        "commodity": "AUD"
    });
    let config = ImportConfig::from_value(cfg);
    let txs = importer.import(&config).expect("import");

    assert_eq!(txs.len(), 1);
    #[expect(
        clippy::indexing_slicing,
        reason = "test: length asserted on the line above"
    )]
    let first = &txs[0];
    assert_eq!(first.description, "SMARTBEAR");

    drop(fs::remove_dir_all(&root));
}
