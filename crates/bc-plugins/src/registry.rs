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
use crate::host::bindings;
use crate::plugin_importer::PluginImporter;

/// Current ABI version supported by this host.
pub const HOST_ABI_VERSION: u32 = 1;

/// Minimum ABI version that this host supports (hard floor — below this is an error).
pub const HOST_ABI_MIN: u32 = 1;

/// Minimum ABI version that is in the deprecation grace window.
///
/// Plugins whose `sdk_abi` is in the range `HOST_ABI_DEPRECATED_MIN ..< HOST_ABI_MIN`
/// are still loaded but emit a warning indicating that support will be dropped
/// in a future release. Currently this equals [`HOST_ABI_MIN`] (no grace window
/// exists yet), but the three-tier validation logic is wired up and ready.
pub const HOST_ABI_DEPRECATED_MIN: u32 = 1;

/// Errors that can occur during plugin registry initialisation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// The wasmtime engine could not be created.
    #[error("failed to create wasmtime engine: {0}")]
    Engine(String),
    /// A plugin WASM file could not be read.
    #[error("cannot read plugin file at {path}: {source}")]
    Io {
        /// Path to the file that could not be read.
        path: String,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The WASM bytes could not be compiled as a component.
    #[error("plugin compilation failed for {path}: {message}")]
    Compile {
        /// Path to the WASM file.
        path: String,
        /// The compilation error message.
        message: String,
    },
    /// The plugin's exported `name()` or `sdk_abi()` function could not be called.
    #[error("failed to query plugin metadata from {path}: {message}")]
    Probe {
        /// Path to the WASM file.
        path: String,
        /// The error message from the probe instantiation.
        message: String,
    },
    /// The plugin's ABI version is not supported by this host.
    #[error(
        "plugin '{name}' requires ABI v{sdk_abi}, host supports \
         v{HOST_ABI_DEPRECATED_MIN}–v{HOST_ABI_VERSION} \
         (deprecated floor v{HOST_ABI_DEPRECATED_MIN}, hard floor v{HOST_ABI_MIN})"
    )]
    UnsupportedAbi {
        /// The plugin name.
        name: String,
        /// The ABI version the plugin was compiled against.
        sdk_abi: u32,
    },
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
    /// Scans `paths` for `*.wasm` plugin files, probes each for metadata, and loads them.
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
        let linker = build_linker(&engine)?;

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
    /// Each item carries the full plugin metadata (`name`, `sdk_abi`, `source_path`)
    /// as well as the compiled WASM component.
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

/// Queries lightweight metadata from a plugin WASM component without registering it.
///
/// Creates a temporary engine and linker, compiles the WASM, and calls the
/// component's exported `name()` and `sdk_abi()` functions to extract identity
/// information. Useful for install/validation workflows that need plugin identity
/// before placing the file in the plugin directory.
///
/// # Arguments
///
/// * `wasm_path` - Path to the `.wasm` file to probe.
///
/// # Returns
///
/// A [`crate::PluginMetadata`] containing the plugin's name and ABI version.
///
/// # Errors
///
/// Returns [`RegistryError`] if the engine cannot be created, the file cannot
/// be read, the WASM fails to compile, or the probe instantiation fails.
pub fn query_metadata(wasm_path: &std::path::Path) -> Result<crate::PluginMetadata, RegistryError> {
    let engine = wasmtime::Engine::new(&wasmtime::Config::default())
        .map_err(|e| RegistryError::Engine(e.to_string()))?;
    let linker = build_linker(&engine)?;

    let bytes = std::fs::read(wasm_path).map_err(|source| RegistryError::Io {
        path: wasm_path.display().to_string(),
        source,
    })?;

    let component = wasmtime::component::Component::new(&engine, &bytes).map_err(|e| {
        RegistryError::Compile {
            path: wasm_path.display().to_string(),
            message: e.to_string(),
        }
    })?;

    let name =
        query_plugin_name(&engine, &component, &linker).map_err(|e| RegistryError::Probe {
            path: wasm_path.display().to_string(),
            message: e.to_string(),
        })?;

    let sdk_abi =
        query_plugin_abi(&engine, &component, &linker).map_err(|e| RegistryError::Probe {
            path: wasm_path.display().to_string(),
            message: e.to_string(),
        })?;

    Ok(crate::PluginMetadata { name, sdk_abi })
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

/// Builds a wasmtime [`Linker`] with WASI and the bc-sdk logger host import wired up.
///
/// # Errors
///
/// Returns [`RegistryError::Engine`] if WASI or the logger import cannot be added.
fn build_linker(engine: &wasmtime::Engine) -> Result<Linker<HostCtx>, RegistryError> {
    let mut linker: Linker<HostCtx> = Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)
        .map_err(|e| RegistryError::Engine(format!("failed to add WASI to linker: {e}")))?;

    // Add host imports (logger interface).
    bindings::borrow_checker::sdk::logger::add_to_linker::<
        HostCtx,
        wasmtime::component::HasSelf<HostCtx>,
    >(&mut linker, |ctx| ctx)
    .map_err(|e| RegistryError::Engine(format!("failed to add host imports to linker: {e}")))?;

    Ok(linker)
}

