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

use bc_sdk::__bindings::exports::borrow_checker::sdk::importer::Guest;
use bc_sdk::__bindings::exports::borrow_checker::sdk::importer::ImportError as WireImportError;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::Importer;
use bc_sdk::RawTransaction;
use pretty_assertions::assert_eq;
use pretty_assertions::assert_str_eq;

/// Sentinel returned by [`NullImporter::validate`].
///
/// Distinctive so that a test can prove the generated glue reached the trait
/// method rather than fabricating its own `Ok(())`.
const VALIDATE_SENTINEL: &str = "null importer rejects every config";

/// Minimal importer that imports nothing and rejects every configuration.
#[derive(Default)]
struct NullImporter;

#[bc_sdk::importer]
impl Importer for NullImporter {
    fn name(&self) -> &'static str {
        "null"
    }

    fn import(&self, _config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        Ok(vec![])
    }

    fn validate(&self, _config: ImportConfig) -> Result<(), ImportError> {
        Err(ImportError::InvalidConfig(VALIDATE_SENTINEL.to_owned()))
    }
}

#[test]
fn name_forwarded() {
    assert_str_eq!(NullImporter.name(), "null");
}

#[test]
fn import_returns_empty_vec() {
    let result = NullImporter.import(ImportConfig::default());
    let txns = result.expect("import of empty input should succeed");
    assert_eq!(txns, vec![]);
}

/// The generated `Guest::validate` must call through to the trait impl.
///
/// Asserting on the sentinel rather than on the shape of the generated tokens
/// is what makes this test fail if the glue is ever rewired to a hard-coded
/// `Ok(())`, which would silently disable validation for every plugin.
#[test]
fn generated_glue_delegates_validate_to_the_trait() {
    let err = <__BcImporterExport as Guest>::validate(String::from("{}"))
        .expect_err("NullImporter::validate rejects every config");

    match &err {
        WireImportError::InvalidConfig(detail) => assert_str_eq!(detail, VALIDATE_SENTINEL),
        WireImportError::Parse(_)
        | WireImportError::MissingField(_)
        | WireImportError::BadValue(_) => {
            panic!("expected InvalidConfig from the trait impl, got {err:?}")
        }
    }
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
