#![expect(
    clippy::pub_use,
    reason = "re-exporting key types at the crate root so users only need bc_format_ofx as an import path"
)]
//! OFX/QFX import for BorrowChecker.
//!
//! Implements [`bc_core::Importer`] for OFX v1 (SGML) and OFX v2 (XML)
//! bank statement files.
//!
//! The main entry point is [`importer::OfxImporter`].

pub(crate) mod ast;
pub mod importer;
pub(crate) mod parser;
pub(crate) mod sgml;

pub use importer::OfxImporter;
