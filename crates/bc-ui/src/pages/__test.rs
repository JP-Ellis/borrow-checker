//! QA test pages — available only in debug builds via `/__test/*` routes.

pub mod component;
pub mod fundamentals;
pub mod index;
pub mod page;
pub mod shell;

pub use index::QaIndex;
use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

/// All `/__test/*` routes wrapped in [`shell::QaShell`].
#[component(transparent)]
pub fn TestRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/__test") view=shell::QaShell>
            <Route path=path!("") view=QaIndex />
            <component::ComponentRoutes />
            <page::PageRoutes />
            <fundamentals::FundamentalsRoutes />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}
