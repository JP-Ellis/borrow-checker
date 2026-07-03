//! Conversion from `bc_plugins::PluginImporter` into `bc_ipc::PluginInfo`.
//!
//! This module is gated behind the `ipc` feature. Because the source type
//! (`PluginImporter`) is local to this crate, the orphan rule permits `impl
//! From` even though the destination DTO lives in `bc-ipc`.

use bc_core::Importer as _;

use crate::plugin_importer::PluginImporter;

/// Converts a loaded [`PluginImporter`] into the IPC [`bc_ipc::PluginInfo`]
/// snapshot consumed by the WASM frontend.
///
/// `file_name` is extracted from the plugin's source path here (on the
/// native side) so that WASM-side path manipulation is not needed.
impl From<&PluginImporter> for bc_ipc::PluginInfo {
    fn from(plugin: &PluginImporter) -> Self {
        let file_name = plugin
            .source_path()
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_else(|| {
                // A plugin path without a filename component is
                // effectively impossible (the registry only loads
                // *.wasm files from directories), but fall back
                // gracefully rather than panicking.
                tracing::warn!(
                    path = %plugin.source_path().display(),
                    "plugin source path has no valid filename component"
                );
                ""
            })
            .to_owned();
        let is_deprecated = plugin.is_deprecated();

        bc_ipc::PluginInfo::new(
            plugin.name().to_owned(),
            plugin.sdk_abi(),
            file_name,
            is_deprecated,
        )
    }
}
