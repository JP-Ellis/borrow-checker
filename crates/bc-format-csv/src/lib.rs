//! CSV import for BorrowChecker.
//!
//! Implements the [`bc_core::Importer`] trait for delimited text files with
//! configurable column mapping. Supports bank-style CSV exports that contain
//! metadata preamble rows before the header.

// `pub use` re-exports are intentional: they lift key types to the crate root
// so that downstream users only need to import from `bc_format_csv`.
#![expect(
    clippy::pub_use,
    reason = "re-exporting key types at the crate root so users only need bc_format_csv as an import path"
)]

pub mod config;
pub use config::AmountColumns;
pub use config::CsvConfig;
pub use config::Preamble;

pub(crate) mod preamble;

pub mod importer;
pub use importer::CsvImporter;

/// Detects whether `bytes` look like a CSV/delimited-text file.
///
/// This is the static format-level detection function for use in an
/// [`ImporterFactory`](bc_core::ImporterFactory). It delegates to
/// [`CsvImporter::detect`] with a default (unconfigured) instance.
#[inline]
#[must_use]
pub fn detect_format(bytes: &[u8]) -> bool {
    use bc_core::Importer as _;
    CsvImporter::default().detect(bytes)
}

/// Creates a new [`CsvImporter`] boxed as a [`bc_core::Importer`] trait object.
#[inline]
#[must_use]
pub fn create_importer() -> Box<dyn bc_core::Importer> {
    Box::new(CsvImporter::new())
}

/// Returns an [`ImporterFactory`](bc_core::ImporterFactory) for the CSV format.
#[inline]
#[must_use]
pub fn importer_factory() -> bc_core::ImporterFactory {
    bc_core::ImporterFactory::new("csv", detect_format, create_importer)
}

#[cfg(test)]
mod factory_tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn importer_factory_has_correct_name() {
        assert_eq!(crate::importer_factory().name(), "csv");
    }

    #[test]
    fn importer_factory_detects_csv_bytes() {
        assert!(crate::importer_factory().detect(b"Date,Amount,Description\n"));
    }

    #[test]
    fn importer_factory_rejects_binary() {
        assert!(!crate::importer_factory().detect(b"\x89PNG\r\n"));
    }

    #[test]
    fn importer_factory_creates_working_importer() {
        let imp = crate::importer_factory().create();
        assert_eq!(imp.name(), "csv");
    }
}
