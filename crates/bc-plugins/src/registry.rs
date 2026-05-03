//! [`PluginRegistry`]: discovers and loads WASM importer plugins from directories.
//!
//! Call [`PluginRegistry::load`] with a list of search paths, then call
//! [`PluginRegistry::into_importer_registry`] to produce a
//! [`bc_core::ImporterRegistry`] pre-populated with all loaded plugins.

use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use bc_core::Importer as _;
use wasmtime::Store;
use wasmtime::component::Linker;

use crate::host::BcPlugin;
use crate::host::HostCtx;
use crate::manifest::ManifestError;
use crate::manifest::PluginManifest;
use crate::plugin_importer::PluginImporter;

/// Errors that can occur during plugin registry initialisation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The wasmtime engine could not be created.
    #[error("failed to create wasmtime engine: {0}")]
    Engine(String),
}

/// Discovers and loads WASM importer plugins from one or more directories.
///
/// Call [`PluginRegistry::load`] with a list of search paths, then call
/// [`build_importer_registry`](Self::build_importer_registry) to produce a
/// [`bc_core::ImporterRegistry`] pre-populated with all loaded plugins.
/// Individual plugin metadata is accessible via [`plugins`](Self::plugins).
#[non_exhaustive]
pub struct PluginRegistry {
    /// Successfully loaded plugin importers, wrapped in `Arc` for sharing.
    importers: Vec<Arc<PluginImporter>>,
}

impl PluginRegistry {
    /// Scans `paths` for `*.wasm` + `*.toml` plugin pairs and loads them.
    ///
    /// Paths are searched in order. Within each directory, plugins are loaded
    /// in filesystem order. A plugin in an earlier path takes precedence over
    /// a plugin with the same name in a later path.
    ///
    /// Failures to load individual plugins are logged as warnings and skipped
    /// rather than aborting the entire load.
    ///
    /// # Arguments
    ///
    /// * `paths` - Directories to scan for plugin files.
    ///
    /// # Returns
    ///
    /// A [`PluginRegistry`] containing all successfully loaded plugins.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryError`] if the wasmtime engine cannot be created.
    #[inline]
    pub fn load(paths: &[PathBuf]) -> Result<Self, RegistryError> {
        let engine = wasmtime::Engine::new(&wasmtime::Config::default())
            .map_err(|e| RegistryError::Engine(e.to_string()))?;
        let mut linker: Linker<HostCtx> = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
            .map_err(|e| RegistryError::Engine(format!("failed to add WASI to linker: {e}")))?;

        let mut importers = Vec::new();
        let mut seen_names: HashSet<String> = HashSet::new();

        for dir in paths {
            load_from_dir(dir, &engine, &linker, &mut importers, &mut seen_names);
        }

        Ok(Self { importers })
    }

    /// Returns the number of successfully loaded plugins.
    ///
    /// # Returns
    ///
    /// The count of loaded plugins.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.importers.len()
    }

    /// Returns `true` if no plugins were loaded.
    ///
    /// # Returns
    ///
    /// `true` if the registry contains no plugins.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.importers.is_empty()
    }

    /// Returns an iterator over the loaded [`PluginImporter`] instances.
    ///
    /// Each item carries the full manifest metadata (`name`, `version`,
    /// `sdk_abi`, `source_path`) as well as the compiled WASM component.
    ///
    /// # Returns
    ///
    /// An iterator of `&Arc<PluginImporter>`.
    #[inline]
    pub fn plugins(&self) -> impl Iterator<Item = &Arc<PluginImporter>> {
        self.importers.iter()
    }

    /// Builds a [`bc_core::ImporterRegistry`] pre-populated with all loaded
    /// plugin importers without consuming `self`.
    ///
    /// This is the preferred method when you need to retain access to plugin
    /// metadata (via [`plugins`](Self::plugins)) after building the registry.
    ///
    /// # Returns
    ///
    /// A [`bc_core::ImporterRegistry`] with one factory per loaded plugin.
    #[inline]
    #[must_use]
    pub fn build_importer_registry(&self) -> bc_core::ImporterRegistry {
        let mut registry = bc_core::ImporterRegistry::new();
        for importer_arc in &self.importers {
            let name = importer_arc.name().to_owned();
            let detect_imp = Arc::clone(importer_arc);
            let create_imp = Arc::clone(importer_arc);
            registry.register(bc_core::ImporterFactory::new(
                name,
                move |bytes| detect_imp.detect(bytes),
                move || Box::new(PluginImporterRef(Arc::clone(&create_imp))),
            ));
        }
        registry
    }

    /// Consumes this registry and returns a [`bc_core::ImporterRegistry`]
    /// pre-populated with all loaded plugin importers.
    ///
    /// For new code, prefer [`build_importer_registry`](Self::build_importer_registry)
    /// which does not consume `self` and allows continued access to plugin metadata.
    ///
    /// # Returns
    ///
    /// A [`bc_core::ImporterRegistry`] with one factory per loaded plugin.
    #[inline]
    #[must_use]
    pub fn into_importer_registry(self) -> bc_core::ImporterRegistry {
        self.build_importer_registry()
    }
}

