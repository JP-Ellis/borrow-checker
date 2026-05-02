//! wasmtime Component Model host bindings generated from the bc-sdk WIT.
//!
//! The `bindgen!` macro reads the WIT files at compile time and generates
//! Rust types for calling into importer plugins.

#[expect(
    clippy::all,
    clippy::restriction,
    reason = "generated code from wasmtime bindgen may not conform to workspace lint rules"
)]
pub(crate) mod bindings {
    wasmtime::component::bindgen!({
        path: "../bc-sdk/wit",
        world: "importer-plugin",
    });
}

pub(crate) use bindings::ImporterPlugin;
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
}

impl HostCtx {
    /// Creates a new `HostCtx` initialized with default WASI capabilities.
    pub(crate) fn new() -> Self {
        let mut wasi = WasiCtxBuilder::new();
        Self {
            table: ResourceTable::new(),
            wasi: wasi.build(),
        }
    }
}

impl WasiView for HostCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
