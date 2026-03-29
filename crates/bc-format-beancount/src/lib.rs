//! Beancount file read/write for BorrowChecker.
//!
//! Implements [`bc_core::Importer`] and [`bc_core::Exporter`] for the
//! [Beancount](https://beancount.github.io/) plain-text accounting format.

#![expect(
    clippy::pub_use,
    reason = "re-exports are intentional for an ergonomic public API surface"
)]

pub(crate) mod ast;
pub mod exporter;
pub mod importer;
pub(crate) mod parser;
pub(crate) mod writer;

pub use exporter::BeancountExporter;
pub use importer::BeancountImporter;
