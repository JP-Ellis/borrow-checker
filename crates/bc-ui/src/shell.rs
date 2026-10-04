//! Shell components: [`ConsoleShell`] wrapper and [`top_bar::TopBar`].
//! The [`palette`] module contains a [`palette::CommandPalette`] wired into [`ConsoleShell`].

pub mod palette;
#[cfg(target_arch = "wasm32")]
pub mod top_bar;

#[cfg(target_arch = "wasm32")]
use leptos::ev;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos_router::components::Outlet;
#[cfg(target_arch = "wasm32")]
pub use top_bar::TopBar;

#[cfg(target_arch = "wasm32")]
use crate::filter_ctx::Edit;
#[cfg(target_arch = "wasm32")]
use crate::shell::palette::CommandPalette;

/// Full-app wrapper that renders [`TopBar`] above the routed content area.
///
/// Owns the palette's open and edit signals, provides the palette opener, and
/// wires the global ⌘K / Ctrl+K shortcut.
/// Child routes render via [`Outlet`].
#[cfg(target_arch = "wasm32")]
#[component]
pub fn ConsoleShell() -> impl IntoView {
    let _currency_store = crate::currency_ctx::provide_currency_store();
    let _meta_key_store = crate::meta_keys_ctx::provide_meta_key_store();
    let _filter_store = crate::filter_ctx::provide_filter_store();
    let _toast_store = crate::components::toast::provide_toast_store();
    let _query_catalog = crate::query_catalog_ctx::provide_query_catalog();
    let palette_open = RwSignal::new(false);
    let palette_edit = RwSignal::new(Edit::append());
    let open_palette = Callback::new(move |edit: Edit| {
        palette_edit.set(edit);
        palette_open.set(true);
    });
    crate::filter_ctx::provide_palette_opener(open_palette);

    /* Global ⌘K / Ctrl+K shortcut: closes an open palette, else opens a blank one. */
    let handle = window_event_listener(ev::keydown, move |ke| {
        if ke.key() == "k" && (ke.meta_key() || ke.ctrl_key()) {
            if palette_open.get_untracked() {
                palette_open.set(false);
            } else {
                open_palette.run(Edit::append());
            }
            ke.prevent_default();
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="console-shell">
            <TopBar on_search=Callback::new(move |()| open_palette.run(Edit::append())) />
            <main class="console-main">
                <Outlet />
            </main>
            <CommandPalette
                open=palette_open.read_only()
                on_close=Callback::new(move |()| palette_open.set(false))
                edit=palette_edit.read_only()
            />
            <crate::components::toast::ToastHost />
        </div>
    }
}
