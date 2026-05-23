//! Shell components: [`ConsoleShell`] wrapper and [`top_bar::TopBar`].
//! The [`palette`] module contains a [`palette::CommandPalette`] stub wired up in Phase 2.

pub mod palette;
pub mod top_bar;

use leptos::prelude::*;
use leptos_router::components::Outlet;
pub use top_bar::TopBar;

/// Full-app wrapper that renders [`TopBar`] above the routed content area.
///
/// Used as a Leptos Router layout route — child routes render via [`Outlet`].
#[component]
pub fn ConsoleShell() -> impl IntoView {
    view! {
        <div class="console-shell">
            <TopBar />
            <main class="console-main">
                <Outlet />
            </main>
        </div>
    }
}