/// Thin newtype that delegates [`bc_core::Importer`] to an `Arc<PluginImporter>`.
///
/// This allows the factory's `create` closure to return `Box<dyn Importer>`
/// pointing to the same underlying component without cloning or re-compiling it.
struct PluginImporterRef(Arc<PluginImporter>);

impl bc_core::Importer for PluginImporterRef {
    #[inline]
    fn name(&self) -> &str {
        self.0.name()
    }

    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        self.0.detect(bytes)
    }

    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        config: &bc_core::ImportConfig,
    ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
        self.0.import(bytes, config)
    }
}

/// Instantiates a component briefly to call its exported `name()` function.
///
/// Used during plugin load to verify the component's self-reported name against
/// the sidecar manifest. A temporary `Store` is created and discarded after the call.
///
/// # Errors
///
/// Returns a wasmtime error if instantiation or the `name()` call fails.
fn query_plugin_name(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    linker: &Linker<HostCtx>,
) -> wasmtime::Result<String> {
    let mut store = Store::new(engine, HostCtx::new());
    let bindings = BcPlugin::instantiate(&mut store, component, linker)?;
    bindings.borrow_checker_sdk_importer().call_name(&mut store)
}

/// Instantiates a component briefly to call its exported `sdk_abi()` function.
///
/// Used during plugin load to verify the component's self-reported ABI version
/// matches the sidecar manifest. Prevents a tampered manifest from bypassing
/// the ABI compatibility check. A temporary `Store` is created and discarded.
///
/// # Errors
///
/// Returns a wasmtime error if instantiation or the `sdk_abi()` call fails.
fn query_plugin_abi(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    linker: &Linker<HostCtx>,
) -> wasmtime::Result<u32> {
    let mut store = Store::new(engine, HostCtx::new());
    let bindings = BcPlugin::instantiate(&mut store, component, linker)?;
    bindings
        .borrow_checker_sdk_importer()
        .call_sdk_abi(&mut store)
}

/// Scans a single directory for `*.wasm` + `*.toml` pairs and appends loaded plugins.
fn load_from_dir(
    dir: &Path,
    engine: &wasmtime::Engine,
    linker: &Linker<HostCtx>,
    importers: &mut Vec<Arc<PluginImporter>>,
    seen_names: &mut HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(dir = %dir.display(), error = %e, "plugin dir not accessible, skipping");
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }
        if let Some(imp) = try_load_plugin(&path, engine, linker, seen_names) {
            let name = imp.name().to_owned();
            let version = imp.version().to_owned();
            let sdk_abi = imp.sdk_abi();
            seen_names.insert(name.clone());
            importers.push(Arc::new(imp));
            tracing::info!(name, version, sdk_abi, "loaded plugin");
        }
    }
}

/// Attempts to load a single `*.wasm` plugin, returning `None` (with a warning) on failure.
///
/// Validates: sidecar manifest exists and is compatible, WASM bytes compile,
/// component's `name()` matches the manifest, component's `sdk_abi()` matches the manifest.
fn try_load_plugin(
    path: &Path,
    engine: &wasmtime::Engine,
    linker: &Linker<HostCtx>,
    seen_names: &HashSet<String>,
) -> Option<PluginImporter> {
    let toml_path = path.with_extension("toml");
    let manifest = match PluginManifest::load(&toml_path) {
        Ok(m) => m,
        Err(ManifestError::Io { .. }) => {
            tracing::warn!(wasm = %path.display(), "no sidecar manifest found, skipping plugin");
            return None;
        }
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "invalid plugin manifest, skipping");
            return None;
        }
    };

    if seen_names.contains(&manifest.name) {
        tracing::debug!(
            name = manifest.name,
            "duplicate plugin name, earlier path takes precedence"
        );
        return None;
    }

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "cannot read plugin file, skipping");
            return None;
        }
    };

    let component = match wasmtime::component::Component::new(engine, &bytes) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "plugin compilation failed, skipping");
            return None;
        }
    };

    match query_plugin_name(engine, &component, linker) {
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "plugin name() failed at load, skipping");
            return None;
        }
        Ok(n) if n != manifest.name => {
            tracing::warn!(wasm = %path.display(), manifest_name = manifest.name, plugin_name = n, "plugin name() does not match manifest, skipping");
            return None;
        }
        Ok(_) => {}
    }

    match query_plugin_abi(engine, &component, linker) {
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "plugin sdk_abi() failed at load, skipping");
            return None;
        }
        Ok(abi) if abi != manifest.sdk_abi => {
            tracing::warn!(wasm = %path.display(), manifest_abi = manifest.sdk_abi, component_abi = abi, "plugin sdk_abi() does not match manifest, skipping");
            return None;
        }
        Ok(_) => {}
    }

    Some(PluginImporter::new(
        manifest.name,
        manifest.version,
        manifest.sdk_abi,
        path.to_owned(),
        engine.clone(),
        component,
        linker.clone(),
    ))
}
