//! WASM host runtime and plugin ABI bridge for BorrowChecker.
//!
//! Loads `.wasm` plugin files via the WASM Component Model (wasmtime),
//! probes their metadata via exported `name()` / `sdk_abi()` functions,
//! and bridges them into `bc-core`'s
//! [`ImporterRegistry`](bc_core::ImporterRegistry).

pub(crate) mod host;
pub(crate) mod plugin_importer;
pub(crate) mod registry;
pub(crate) mod translate;

/// Lightweight metadata queried from a plugin WASM component.
///
/// Returned by [`query_metadata`] for use by install/validation workflows
/// that need plugin identity without loading a full registry.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct PluginMetadata {
    /// The stable plugin name (e.g. `"csv"`).
    pub name: String,
    /// The SDK ABI version the plugin was compiled against.
    pub sdk_abi: u32,
}

pub use registry::PluginRegistry;
pub use registry::RegistryError;
pub use registry::query_metadata;
