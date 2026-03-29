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
