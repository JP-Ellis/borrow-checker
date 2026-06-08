//! Route entries for plugins page QA (`/__test/page/plugins/*`).

/// Display name shown in the QA index.
pub const TITLE: &str = "plugins";
/// Route path.
pub const PATH: &str = "/__test/page/plugins";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Plugins page: empty state, populated table, and error state.";

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;
use crate::pages::plugins::qa::PluginsEmptyQa;
use crate::pages::plugins::qa::PluginsErrorQa;
use crate::pages::plugins::qa::PluginsFullQa;

/// All `/__test/page/plugins/*` routes.
#[component(transparent)]
pub fn PluginsRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/plugins") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=PluginsIndex />
            <Route path=path!("/empty") view=PluginsEmptyQa />
            <Route path=path!("/full") view=PluginsFullQa />
            <Route path=path!("/error") view=PluginsErrorQa />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page listing all plugins page QA pages.
#[component]
pub fn PluginsIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// pages / plugins"</h1>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title="Plugins (empty)"
                    path="/__test/page/plugins/empty"
                    description="Empty state — no plugins installed."
                />
                <QaCard
                    title="Plugins (full)"
                    path="/__test/page/plugins/full"
                    description="Populated table — one current plugin and one deprecated plugin."
                />
                <QaCard
                    title="Plugins (error)"
                    path="/__test/page/plugins/error"
                    description="Error state — IPC command returned an error."
                />
            </div>
        </div>
    }
}
