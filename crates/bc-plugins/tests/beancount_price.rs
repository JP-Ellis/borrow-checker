//! End-to-end proof that a priced and a costed Beancount posting cross the
//! real `wasip2` component boundary intact.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use bc_core::ImportConfig;
use bc_models::Amount;
use bc_plugins::PluginRegistry;
use pretty_assertions::assert_eq;
use rust_decimal_macros::dec;

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

/// Loads the Beancount importer through the registry, as a caller normally
/// would, with `documents_root` preopened read-only.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn load_beancount_importer(documents_root: &Path) -> Box<dyn bc_core::Importer> {
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
        .create_for_name("beancount")
        .expect("beancount plugin not found in registry")
}

/// Writes `files` into a fresh directory unique to `test_name`.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn fixture(test_name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = env::temp_dir().join(format!("bc-plugins-bean-{test_name}"));
    drop(fs::remove_dir_all(&dir));
    for (rel, contents) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdir");
        }
        fs::write(&path, contents).expect("write");
    }
    dir
}

/// A priced and a costed leg cross the real `wasip2` component boundary in
/// their stated forms.
#[test]
#[expect(
    clippy::indexing_slicing,
    reason = "test code: panicking on wrong index is the desired behaviour"
)]
fn a_priced_ledger_reaches_the_host_with_its_annotations() {
    let root = fixture(
        "price",
        &[(
            "ledger/main.bean",
            "2026-06-01 * \"Sell 2 AAPL\"\n  \
             Assets:Shares  -2 AAPL {105 AUD, 2024-03-01, \"lot-a\"} @ 150 AUD\n  \
             Assets:Bank  290 AUD\n  \
             Income:Gains\n\
             2026-01-15 * \"Software\"\n  \
             Expenses:Software  4.00 USD @@ 6.37 AUD\n  \
             Assets:Bank  -6.37 AUD\n",
        )],
    );

    let importer = load_beancount_importer(&root);
    let config = ImportConfig::from_value(serde_json::json!({ "source_file": "ledger/main.bean" }));
    let txs = importer.import(&config).expect("a priced ledger imports");

    assert_eq!(txs.len(), 2);
    let shares = &txs[0].postings[0];
    assert_eq!(
        shares.cost,
        Some(
            bc_models::Cost::builder()
                .basis(bc_models::Quote::PerUnit(Amount::new(dec!(105), "AUD")))
                .date(jiff::civil::date(2024, 3, 1))
                .label("lot-a")
                .build()
        )
    );
    assert_eq!(
        shares.price,
        Some(bc_models::Quote::PerUnit(Amount::new(dec!(150), "AUD")))
    );
    assert_eq!(
        txs[1].postings[0].price,
        Some(bc_models::Quote::Total(Amount::new(dec!(6.37), "AUD")))
    );
    assert!(txs[1].postings[1].price.is_none());

    drop(fs::remove_dir_all(&root));
}
