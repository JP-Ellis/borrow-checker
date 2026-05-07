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
///
/// In debug builds, additional QA routes are registered under `/__test/*`.
#[cfg(not(debug_assertions))]
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

/// Root application component with additional QA routes under `/__test/*`.
#[cfg(debug_assertions)]
#[component]
#[expect(
    clippy::absolute_paths,
    reason = "leptos_router path! macro expansion emits absolute paths we cannot control"
)]
pub fn App() -> impl IntoView {
    use crate::pages::__test;

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
                    <Route path=path!("/__test") view=__test::Root />
                    <Route path=path!("/__test/num") view=__test::NumTest />
                    <Route path=path!("/__test/status-pill") view=__test::StatusPillTest />
                    <Route path=path!("/__test/tag-token") view=__test::TagTokenTest />
                </Routes>
            </ConsoleShell>
        </Router>
    }
}
