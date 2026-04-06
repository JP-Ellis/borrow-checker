//! Beancount file export for BorrowChecker.
//!
//! Implements [`bc_core::Exporter`] for the
//! [Beancount](https://beancount.github.io/) plain-text accounting format.

#![expect(
    clippy::pub_use,
    reason = "re-exporting key types at the crate root for ergonomic imports"
)]

pub(crate) mod exporter;
pub(crate) mod writer;

pub use exporter::Exporter as BeancountExporter;
