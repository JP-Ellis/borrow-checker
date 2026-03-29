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
