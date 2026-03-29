#![expect(
    clippy::pub_use,
    reason = "re-exporting key types at the crate root so users only need bc_format_ledger as an import path"
)]
//! Ledger file read/write for BorrowChecker.
//!
//! Implements [`bc_core::Importer`] and [`bc_core::Exporter`] for the
//! [Ledger](https://ledger-cli.org/) plain-text accounting format.

pub(crate) mod ast;
pub mod exporter;
pub mod importer;
pub(crate) mod parser;
pub(crate) mod writer;

pub use exporter::Exporter as LedgerExporter;
pub use importer::Importer as LedgerImporter;

/// Detects whether `bytes` look like a Ledger plain-text accounting file.
///
/// Delegates to [`LedgerImporter::detect`] with a default instance.
#[inline]
#[must_use]
pub fn detect_format(bytes: &[u8]) -> bool {
    use bc_core::Importer as _;
    LedgerImporter::default().detect(bytes)
}

/// Creates a new [`LedgerImporter`] boxed as a [`bc_core::Importer`] trait object.
#[inline]
#[must_use]
pub fn create_importer() -> Box<dyn bc_core::Importer> {
    Box::new(LedgerImporter::new())
}

/// Returns an [`ImporterFactory`](bc_core::ImporterFactory) for the Ledger format.
#[inline]
#[must_use]
pub fn importer_factory() -> bc_core::ImporterFactory {
    bc_core::ImporterFactory::new("ledger", detect_format, create_importer)
}

#[cfg(test)]
mod factory_tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn importer_factory_has_correct_name() {
        assert_eq!(crate::importer_factory().name(), "ledger");
    }

    #[test]
    fn importer_factory_detects_ledger_bytes() {
        // Classic Ledger format: unquoted payee after flag
        assert!(
            crate::importer_factory()
                .detect(b"2025-01-15 * Woolworths\n    Assets:Bank   -50.00 AUD\n")
        );
    }

    #[test]
    fn importer_factory_rejects_beancount() {
        // Beancount uses quoted payees — Ledger importer must reject these
        assert!(
            !crate::importer_factory()
                .detect(b"2025-01-15 * \"Woolworths\" \"Groceries\"\n  Assets:Bank   50.00 AUD\n")
        );
    }

    #[test]
    fn importer_factory_creates_working_importer() {
        let imp = crate::importer_factory().create();
        assert_eq!(imp.name(), "ledger");
    }
}
