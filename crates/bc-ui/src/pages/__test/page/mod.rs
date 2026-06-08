//! Route entries for page-level QA pages (`/__test/page/*`).

pub mod accounts;
pub mod plugins;
pub mod settings;

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;

/// All `/__test/page/*` routes.
#[component(transparent)]
pub fn PageRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/page") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=PageIndex />
            <accounts::AccountsRoutes />
            <plugins::PluginsRoutes />
            <settings::SettingsRoutes />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page listing all page QA sub-sections.
#[component]
pub fn PageIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// pages"</h1>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title=accounts::TITLE
                    path=accounts::PATH
                    description=accounts::DESCRIPTION
                />
                <QaCard title=plugins::TITLE path=plugins::PATH description=plugins::DESCRIPTION />
                <QaCard
                    title=settings::TITLE
                    path=settings::PATH
                    description=settings::DESCRIPTION
                />
            </div>
        </div>
    }
}
