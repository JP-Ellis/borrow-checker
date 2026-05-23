//! Route entries for design-system fundamentals QA pages (`/__test/fundamentals/*`).

pub mod colour;
pub mod geometry;
pub mod typography;

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;

/// All `/__test/fundamentals/*` routes.
#[component(transparent)]
pub fn FundamentalsRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/fundamentals") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=FundamentalsIndex />
            <Route path=path!("/typography") view=typography::TypographyFundamentals />
            <Route path=path!("/colour") view=colour::ColourFundamentals />
            <Route path=path!("/geometry") view=geometry::GeometryFundamentals />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page listing all design fundamentals QA pages.
#[component]
pub fn FundamentalsIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;\
            color:var(--bc-ink-mute);margin-bottom:24px;">"// fundamentals"</h1>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title=typography::TITLE
                    path=typography::PATH
                    description=typography::DESCRIPTION
                />
                <QaCard title=colour::TITLE path=colour::PATH description=colour::DESCRIPTION />
                <QaCard
                    title=geometry::TITLE
                    path=geometry::PATH
                    description=geometry::DESCRIPTION
                />
            </div>
        </div>
    }
}
