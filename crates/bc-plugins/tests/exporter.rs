//! End-to-end: the host reaches a plugin's `exporter` interface and can
//! re-root a plugin onto another documents directory.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use std::env;
use std::fs;
use std::path::PathBuf;

use bc_core::Directive;
use bc_core::ExportError;
use bc_core::ExportSnapshot;
use bc_core::Exporter as _;
use bc_core::ImportConfig;
use bc_core::Importer as _;
use bc_plugins::PluginRegistry;

/// Returns the directory containing compiled plugin WASM artifacts.
///
/// **Prerequisite:** run `mise run build-plugins` first, or set
/// `BORROW_CHECKER_PLUGIN_DIR`.
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

/// Loads every plugin with `documents_root` preopened, as a caller would.
#[expect(clippy::expect_used, reason = "test helper panics on setup failure")]
fn load(documents_root: &std::path::Path) -> PluginRegistry {
    let plugin_dir = get_plugin_dir();
    assert!(
        plugin_dir.exists(),
        "Plugin directory does not exist: {}. Please run `mise run build-plugins` first.",
        plugin_dir.display()
    );
    PluginRegistry::load(&[plugin_dir], Some(documents_root)).expect("Failed to load registry")
}

/// A plugin that implements no export answers `Unsupported` through the host.
#[test]
fn csv_plugin_reports_export_unsupported() {
    let root = env::temp_dir().join("bc-plugins-export-unsupported");
    drop(fs::remove_dir_all(&root));
    fs::create_dir_all(&root).expect("mkdir");
    let registry = load(&root);

    let exporter = registry.exporter("csv").expect("csv plugin is loaded");
    let result = exporter.export(&ExportSnapshot::default());

    assert!(
        matches!(result, Err(ExportError::Unsupported)),
        "got {result:?}"
    );
    assert!(registry.exporter("no-such-plugin").is_none());

    drop(fs::remove_dir_all(&root));
}

/// A re-rooted plugin reads a file the registry's own root does not contain.
#[test]
fn with_documents_root_reads_outside_the_configured_root() {
    let configured = env::temp_dir().join("bc-plugins-export-configured-root");
    let elsewhere = env::temp_dir().join("bc-plugins-export-elsewhere-root");
    drop(fs::remove_dir_all(&configured));
    drop(fs::remove_dir_all(&elsewhere));
    fs::create_dir_all(&configured).expect("mkdir");
    fs::create_dir_all(&elsewhere).expect("mkdir");
    fs::write(
        elsewhere.join("main.bean"),
        "2026-01-01 open Assets:Bank:123456789 AUD\n",
    )
    .expect("write");
    let registry = load(&configured);
    let plugin = registry.exporter("beancount").expect("beancount is loaded");
    let config = ImportConfig::from_value(serde_json::json!({ "source_file": "main.bean" }));

    let direct = plugin.import(&config);
    assert!(
        !matches!(direct, Ok(ref out) if !out.directives.is_empty()),
        "the configured root must not contain the file"
    );

    let rerooted = plugin.with_documents_root(elsewhere.clone());
    let output = rerooted.import(&config).expect("imports from the new root");

    assert!(
        matches!(output.directives.as_slice(), [Directive::Open(open)] if open.account == "Assets:Bank:123456789"),
        "got {:?}",
        output.directives
    );

    drop(fs::remove_dir_all(&configured));
    drop(fs::remove_dir_all(&elsewhere));
}
