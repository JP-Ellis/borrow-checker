//! wasmtime Component Model host bindings generated from the bc-sdk WIT.
//!
//! The `bindgen!` macro reads the WIT files at compile time and generates
//! Rust types for calling into importer plugins.

/// Wasmtime-generated bindings for the `borrow-checker` WIT world.
#[expect(
    clippy::integer_division_remainder_used,
    clippy::missing_asserts_for_indexing,
    reason = "generated code from wasmtime bindgen may not conform to workspace lint rules"
)]
pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../bc-sdk/wit",
        world: "borrow-checker",
    });
}

/// Type alias for the wasmtime-generated `BorrowChecker` world bindings.
pub(crate) type BcPlugin = bindings::BorrowChecker;
use wasmtime_wasi::ResourceTable;
use wasmtime_wasi::WasiCtx;
use wasmtime_wasi::WasiCtxBuilder;
use wasmtime_wasi::WasiCtxView;
use wasmtime_wasi::WasiView;

/// Context provided to the WASM host, implementing `WasiView`.
pub(crate) struct HostCtx {
    /// The resource table used by WASI.
    table: ResourceTable,
    /// The WASI context state.
    wasi: WasiCtx,
    /// The name of the plugin this context belongs to, used as a tracing field.
    plugin_name: String,
}

impl HostCtx {
    /// Creates a new `HostCtx` initialized with default WASI capabilities.
    ///
    /// # Arguments
    ///
    /// * `plugin_name` - The name of the plugin, attached to all log entries
    ///   emitted by the plugin via the `logger` WIT import.
    #[inline]
    pub(crate) fn new(plugin_name: impl Into<String>) -> Self {
        let mut wasi = WasiCtxBuilder::new();
        Self {
            table: ResourceTable::new(),
            wasi: wasi.build(),
            plugin_name: plugin_name.into(),
        }
    }
}

impl WasiView for HostCtx {
    #[inline]
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl bindings::borrow_checker::sdk::logger::Host for HostCtx {
    /// Re-emits a plugin log entry through the host's `tracing` subscriber.
    ///
    /// The entry is attributed to `target = "bc::plugin"` with a `plugin` field
    /// identifying the calling plugin and an `extra_fields` field containing all
    /// structured key-value pairs the plugin attached.
    #[inline]
    fn log(
        &mut self,
        level: bindings::borrow_checker::sdk::logger::LogLevel,
        message: String,
        fields: Vec<bindings::borrow_checker::sdk::logger::LogField>,
    ) {
        use bindings::borrow_checker::sdk::logger::LogLevel;

        let plugin = self.plugin_name.as_str();
        let extra: String = fields
            .iter()
            .map(|f| format!("{} = {}", f.key, f.value))
            .collect::<Vec<_>>()
            .join(", ");

        match level {
            LogLevel::Trace => {
                tracing::trace!(target: "bc::plugin", plugin, extra_fields = %extra, "{message}");
            }
            LogLevel::Debug => {
                tracing::debug!(target: "bc::plugin", plugin, extra_fields = %extra, "{message}");
            }
            LogLevel::Info => {
                tracing::info!(target: "bc::plugin", plugin, extra_fields = %extra, "{message}");
            }
            LogLevel::Warn => {
                tracing::warn!(target: "bc::plugin", plugin, extra_fields = %extra, "{message}");
            }
            LogLevel::Error => {
                tracing::error!(target: "bc::plugin", plugin, extra_fields = %extra, "{message}");
            }
        }
    }
}
