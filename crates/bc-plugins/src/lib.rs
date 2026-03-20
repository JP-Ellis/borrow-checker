//! WASM host runtime and plugin ABI bridge.
//!
//! Loads `.wasm` plugin files via extism, wires up host functions,
//! and bridges plugin calls into `bc-core`.
