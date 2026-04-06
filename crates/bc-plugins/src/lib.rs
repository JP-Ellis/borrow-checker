//! WASM host runtime and plugin ABI bridge for BorrowChecker.
//!
//! Loads `.wasm` plugin files via the WASM Component Model (wasmtime),
//! validates their manifests, and bridges them into `bc-core`'s
//! [`ImporterRegistry`](bc_core::ImporterRegistry).

pub(crate) mod host;
pub(crate) mod manifest;
pub(crate) mod plugin_importer;
pub(crate) mod registry;
pub(crate) mod translate;

pub use manifest::ManifestError;
pub use manifest::PluginManifest;
pub use registry::PluginRegistry;
pub use registry::RegistryError;
