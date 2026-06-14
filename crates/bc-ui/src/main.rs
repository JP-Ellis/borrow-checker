//! BorrowChecker WASM frontend — CSR entry point.
//!
//! This binary is compiled to `wasm32-unknown-unknown` by Trunk and served
//! inside the Tauri `WebView`. The native stub satisfies
//! `cargo check --workspace` without importing WASM-only APIs.

#![cfg_attr(
    target_arch = "wasm32",
    // mod.rs is used throughout to collocate source with SCSS module files.
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates source with its SCSS module file"
    ),
    // Leptos's #[component] macro generates a struct + IntoView impl whose
    // method matches the function name.
    expect(
        clippy::same_name_method,
        reason = "Leptos #[component] macro generates structs with a method name matching the function"
    ),
    // Leptos 0.8 prop handling internally shadows parameter names.
    expect(
        clippy::shadow_reuse,
        reason = "Leptos #[component] prop handling introduces parameter shadowing"
    ),
    // Component names are intentionally named after their modules.,
    expect(
        clippy::module_name_repetitions,
        reason = "page and component names match their module names by design (Dashboard in dashboard, etc.)"
    ),
    // Leptos and leptos_router macros emit absolute paths in their expansions.
    expect(
        clippy::absolute_paths,
        reason = "leptos macro expansions (path!, view!, #[component]) emit absolute paths we cannot control"
    ),
)]

#[cfg(any(target_arch = "wasm32", test))]
mod format;

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
#[expect(clippy::panic, reason = "wasm32 exclusive crate")]
fn main() {
    panic!("wasm32 exclusive crate");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    #[should_panic(expected = "wasm32 exclusive crate")]
    fn it_panics() {
        main();
    }
}
