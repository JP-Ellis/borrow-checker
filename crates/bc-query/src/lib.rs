//! The query language for transaction search.
//!
//! [`parse`] turns query text into an [`Expr`], [`print()`] writes it back as
//! canonical text, [`resolve()`] types each term against a [`Catalog`], and
//! [`parse_partial`] tells autocomplete what the cursor sits in. The crate
//! never depends on `bc-models`, so the palette (WASM) and the server run the
//! same parser.
#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

pub mod ast;
pub mod currency;
mod parser;
mod span;

pub use ast::Expr;
pub use parser::ParseError;
pub use parser::parse;
pub use span::Span;
