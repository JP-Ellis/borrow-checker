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
use wasmtime::component::Linker;

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
/// [`into_importer_registry`](Self::into_importer_registry) to produce a
/// [`bc_core::ImporterRegistry`] pre-populated with all loaded plugins.
#[non_exhaustive]
pub struct PluginRegistry {
    /// Successfully loaded plugin importers.
    importers: Vec<PluginImporter>,
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
        let engine = wasmtime::Engine::default();
        let linker: Linker<()> = Linker::new(&engine);

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

    /// Consumes this registry and returns a [`bc_core::ImporterRegistry`]
    /// pre-populated with all loaded plugin importers.
    ///
    /// # Returns
    ///
    /// A [`bc_core::ImporterRegistry`] with one factory per loaded plugin.
    #[inline]
    #[must_use]
    pub fn into_importer_registry(self) -> bc_core::ImporterRegistry {
        let mut registry = bc_core::ImporterRegistry::new();
        for importer in self.importers {
            let name = importer.name().to_owned();
            let importer_arc = Arc::new(importer);
            let detect_imp = Arc::clone(&importer_arc);
            let create_imp = Arc::clone(&importer_arc);
            registry.register(bc_core::ImporterFactory::new(
                name,
                move |bytes| detect_imp.detect(bytes),
                move || Box::new(PluginImporterRef(Arc::clone(&create_imp))),
            ));
        }
        registry
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

/// Scans a single directory for `*.wasm` + `*.toml` pairs and appends loaded plugins.
fn load_from_dir(
    dir: &Path,
    engine: &wasmtime::Engine,
    linker: &Linker<()>,
    importers: &mut Vec<PluginImporter>,
    seen_names: &mut HashSet<String>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::debug!(
                dir = %dir.display(),
                error = %e,
                "plugin dir not accessible, skipping"
            );
            return;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("wasm") {
            continue;
        }

        let toml_path = path.with_extension("toml");
        let manifest = match PluginManifest::load(&toml_path) {
            Ok(m) => m,
            Err(ManifestError::Io { .. }) => {
                tracing::warn!(
                    wasm = %path.display(),
                    "no sidecar manifest found, skipping plugin"
                );
                continue;
            }
            Err(e) => {
                tracing::warn!(
                    wasm = %path.display(),
                    error = %e,
                    "invalid plugin manifest, skipping"
                );
                continue;
            }
        };

        if seen_names.contains(&manifest.name) {
            tracing::debug!(
                name = manifest.name,
                "duplicate plugin name, earlier path takes precedence"
            );
            continue;
        }

        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                tracing::warn!(
                    wasm = %path.display(),
                    error = %e,
                    "cannot read plugin file, skipping"
                );
                continue;
            }
        };

        let component = match wasmtime::component::Component::new(engine, &bytes) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    wasm = %path.display(),
                    error = %e,
                    "plugin compilation failed, skipping"
                );
                continue;
            }
        };

        let name = manifest.name.clone();
        let version = manifest.version.clone();
        seen_names.insert(manifest.name);
        importers.push(PluginImporter::new(
            name.clone(),
            engine.clone(),
            component,
            linker.clone(),
        ));
        tracing::info!(name, version, "loaded plugin");
    }
}