/// Instantiates a component briefly to call its exported `name()` function.
///
/// A temporary `Store` is created and discarded after the call.
///
/// # Errors
///
/// Returns a wasmtime error if instantiation or the `name()` call fails.
fn query_plugin_name(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    linker: &Linker<HostCtx>,
) -> wasmtime::Result<String> {
    let mut store = Store::new(engine, HostCtx::new("__probe__"));
    let bindings = BcPlugin::instantiate(&mut store, component, linker)?;
    bindings.borrow_checker_sdk_importer().call_name(&mut store)
}

/// Instantiates a component briefly to call its exported `sdk_abi()` function.
///
/// A temporary `Store` is created and discarded after the call.
///
/// # Errors
///
/// Returns a wasmtime error if instantiation or the `sdk_abi()` call fails.
fn query_plugin_abi(
    engine: &wasmtime::Engine,
    component: &wasmtime::component::Component,
    linker: &Linker<HostCtx>,
) -> wasmtime::Result<u32> {
    let mut store = Store::new(engine, HostCtx::new("__probe__"));
    let bindings = BcPlugin::instantiate(&mut store, component, linker)?;
    bindings
        .borrow_checker_sdk_importer()
        .call_sdk_abi(&mut store)
}

/// Scans a single directory for `*.wasm` files and appends loaded plugins.
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
            let sdk_abi = imp.sdk_abi();
            seen_names.insert(name.clone());
            importers.push(Arc::new(imp));
            tracing::info!(name, sdk_abi, "loaded plugin");
        }
    }
}

/// Validates the ABI version queried from the WASM component.
///
/// Returns `true` if the ABI is within the supported range (emitting a warning
/// for deprecated-but-still-loaded versions). Returns `false` for hard
/// out-of-range values, which causes the plugin to be skipped.
fn validate_abi(path: &Path, name: &str, sdk_abi: u32) -> bool {
    if sdk_abi > HOST_ABI_VERSION || sdk_abi < HOST_ABI_DEPRECATED_MIN {
        tracing::warn!(
            wasm = %path.display(),
            plugin = name,
            sdk_abi,
            host_abi_version = HOST_ABI_VERSION,
            host_abi_min = HOST_ABI_DEPRECATED_MIN,
            "plugin ABI version not supported, skipping"
        );
        return false;
    }
    if sdk_abi < HOST_ABI_MIN {
        tracing::warn!(
            wasm = %path.display(),
            plugin = name,
            sdk_abi,
            host_abi_min = HOST_ABI_MIN,
            "plugin uses a deprecated ABI version and will not be loadable in a future release"
        );
    }
    true
}

/// Attempts to load a single `*.wasm` plugin, returning `None` (with a warning) on failure.
///
/// The flow is: read bytes → compile component → query `name()` → check for
/// duplicates → query `sdk_abi()` → validate ABI → build [`PluginImporter`].
fn try_load_plugin(
    path: &Path,
    engine: &wasmtime::Engine,
    linker: &Linker<HostCtx>,
    seen_names: &HashSet<String>,
) -> Option<PluginImporter> {
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

    let name = match query_plugin_name(engine, &component, linker) {
        Ok(n) => n,
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "plugin name() failed at load, skipping");
            return None;
        }
    };

    if seen_names.contains(&name) {
        tracing::debug!(name, "duplicate plugin name, earlier path takes precedence");
        return None;
    }

    let sdk_abi = match query_plugin_abi(engine, &component, linker) {
        Ok(abi) => abi,
        Err(e) => {
            tracing::warn!(wasm = %path.display(), error = %e, "plugin sdk_abi() failed at load, skipping");
            return None;
        }
    };

    if !validate_abi(path, &name, sdk_abi) {
        return None;
    }

    Some(PluginImporter::new(
        name,
        sdk_abi,
        path.to_owned(),
        engine.clone(),
        component,
        linker.clone(),
    ))
}
