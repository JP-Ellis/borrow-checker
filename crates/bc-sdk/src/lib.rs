//! BorrowChecker Plugin SDK.
//!
//! Plugin authors depend on this crate, compile to `wasm32-wasip2`,
//! and distribute a single `.wasm` file.
//!
//! This crate has no dependencies on the rest of the BorrowChecker workspace.
//!
//! # Quick Start
//!
//! ```rust,ignore
//! use bc_sdk::{ImportConfig, ImportError, Importer, RawTransaction};
//!
//! struct MyImporter;
//!
//! #[bc_sdk::importer]
//! impl Importer for MyImporter {
//!     fn name(&self) -> &str { "my-format" }
//!
//!     fn detect(&self, bytes: &[u8]) -> bool {
//!         bytes.starts_with(b"MY")
//!     }
//!
//!     fn import(
//!         &self,
//!         bytes: &[u8],
//!         config: ImportConfig,
//!     ) -> Result<Vec<RawTransaction>, ImportError> {
//!         Ok(vec![])
//!     }
//! }
//! ```

// Generate all guest WIT bindings from the wit/ directory.
// This module is re-exported as `__bindings` for use by bc-sdk-macros.
#[doc(hidden)]
#[allow(
    warnings,
    clippy::all,
    reason = "generated code from wit-bindgen may not conform to workspace lint rules"
)]
pub mod __bindings {
    wit_bindgen::generate!({
        path: "wit",
        world: "importer-plugin",
    });
}

pub mod types;
pub use types::Amount;
pub use types::Date;
pub use types::ImportConfig;
pub use types::ImportError;
pub use types::RawTransaction;

/// The trait that every importer plugin must implement.
///
/// Apply `#[bc_sdk::importer]` to the `impl` block to generate the required
/// WASM export glue automatically.
///
/// # Requirements
///
/// The implementing type must also implement [`Default`] so the generated
/// export glue can instantiate it without arguments.
pub trait Importer: Default {
    /// A short, stable identifier for this importer (e.g. `"csv"`, `"ofx"`).
    fn name(&self) -> &str;

    /// Returns `true` if `bytes` look like input this importer can handle.
    ///
    /// Implementations must be fast and non-panicking.
    #[must_use]
    fn detect(&self, bytes: &[u8]) -> bool;

    /// Parses `bytes` into a list of [`RawTransaction`] values.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes.
    /// * `config` - Opaque JSON configuration from the import profile.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError`] on configuration, parse, or field errors.
    fn import(
        &self,
        bytes: &[u8],
        config: ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError>;
}
