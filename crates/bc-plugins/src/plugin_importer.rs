//! [`PluginImporter`]: a `bc_core::Importer` backed by a WASM component.
//!
//! Each call to [`bc_core::Importer::import`] creates a fresh wasmtime
//! [`Store`] and instantiates the component. This is safe and correct; the
//! component holds no persistent state between calls.

use std::sync::Arc;

use wasmtime::Store;
use wasmtime::component::Component;
use wasmtime::component::Linker;

use crate::host::BcPlugin;
use crate::host::HostCtx;

/// Wraps a loaded WASM importer component and implements [`bc_core::Importer`].
///
/// The underlying wasmtime `Engine`, `Component`, and `Linker` are shared
/// across all clones of this importer via `Arc`. Each call to `import`
/// creates a fresh `Store` and instantiates the component independently, so
/// concurrent calls are safe.
#[non_exhaustive]
pub struct PluginImporter {
    /// The stable plugin name queried from the WASM component.
    name: String,
    /// Integer ABI version the plugin was compiled against.
    sdk_abi: u32,
    /// Filesystem path to the `.wasm` file that was loaded.
    source_path: std::path::PathBuf,
    /// Shared wasmtime engine (internally Arc-backed).
    engine: wasmtime::Engine,
    /// The compiled WASM component.
    component: Arc<Component>,
    /// Pre-configured linker for instantiating the component.
    linker: Arc<Linker<HostCtx>>,
    /// Host directory preopened read-only as the plugin's filesystem root,
    /// or `None` for metadata-only contexts (probes).
    documents_root: Option<std::path::PathBuf>,
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
    /// Creates a new [`PluginImporter`] from a compiled component and queried metadata.
    ///
    /// # Arguments
    ///
    /// * `name` - The stable plugin name (queried from the WASM component).
    /// * `sdk_abi` - The integer ABI version the plugin was compiled against.
    /// * `source_path` - Filesystem path to the `.wasm` file.
    /// * `engine` - The shared wasmtime engine.
    /// * `component` - The compiled WASM component.
    /// * `linker` - The pre-configured component linker.
    /// * `documents_root` - Host directory preopened read-only as the
    ///   plugin's filesystem root, or `None` for metadata-only contexts.
    ///
    /// # Returns
    ///
    /// A new [`PluginImporter`] ready for use.
    #[inline]
    #[must_use]
    pub(crate) fn new(
        name: String,
        sdk_abi: u32,
        source_path: std::path::PathBuf,
        engine: wasmtime::Engine,
        component: Component,
        linker: Linker<HostCtx>,
        documents_root: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            name,
            sdk_abi,
            source_path,
            engine,
            component: Arc::new(component),
            linker: Arc::new(linker),
            documents_root,
        }
    }

    /// Returns the integer ABI version the plugin was compiled against.
    ///
    /// # Returns
    ///
    /// The ABI version number.
    #[inline]
    #[must_use]
    pub fn sdk_abi(&self) -> u32 {
        self.sdk_abi
    }

    /// Returns the filesystem path to the `.wasm` file that was loaded.
    ///
    /// # Returns
    ///
    /// A reference to the source path.
    #[inline]
    #[must_use]
    pub fn source_path(&self) -> &std::path::Path {
        &self.source_path
    }

    /// Returns `true` if the plugin was compiled against a deprecated ABI version.
    ///
    /// A plugin is deprecated when its `sdk_abi` falls in the grace window
    /// `HOST_ABI_DEPRECATED_MIN ..< HOST_ABI_MIN`. It still loads but will stop
    /// loading once the grace window closes at the next breaking ABI bump.
    ///
    /// # Returns
    ///
    /// `true` when `sdk_abi >= HOST_ABI_DEPRECATED_MIN && sdk_abi < HOST_ABI_MIN`.
    #[inline]
    #[must_use]
    #[expect(
        clippy::impossible_comparisons,
        reason = "HOST_ABI_DEPRECATED_MIN == HOST_ABI_MIN today (empty grace window); \
                  expression is correct and will activate automatically when HOST_ABI_MIN is bumped"
    )]
    pub fn is_deprecated(&self) -> bool {
        use crate::registry::HOST_ABI_DEPRECATED_MIN;
        use crate::registry::HOST_ABI_MIN;

        self.sdk_abi >= HOST_ABI_DEPRECATED_MIN && self.sdk_abi < HOST_ABI_MIN
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
    fn instantiate(&self) -> wasmtime::Result<(BcPlugin, Store<HostCtx>)> {
        let ctx = HostCtx::new(&self.name, self.documents_root.as_deref())?;
        let mut store = Store::new(&self.engine, ctx);
        let bindings = BcPlugin::instantiate(&mut store, &self.component, &self.linker)?;
        Ok((bindings, store))
    }

    /// Runs the import against the guest on the calling thread.
    ///
    /// # Arguments
    ///
    /// * `config` - The profile configuration handed to the plugin.
    ///
    /// # Returns
    ///
    /// The transactions the plugin parsed.
    ///
    /// # Errors
    ///
    /// Returns [`bc_core::ImportError`] when `documents_root` is unset, the
    /// config cannot be serialised, instantiation fails, or the guest rejects
    /// the config or fails to parse.
    fn import_on_this_thread(
        &self,
        config: &bc_core::ImportConfig,
    ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
        if self.documents_root.is_none() {
            return Err(bc_core::ImportError::MissingField(
                "documents_root not configured".to_owned(),
            ));
        }

        let config_json = serde_json::to_string(config.as_value())
            .map_err(|e| bc_core::ImportError::Parse(format!("config serialisation: {e}")))?;

        let (bindings, mut store) = self.instantiate().map_err(|e| {
            bc_core::ImportError::Parse(format!("plugin instantiation failed: {e}"))
        })?;

        // Check the config before parsing, so that a plugin which does not
        // check its own is still covered. Well-behaved plugins validate at the
        // top of `parse` too, which makes this call redundant for them — and
        // therefore not independently observable in the tests.
        bindings
            .borrow_checker_sdk_importer()
            .call_validate(&mut store, &config_json)
            .map_err(|e| bc_core::ImportError::Parse(format!("plugin call failed: {e}")))?
            .map_err(bc_core::ImportError::from)?;

        let result = bindings
            .borrow_checker_sdk_importer()
            .call_parse(&mut store, &config_json)
            .map_err(|e| bc_core::ImportError::Parse(format!("plugin call failed: {e}")))?;

        let txs = result.map_err(bc_core::ImportError::from)?;
        txs.into_iter()
            .map(bc_core::RawTransaction::try_from)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Runs the config check against the guest on the calling thread.
    ///
    /// # Arguments
    ///
    /// * `config` - The profile configuration handed to the plugin.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the plugin accepts the config.
    ///
    /// # Errors
    ///
    /// Returns [`bc_core::ImportError`] when the config cannot be serialised,
    /// instantiation fails, or the guest rejects the config.
    fn validate_on_this_thread(
        &self,
        config: &bc_core::ImportConfig,
    ) -> Result<(), bc_core::ImportError> {
        let config_json = serde_json::to_string(config.as_value())
            .map_err(|e| bc_core::ImportError::Parse(format!("config serialisation: {e}")))?;

        // No `documents_root` check: validation reads no files, so a profile
        // can be checked before its source directory exists.
        let (bindings, mut store) = self.instantiate().map_err(|e| {
            bc_core::ImportError::Parse(format!("plugin instantiation failed: {e}"))
        })?;

        bindings
            .borrow_checker_sdk_importer()
            .call_validate(&mut store, &config_json)
            .map_err(|e| bc_core::ImportError::Parse(format!("plugin call failed: {e}")))?
            .map_err(bc_core::ImportError::from)
    }
}

/// Runs `f` on a scoped thread that carries no async runtime context.
///
/// WASI is linked with [`wasmtime_wasi::p2::add_to_linker_sync`], whose
/// blocking shim calls `Handle::block_on` whenever a Tokio runtime is current.
/// That panics with "Cannot start a runtime from within a runtime", so a guest
/// call made from an `async fn` would abort the process. Every caller is async
/// — the CLI is `#[tokio::main]` and the Tauri app runs on a runtime — so the
/// hop belongs here rather than at each call site.
///
/// # Arguments
///
/// * `f` - The closure to run off the runtime.
///
/// # Returns
///
/// Whatever `f` returns.
///
/// # Panics
///
/// Propagates a panic raised by `f`, so a guest trap surfaces unchanged.
fn off_async_runtime<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send,
    T: Send,
{
    std::thread::scope(|scope| match scope.spawn(f).join() {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    })
}

impl bc_core::Importer for PluginImporter {
    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    fn import(
        &self,
        config: &bc_core::ImportConfig,
    ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
        off_async_runtime(|| self.import_on_this_thread(config))
    }

    #[inline]
    fn validate(&self, config: &bc_core::ImportConfig) -> Result<(), bc_core::ImportError> {
        off_async_runtime(|| self.validate_on_this_thread(config))
    }
}
