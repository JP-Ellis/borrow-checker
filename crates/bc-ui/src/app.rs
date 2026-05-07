//! Root application component with client-side router.

use leptos::prelude::*;
use leptos_router::components::Route;
use leptos_router::components::Router;
use leptos_router::components::Routes;
use leptos_router::path;

use crate::pages::Accounts;
use crate::pages::Budget;
use crate::pages::Dashboard;
use crate::pages::Plugins;
use crate::pages::Reports;
use crate::pages::Settings;
use crate::shell::ConsoleShell;

/// Root application component. Mounts the router and wraps all routes in
/// [`ConsoleShell`].
#[component]
#[expect(
    clippy::absolute_paths,
    reason = "leptos_router path! macro expansion emits absolute paths we cannot control"
)]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <ConsoleShell>
                <Routes fallback=|| view! { <p class="not-found">"page not found"</p> }>
                    <Route path=path!("/") view=Dashboard />
                    <Route path=path!("/accounts") view=Accounts />
                    <Route path=path!("/accounts/:id") view=Accounts />
                    <Route path=path!("/budget") view=Budget />
                    <Route path=path!("/reports") view=Reports />
                    <Route path=path!("/plugins") view=Plugins />
                    <Route path=path!("/settings") view=Settings />
                </Routes>
            </ConsoleShell>
        </Router>
    }
}
