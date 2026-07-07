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
use crate::shell::palette::CommandPalette;

/// Full-app wrapper that renders [`TopBar`] above the routed content area.
///
/// Owns the `palette_open` signal and wires the global ⌘K / Ctrl+K shortcut.
/// Child routes render via [`Outlet`].
#[cfg(target_arch = "wasm32")]
#[component]
pub fn ConsoleShell() -> impl IntoView {
    let _currency_store = crate::currency_ctx::provide_currency_store();
    let _filter_store = crate::filter_ctx::provide_filter_store();
    let _toast_store = crate::components::toast::provide_toast_store();
    let palette_open = RwSignal::new(false);

    /* Global ⌘K / Ctrl+K shortcut — toggles the command palette from anywhere. */
    let handle = window_event_listener(ev::keydown, move |ke| {
        if ke.key() == "k" && (ke.meta_key() || ke.ctrl_key()) {
            palette_open.update(|v| *v = !*v);
            ke.prevent_default();
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div class="console-shell">
            <TopBar on_search=Callback::new(move |()| palette_open.update(|v| *v = !*v)) />
            <main class="console-main">
                <Outlet />
            </main>
            <CommandPalette
                open=palette_open.read_only()
                on_close=Callback::new(move |()| palette_open.set(false))
            />
            <crate::components::toast::ToastHost />
        </div>
    }
}
