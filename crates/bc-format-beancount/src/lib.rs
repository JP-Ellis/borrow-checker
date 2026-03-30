//! Beancount file read/write for BorrowChecker.
//!
//! Implements [`bc_core::Importer`] and [`bc_core::Exporter`] for the
//! [Beancount](https://beancount.github.io/) plain-text accounting format.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

pub(crate) mod ast;
pub(crate) mod exporter;
pub(crate) mod importer;
pub(crate) mod parser;
pub(crate) mod writer;

pub use exporter::Exporter as BeancountExporter;
pub use importer::Importer as BeancountImporter;

/// Detects whether `bytes` look like a Beancount plain-text accounting file.
///
/// Delegates to [`BeancountImporter::detect`] with a default instance.
#[inline]
#[must_use]
pub fn detect_format(bytes: &[u8]) -> bool {
    use bc_core::Importer as _;
    BeancountImporter::default().detect(bytes)
}

/// Creates a new [`BeancountImporter`] boxed as a [`bc_core::Importer`] trait object.
#[inline]
#[must_use]
pub fn create_importer() -> Box<dyn bc_core::Importer> {
    Box::new(BeancountImporter::new())
}

/// Returns an [`ImporterFactory`](bc_core::ImporterFactory) for the Beancount format.
#[inline]
#[must_use]
pub fn importer_factory() -> bc_core::ImporterFactory {
    bc_core::ImporterFactory::new("beancount", detect_format, create_importer)
}

#[cfg(test)]
mod factory_tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn importer_factory_has_correct_name() {
        assert_eq!(crate::importer_factory().name(), "beancount");
    }

    #[test]
    fn importer_factory_detects_beancount_bytes() {
        assert!(
            crate::importer_factory()
                .detect(b"2025-01-15 * \"Payee\" \"Narration\"\n  Assets:Bank   50.00 AUD\n")
        );
    }

    #[test]
    fn importer_factory_rejects_ledger() {
        assert!(
            !crate::importer_factory()
                .detect(b"2025-01-15 * Payee without quotes\n    Assets:Bank    50.00 AUD\n")
        );
    }

    #[test]
    fn importer_factory_creates_working_importer() {
        let imp = crate::importer_factory().create();
        assert_eq!(imp.name(), "beancount");
    }
}
