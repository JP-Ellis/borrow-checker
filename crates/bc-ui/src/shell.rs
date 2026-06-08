//! Shell components: [`ConsoleShell`] wrapper and [`top_bar::TopBar`].
//! The [`palette`] module contains a [`palette::CommandPalette`] wired into [`ConsoleShell`].

pub mod palette;
pub mod top_bar;

use leptos::prelude::*;
use leptos::wasm_bindgen::JsCast as _;
use leptos::web_sys;
use leptos_router::components::Outlet;
pub use top_bar::TopBar;

use crate::shell::palette::CommandPalette;

/// Full-app wrapper that renders [`TopBar`] above the routed content area.
///
/// Owns the `palette_open` signal and wires the global ⌘K / Ctrl+K shortcut.
/// Child routes render via [`Outlet`].
#[component]
pub fn ConsoleShell() -> impl IntoView {
    let palette_open = RwSignal::new(false);

    /* Global ⌘K / Ctrl+K shortcut — opens the command palette from anywhere. */
    window_event_listener_untyped("keydown", move |e| {
        let ke: web_sys::KeyboardEvent = e.unchecked_into();
        if ke.key() == "k" && (ke.meta_key() || ke.ctrl_key()) {
            palette_open.set(true);
            ke.prevent_default();
        }
    });

    view! {
        <div class="console-shell">
            <TopBar on_search=Callback::new(move |()| palette_open.set(true)) />
            <main class="console-main">
                <Outlet />
            </main>
            <CommandPalette
                open=palette_open.read_only()
                on_close=Callback::new(move |()| palette_open.set(false))
            />
        </div>
    }
}
