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

#[cfg(any(target_arch = "wasm32", test))]
mod label;

#[cfg(target_arch = "wasm32")]
mod app;
#[cfg(any(target_arch = "wasm32", test))]
mod components;
#[cfg(target_arch = "wasm32")]
mod currency_ctx;
#[cfg(any(target_arch = "wasm32", test))]
mod filter_ctx;
#[cfg(any(target_arch = "wasm32", test))]
mod pages;
#[cfg(any(target_arch = "wasm32", test))]
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

/// Native-test access to pure logic modules that live under the wasm-only
/// `components` tree.
///
/// `components` (and its descendants) are gated on `target_arch = "wasm32"`
/// because they pull in Leptos and `web-sys`, so their pure, host-testable
/// submodules are unreachable from a native `cargo nextest` run through the
/// normal module path. Each such module is re-mounted here under `cfg(test)`
/// via `include!`, so its `#[cfg(test)] mod tests` runs on the host. The file
/// is authored once at its canonical path; this is an alternate mount, not a
/// copy.
#[cfg(test)]
mod components_tests {
    pub mod transaction_row {
        pub mod editable {
            include!("components/transaction_row/editable.rs");
        }
        pub mod audit {
            include!("components/transaction_row/audit.rs");
        }
    }
    pub mod account_picker {
        pub mod matching {
            include!("components/account_picker/matching.rs");
        }
    }
    pub mod num {
        pub mod meta {
            include!("components/num/meta.rs");
        }
    }
}

/// Native-test access to pure logic modules that live under the wasm-only
/// `pages` tree.
///
/// Same rationale as [`components_tests`]: `pages::settings` is gated on
/// `target_arch = "wasm32"` in `pages.rs`, so its pure, host-testable
/// submodules (e.g. `first_conflict`) are unreachable from a native
/// `cargo nextest` run through the normal module path. Re-mounted here under
/// `cfg(test)` via `include!`; the file is authored once at its canonical
/// path.
#[cfg(test)]
mod pages_tests {
    pub mod settings {
        pub mod backup {
            include!("pages/settings/backup/mod.rs");
        }
        pub mod currencies {
            include!("pages/settings/currencies/mod.rs");
        }
    }
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
