//! BorrowChecker WASM frontend — CSR entry point.
//!
//! This binary is compiled to `wasm32-unknown-unknown` by Trunk with
//! `--features csr` and served inside the Tauri `WebView`. The native stub
//! satisfies `cargo check --workspace` without importing WASM-only APIs.

fn main() {}
