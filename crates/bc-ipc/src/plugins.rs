//! Plugin IPC types — describes a loaded WASM importer plugin.

use serde::Deserialize;
use serde::Serialize;

/// Metadata for a single installed plugin, returned by the `list_plugins` command.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Human-readable plugin name as reported by the WASM component.
    pub name: String,
    /// Integer ABI version the plugin was compiled against.
    pub sdk_abi: u32,
    /// Path to the `.wasm` source file (full filesystem path).
    pub source_path: String,
}

impl PluginInfo {
    /// Constructs a new [`PluginInfo`].
    ///
    /// # Arguments
    ///
    /// * `name` - The plugin name queried from the WASM component.
    /// * `sdk_abi` - The ABI version the plugin was compiled against.
    /// * `source_path` - The filesystem path to the `.wasm` file.
    ///
    /// # Returns
    ///
    /// A new [`PluginInfo`] with the given metadata.
    #[inline]
    #[must_use]
    pub fn new(name: String, sdk_abi: u32, source_path: String) -> Self {
        Self {
            name,
            sdk_abi,
            source_path,
        }
    }
}
