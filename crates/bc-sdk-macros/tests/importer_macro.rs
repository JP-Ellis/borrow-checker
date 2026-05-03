//! Integration tests for the `#[bc_sdk::importer]` proc-macro.
//!
//! Verifies that the macro generates correct WIT export glue by compiling
//! against `bc-sdk` and exercising the `bc_sdk::Importer` trait.
//! Tests run on native targets (not WASM) — the generated `export!()` call is
//! a no-op outside WASM.
//!
//! Only one `#[importer]`-annotated type may appear per test binary because the
//! macro emits crate-level WASM export symbols that would conflict otherwise.

#![expect(
    clippy::tests_outside_test_module,
    reason = "integration test file — tests/ directory is implicitly cfg(test)"
)]

use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::Importer;
use bc_sdk::RawTransaction;
use pretty_assertions::assert_eq;
use pretty_assertions::assert_str_eq;

/// Minimal importer that rejects everything and always returns an empty list.
#[derive(Default)]
struct NullImporter;

#[bc_sdk::importer]
impl Importer for NullImporter {
    fn name(&self) -> &'static str {
        "null"
    }

    fn detect(&self, _bytes: &[u8]) -> bool {
        false
    }

    fn import(
        &self,
        _bytes: &[u8],
        _config: ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError> {
        Ok(vec![])
    }
}

#[test]
fn name_forwarded() {
    assert_str_eq!(NullImporter.name(), "null");
}

#[test]
fn detect_always_false() {
    assert!(!NullImporter.detect(b""));
    assert!(!NullImporter.detect(b"anything"));
}

#[test]
fn import_returns_empty_vec() {
    let result = NullImporter.import(b"", ImportConfig::default());
    let txns = result.expect("import of empty input should succeed");
    assert_eq!(txns, vec![]);
}

#[test]
fn import_error_types_roundtrip() {
    let cases: &[ImportError] = &[
        ImportError::InvalidConfig("bad cfg".to_owned()),
        ImportError::Parse("bad data".to_owned()),
        ImportError::MissingField("date".to_owned()),
        ImportError::BadValue {
            field: "amount".to_owned(),
            detail: "not a number".to_owned(),
        },
    ];
    for err in cases {
        let msg = format!("{err}");
        assert!(
            !msg.is_empty(),
            "error message should not be empty: {err:?}"
        );
    }
}
