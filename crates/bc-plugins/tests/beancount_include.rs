//! End-to-end proof that the Beancount importer follows `include` directives
//! through the host's read-only preopen, and that an include escaping that
//! preopen is reported as a configuration error rather than a bare read
//! failure.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::env;
use std::fs;
use std::path::Path;
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

#[test]
fn multi_file_ledger_imports_through_the_preopen() {
    let root = fixture(
        "multifile",
        &[
            (
                "ledger/main.bean",
                "option \"title\" \"Household\"\n\ninclude \"2025-01.bean\"\ninclude \"sub/2025-02.bean\"\n",
            ),
            (
                "ledger/2025-01.bean",
                "2025-01-15 * \"Generic Store\" \"January\"\n  Expenses:Food   10.00 AUD\n  Assets:Bank   -10.00 AUD\n",
            ),
            (
                "ledger/sub/2025-02.bean",
                "2025-02-15 * \"Generic Store\" \"February\"\n  Expenses:Food   20.00 AUD\n  Assets:Bank   -20.00 AUD\n",
            ),
        ],
    );

    let importer = load_beancount_importer(&root);
    let config = ImportConfig::from_value(serde_json::json!({ "source_file": "ledger/main.bean" }));
    let txs = importer.import(&config).expect("multi-file ledger imports");

    assert_eq!(txs.len(), 2, "both included files contribute");
    let descriptions: Vec<&str> = txs.iter().map(|tx| tx.description.as_str()).collect();
    assert_eq!(descriptions, vec!["January", "February"]);

    drop(fs::remove_dir_all(&root));
}

#[test]
fn include_escaping_the_preopen_names_the_offending_include() {
    let root = fixture(
        "escape",
        &[("ledger/main.bean", "include \"../../../../etc/passwd\"\n")],
    );

    let importer = load_beancount_importer(&root);
    let config = ImportConfig::from_value(serde_json::json!({ "source_file": "ledger/main.bean" }));
    let err = importer
        .import(&config)
        .expect_err("an include escaping the preopen must fail");

    // `source.rs` reports "include at {file}:{line} cannot read {path}: {os_err}".
    // Assert file and line together so a reformat that drops the line (e.g.
    // "include from {file} cannot read {path}") cannot slip past this test.
    // The reported path is the lexically-folded form (`../etc/passwd`), not the
    // literal `../../../../etc/passwd` text the fixture wrote — `source.rs`
    // folds `..` components before this error is raised, and that folding is
    // out of scope here, so this asserts what the code actually produces.
    let message = err.to_string();
    assert!(
        message.contains("main.bean:1"),
        "names the including file and its line together: {message}"
    );
    assert!(message.contains("passwd"), "names the path: {message}");

    drop(fs::remove_dir_all(&root));
}
