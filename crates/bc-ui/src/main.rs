//! BorrowChecker WASM frontend — CSR entry point.
//!
//! This binary is compiled to `wasm32-unknown-unknown` by Trunk and served
//! inside the Tauri `WebView`. The native stub satisfies
//! `cargo check --workspace` without importing WASM-only APIs.

// Leptos's #[component] macro generates a struct + IntoView impl whose method
// matches the function name.
#![cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::same_name_method,
        reason = "Leptos #[component] macro generates structs with a method name matching the function"
    )
)]
// Leptos 0.8 prop handling internally shadows parameter names.
#![cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::shadow_reuse,
        reason = "Leptos #[component] prop handling introduces parameter shadowing"
    )
)]
// Component names are intentionally named after their modules.
#![cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::module_name_repetitions,
        reason = "page and component names match their module names by design (Dashboard in dashboard, etc.)"
    )
)]
// Leptos and leptos_router macros emit absolute paths in their expansions.
#![cfg_attr(
    target_arch = "wasm32",
    expect(
        clippy::absolute_paths,
        reason = "leptos macro expansions (path!, view!, #[component]) emit absolute paths we cannot control"
    )
)]

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(target_arch = "wasm32")]
mod components;
#[cfg(target_arch = "wasm32")]
mod pages;
#[cfg(target_arch = "wasm32")]
mod shell;

#[cfg(target_arch = "wasm32")]
fn main() {
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(app::App);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {}
