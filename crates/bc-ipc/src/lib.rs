//! Shared IPC types for the BorrowChecker Tauri application.
//!
//! This crate is the only channel between the native Tauri backend (`bc-app`)
//! and the WASM frontend (`bc-ui`). It must compile to both
//! `wasm32-unknown-unknown` and native targets — zero native-only dependencies.
//!
//! # Type conventions
//!
//! - Monetary amounts use `i64` cents — never `f64`
//! - IDs use `String` — newtype IDs serialise to their string representation
//! - All public enums carry `#[non_exhaustive]` for forward compatibility
//! - All types implement `Send + Sync`, `Serialize`, `Deserialize`, `Clone`,
//!   `Debug`

/// Serialisable error types returned by Tauri commands.
pub mod error;

pub use error::BcError;
