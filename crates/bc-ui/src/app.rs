//! Root application component with client-side router.

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::components::ParentRoute;
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

/// Debug-only routes, or an empty route set in release builds.
#[component(transparent)]
fn DebugRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    cfg_select! {
        debug_assertions => { crate::pages::__test::TestRoutes() }
        _ => { () }
    }
}

/// Root application component. Mounts the router and wraps all routes in
/// [`ConsoleShell`].
///
/// In debug builds, additional QA routes are registered under `/__test/*`
/// wrapped in a [`crate::pages::__test::shell::QaShell`] layout route.
#[component]
#[expect(
    clippy::absolute_paths,
    reason = "leptos_router path! macro expansion emits absolute paths we cannot control"
)]
pub fn App() -> impl IntoView {
    view! {
        <Router>
            <Routes fallback=|| view! { <p class="not-found">"page not found"</p> }>
                <ParentRoute path=path!("/") view=ConsoleShell>
                    <Route path=path!("/") view=Dashboard />
                    <Route path=path!("/accounts") view=Accounts />
                    <Route path=path!("/accounts/:id") view=Accounts />
                    <Route path=path!("/budget") view=Budget />
                    <Route path=path!("/reports") view=Reports />
                    <Route path=path!("/plugins") view=Plugins />
                    <Route path=path!("/settings") view=Settings />
                </ParentRoute>
                <DebugRoutes />
            </Routes>
        </Router>
    }
}
