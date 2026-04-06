//! [`PluginImporter`]: a `bc_core::Importer` backed by a WASM component.
//!
//! Each call to [`bc_core::Importer::detect`] or [`bc_core::Importer::import`]
//! creates a fresh wasmtime [`Store`] and instantiates the component. This is
//! safe and correct; the component holds no persistent state between calls.

use std::sync::Arc;

use wasmtime::Store;
use wasmtime::component::Component;
use wasmtime::component::Linker;

use crate::host::ImporterPlugin;
use crate::translate::wit_to_import_error;
use crate::translate::wit_to_raw_transaction;

/// Wraps a loaded WASM importer component and implements [`bc_core::Importer`].
///
/// The underlying wasmtime `Engine`, `Component`, and `Linker` are shared
/// across all clones of this importer via `Arc`. Each call to `detect` or
/// `import` creates a fresh `Store` and instantiates the component
/// independently, so concurrent calls are safe.
#[non_exhaustive]
pub(crate) struct PluginImporter {
    /// The stable plugin name read from the manifest.
    name: String,
    /// Shared wasmtime engine (internally Arc-backed).
    engine: wasmtime::Engine,
    /// The compiled WASM component.
    component: Arc<Component>,
    /// Pre-configured linker for instantiating the component.
    linker: Arc<Linker<()>>,
}

impl core::fmt::Debug for PluginImporter {
    /// Formats the importer, showing only the `name` field.
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PluginImporter")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl PluginImporter {
    /// Creates a new [`PluginImporter`] from a compiled component.
    ///
    /// # Arguments
    ///
    /// * `name` - The stable plugin name (from the manifest).
    /// * `engine` - The shared wasmtime engine.
    /// * `component` - The compiled WASM component.
    /// * `linker` - The pre-configured component linker.
    ///
    /// # Returns
    ///
    /// A new [`PluginImporter`] ready for use.
    #[inline]
    #[must_use]
    pub(crate) fn new(
        name: String,
        engine: wasmtime::Engine,
        component: Component,
        linker: Linker<()>,
    ) -> Self {
        Self {
            name,
            engine,
            component: Arc::new(component),
            linker: Arc::new(linker),
        }
    }

    /// Instantiates the component with a fresh store.
    ///
    /// # Returns
    ///
    /// A tuple of the instantiated bindings and the store.
    ///
    /// # Errors
    ///
    /// Returns a wasmtime error if instantiation fails.
    #[inline]
    fn instantiate(&self) -> wasmtime::Result<(ImporterPlugin, Store<()>)> {
        let mut store = Store::new(&self.engine, ());
        let bindings = ImporterPlugin::instantiate(&mut store, &self.component, &self.linker)?;
        Ok((bindings, store))
    }
}

impl bc_core::Importer for PluginImporter {
    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        match self.instantiate() {
            Ok((bindings, mut store)) => bindings
                .borrow_checker_sdk_importer()
                .call_detect(&mut store, bytes)
                .unwrap_or(false),
            Err(e) => {
                tracing::warn!(plugin = %self.name, error = %e, "plugin detect() failed to instantiate");
                false
            }
        }
    }

    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        config: &bc_core::ImportConfig,
    ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
        let config_json = serde_json::to_string(config.as_value())
            .map_err(|e| bc_core::ImportError::Parse(format!("config serialisation: {e}")))?;

        let (bindings, mut store) = self.instantiate().map_err(|e| {
            bc_core::ImportError::Parse(format!("plugin instantiation failed: {e}"))
        })?;

        let result = bindings
            .borrow_checker_sdk_importer()
            .call_parse(&mut store, bytes, &config_json)
            .map_err(|e| bc_core::ImportError::Parse(format!("plugin call failed: {e}")))?;

        result
            .map(|txs| txs.into_iter().map(wit_to_raw_transaction).collect())
            .map_err(wit_to_import_error)
    }
}
