//! End-to-end proof that Beancount `open` and `close` directives cross the
//! real `wasip2` component boundary, in source order, with their currencies.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use bc_core::Directive;
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

/// Declarations and the transaction between them reach the host as one
/// ordered list, with the `open` currency intact.
#[test]
fn beancount_declarations_reach_the_host() {
    let root = fixture(
        "declarations",
        &[(
            "ledger/main.bean",
            "2026-01-01 open Assets:Bank:Checking AUD\n\
             2026-01-15 * \"Generic Store\"\n  \
             Expenses:Food  5.00 AUD\n  \
             Assets:Bank:Checking\n\
             2026-06-30 close Assets:Bank:Checking\n",
        )],
    );

    let importer = load_beancount_importer(&root);
    let config = ImportConfig::from_value(serde_json::json!({ "source_file": "ledger/main.bean" }));
    let directives = importer
        .import(&config)
        .expect("a ledger with declarations imports");

    let [
        Directive::Open(open),
        Directive::Transaction(_),
        Directive::Close(close),
    ] = directives.as_slice()
    else {
        panic!("expected [Open, Transaction, Close], got {directives:?}");
    };
    assert_eq!(open.account, "Assets:Bank:Checking");
    assert_eq!(open.date, jiff::civil::date(2026, 1, 1));
    assert_eq!(open.commodities, vec!["AUD".to_owned()]);
    assert_eq!(close.account, "Assets:Bank:Checking");
    assert_eq!(close.date, jiff::civil::date(2026, 6, 30));

    drop(fs::remove_dir_all(&root));
}
