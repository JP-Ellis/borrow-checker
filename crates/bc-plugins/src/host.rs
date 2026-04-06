//! wasmtime Component Model host bindings generated from the bc-sdk WIT.
//!
//! The `bindgen!` macro reads the WIT files at compile time and generates
//! Rust types for calling into importer plugins.

#[allow(
    clippy::all,
    clippy::pedantic,
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
