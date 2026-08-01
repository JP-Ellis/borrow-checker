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
    let dir = root.join("Assets/Bank/Checking");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&dir).expect("mkdir");

    let header = "Date,Amount,Account Number,,Transaction Type,Transaction Details,Balance,Category,Merchant Name\n";
    let good_row = "27 Jun 25,-4321.00,123456789, ,TRANSFER DEBIT,ACME,0.00,Transfers out,\n";
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
        "account": "Assets:Bank:Checking",
        "source_dir": "Assets/Bank/Checking",
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
    assert_eq!(first.description, "ACME");

    drop(fs::remove_dir_all(&root));
}

/// Loads the CSV plugin through the registry, as a caller normally would.
///
/// Returns a `Box<dyn Importer>` — the same handle the application gets, which
/// is why `validate` has to be a trait method.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn load_csv_importer(documents_root: &std::path::Path) -> Box<dyn bc_core::Importer> {
    let plugin_dir = get_plugin_dir();
    assert!(
        plugin_dir.exists(),
        "Plugin directory does not exist: {}. Please run `mise run build-plugins` first.",
        plugin_dir.display()
    );
    let registry = PluginRegistry::load(&[plugin_dir], Some(documents_root))
        .expect("Failed to load plugin registry");
    registry
        .into_importer_registry()
        .create_for_name("csv")
        .expect("CSV plugin not found in registry")
}

#[test]
fn validate_rejects_an_incoherent_csv_config_without_reading_files() {
    let root = env::temp_dir().join("bc-validate-incoherent-e2e");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).expect("mkdir");
    let importer = load_csv_importer(root.as_path());

    // `source_dir` deliberately does not exist: validate must not read files.
    let config = ImportConfig::from_value(serde_json::json!({
        "commodity": "AUD",
        "account": "Liabilities:Bank:Card",
        "source_dir": "does/not/exist",
        "source_glob": "*.csv",
        "header": {"kind": "absent"},
        "date_column": "Date",
        "amount_columns": {"style": "single", "column": 1_i32}
    }));

    let err = importer
        .validate(&config)
        .expect_err("a named column on a headerless file is incoherent");
    assert!(
        err.to_string().contains("date_column"),
        "error should name the offending field, got: {err}"
    );
}

#[test]
fn validate_accepts_a_coherent_csv_config() {
    let root = env::temp_dir().join("bc-validate-coherent-e2e");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).expect("mkdir");
    let importer = load_csv_importer(root.as_path());

    let config = ImportConfig::from_value(serde_json::json!({
        "commodity": "AUD",
        "account": "Liabilities:Bank:Card",
        "source_dir": "does/not/exist",
        "source_glob": "*.csv",
        "header": {"kind": "absent"},
        "date_column": 0_i32,
        "amount_columns": {"style": "single", "column": 1_i32}
    }));

    importer
        .validate(&config)
        .expect("an all-positional headerless config is coherent, and validate reads no files");
}

#[test]
fn import_errors_when_documents_root_unset() {
    let plugin_dir = get_plugin_dir();
    assert!(
        plugin_dir.exists(),
        "Plugin directory does not exist: {}. Please run `mise run build-plugins` first.",
        plugin_dir.display()
    );
    let registry =
        PluginRegistry::load(&[plugin_dir], None).expect("Failed to load plugin registry");
    let importers = registry.into_importer_registry();
    let importer = importers
        .create_for_name("csv")
        .expect("CSV plugin not found in registry");

    let config = ImportConfig::from_value(serde_json::json!({}));
    let err = importer.import(&config).expect_err("import should fail");

    assert!(
        matches!(err, bc_core::ImportError::MissingField(ref msg) if msg == "documents_root not configured"),
        "unexpected error: {err:?}"
    );
}
