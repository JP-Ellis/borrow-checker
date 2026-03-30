#![expect(
    clippy::pub_use,
    reason = "re-exporting key types at the crate root so users only need bc_format_ofx as an import path"
)]
//! OFX/QFX import for BorrowChecker.
//!
//! Implements [`bc_core::Importer`] for OFX v1 (SGML) and OFX v2 (XML)
//! bank statement files.
//!
//! The main entry point is [`OfxImporter`].

pub(crate) mod ast;
pub(crate) mod importer;
pub(crate) mod parser;
pub(crate) mod sgml;

pub use importer::Importer as OfxImporter;

/// Detects whether `bytes` look like an OFX or QFX file.
///
/// Delegates to [`OfxImporter::detect`] with a default instance.
#[inline]
#[must_use]
pub fn detect_format(bytes: &[u8]) -> bool {
    use bc_core::Importer as _;
    OfxImporter::default().detect(bytes)
}

/// Creates a new [`OfxImporter`] boxed as a [`bc_core::Importer`] trait object.
#[inline]
#[must_use]
pub fn create_importer() -> Box<dyn bc_core::Importer> {
    Box::new(OfxImporter::new())
}

/// Returns an [`ImporterFactory`](bc_core::ImporterFactory) for the OFX/QFX format.
#[inline]
#[must_use]
pub fn importer_factory() -> bc_core::ImporterFactory {
    bc_core::ImporterFactory::new("ofx", detect_format, create_importer)
}

#[cfg(test)]
mod factory_tests {
    use pretty_assertions::assert_eq;

    #[test]
    fn importer_factory_has_correct_name() {
        assert_eq!(crate::importer_factory().name(), "ofx");
    }

    #[test]
    fn importer_factory_detects_ofx_bytes() {
        assert!(crate::importer_factory().detect(b"OFXHEADER:100\nDATA:OFXSGML\n"));
    }

    #[test]
    fn importer_factory_rejects_csv() {
        assert!(!crate::importer_factory().detect(b"Date,Amount,Description\n"));
    }

    #[test]
    fn importer_factory_creates_working_importer() {
        let imp = crate::importer_factory().create();
        assert_eq!(imp.name(), "ofx");
    }
}
