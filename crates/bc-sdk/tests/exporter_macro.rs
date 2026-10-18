//! Integration test for `#[bc_sdk::importer(exporter)]`.
//!
//! Lives in its own binary because the macro emits crate-level export symbols,
//! so only one annotated type may appear per test binary.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportError as WireExportError;
use bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportLevel as WireExportLevel;
use bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::ExportSnapshot as WireExportSnapshot;
use bc_sdk::__bindings::exports::borrow_checker::sdk::exporter::Guest;
use bc_sdk::ExportError;
use bc_sdk::ExportFile;
use bc_sdk::ExportLevel;
use bc_sdk::ExportSnapshot;
use bc_sdk::Exporter;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::ImportOutput;
use bc_sdk::Importer;
use pretty_assertions::assert_eq;

/// An exporter that writes one file naming how many transactions it saw, and
/// refuses an archive snapshot.
#[derive(Default)]
struct CountingExporter;

#[bc_sdk::importer(exporter)]
impl Importer for CountingExporter {
    fn name(&self) -> &'static str {
        "counting"
    }

    fn import(&self, _config: ImportConfig) -> Result<ImportOutput, ImportError> {
        Ok(ImportOutput::default())
    }

    fn validate(&self, _config: ImportConfig) -> Result<(), ImportError> {
        Ok(())
    }
}

impl Exporter for CountingExporter {
    fn export(&self, snapshot: ExportSnapshot) -> Result<Vec<ExportFile>, ExportError> {
        if snapshot.level == ExportLevel::Archive {
            return Err(ExportError::Unspellable(vec!["archive".to_owned()]));
        }
        Ok(vec![
            ExportFile::builder()
                .path("count.txt")
                .contents(snapshot.transactions.len().to_string())
                .build(),
        ])
    }
}

fn wire_snapshot(level: WireExportLevel) -> WireExportSnapshot {
    WireExportSnapshot {
        level,
        accounts: vec![],
        commodities: vec![],
        transactions: vec![],
        budgets: vec![],
        balances: vec![],
        narrowed: false,
    }
}

/// The glue must reach the trait impl and carry its files back.
#[test]
fn exporter_flag_delegates_to_the_trait_impl() {
    let files = <__BcImporterExport as Guest>::export(wire_snapshot(WireExportLevel::Working))
        .expect("working export succeeds");
    let [file] = files.as_slice() else {
        panic!("expected one file, got {files:?}");
    };
    assert_eq!(file.path, "count.txt");
    assert_eq!(file.contents, "0");
}

/// The glue must carry the trait's error across, not replace it.
#[test]
fn exporter_errors_cross_the_wire() {
    let err = <__BcImporterExport as Guest>::export(wire_snapshot(WireExportLevel::Archive))
        .expect_err("archive export is refused");
    assert!(
        matches!(&err, WireExportError::Unspellable(items) if items == &["archive"]),
        "got {err:?}"
    );
}
