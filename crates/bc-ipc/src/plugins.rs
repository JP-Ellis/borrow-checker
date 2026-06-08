//! Plugin IPC types — describes a loaded WASM importer plugin.

use serde::Deserialize;
use serde::Serialize;

/// Metadata for a single installed plugin, returned by the `list_plugins` command.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginInfo {
    /// Human-readable plugin name as reported by the WASM component.
    pub name: String,
    /// Integer ABI version the plugin was compiled against.
    pub sdk_abi: u32,
    /// Basename of the `.wasm` source file (filename only, no directory component).
    ///
    /// Extracted on the native side before crossing the IPC boundary so that
    /// WASM-side path manipulation is not needed (WASM `Path` only understands
    /// `/` as a separator and would misparse Windows-style backslash paths).
    pub file_name: String,
    /// Whether the plugin was compiled against a deprecated ABI version.
    ///
    /// `true` when the plugin's `sdk_abi` falls in the deprecated-but-still-loaded
    /// range (between `HOST_ABI_DEPRECATED_MIN` and `HOST_ABI_MIN` in `bc-plugins`).
    /// The plugin still loads but support will be dropped at the next breaking ABI bump.
    pub is_deprecated: bool,
}

impl PluginInfo {
    /// Constructs a new [`PluginInfo`].
    ///
    /// # Arguments
    ///
    /// * `name` - The plugin name queried from the WASM component.
    /// * `sdk_abi` - The ABI version the plugin was compiled against.
    /// * `file_name` - The basename of the `.wasm` file (no directory component).
    /// * `is_deprecated` - Whether the plugin is in the deprecated ABI range.
    ///
    /// # Returns
    ///
    /// A new [`PluginInfo`] with the given metadata.
    #[inline]
    #[must_use]
    pub fn new(name: String, sdk_abi: u32, file_name: String, is_deprecated: bool) -> Self {
        Self {
            name,
            sdk_abi,
            file_name,
            is_deprecated,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn plugin_info_constructor() {
        let info = PluginInfo::new(
            "my-plugin".to_owned(),
            3,
            "my-plugin.wasm".to_owned(),
            false,
        );
        assert_eq!(info.name, "my-plugin");
        assert_eq!(info.sdk_abi, 3);
        assert_eq!(info.file_name, "my-plugin.wasm");
        assert!(!info.is_deprecated);
    }

    #[test]
    fn plugin_info_serde_roundtrip() {
        let info = PluginInfo::new(
            "csv-importer".to_owned(),
            2,
            "csv-importer.wasm".to_owned(),
            false,
        );
        let json = serde_json::to_string(&info).expect("serialises");
        let info2: PluginInfo = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(info, info2);
    }

    #[test]
    fn plugin_info_deprecated_roundtrip() {
        let info = PluginInfo::new(
            "old-plugin".to_owned(),
            1,
            "old-plugin.wasm".to_owned(),
            true,
        );
        let json = serde_json::to_string(&info).expect("serialises");
        let info2: PluginInfo = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(info, info2);
        assert!(info2.is_deprecated);
    }
}
