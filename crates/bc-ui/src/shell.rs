//! Shell components: [`ConsoleShell`] wrapper and [`top_bar::TopBar`].
//! The [`palette`] module contains a [`palette::CommandPalette`] stub wired up in Phase 2.

pub mod palette;
pub mod top_bar;

use leptos::prelude::*;
pub use top_bar::TopBar;

/// Full-app wrapper that renders [`TopBar`] above the routed content area.
///
/// Every route is wrapped in this component — nav and search are always
/// visible.
#[component]
pub fn ConsoleShell(
    /// Routed page content rendered in the main content area.
    children: Children,
) -> impl IntoView {
    view! {
        <div class="console-shell">
            <TopBar />
            <main class="console-main">{children()}</main>
        </div>
    }
}
