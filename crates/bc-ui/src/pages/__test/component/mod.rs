//! Route entries for shared component QA pages (`/__test/component/*`).

pub mod num;
pub mod sparkline;
pub mod stat_card;
pub mod status_pill;
pub mod tag_token;
pub mod toml_view;

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;

/// All `/__test/component/*` routes.
#[component(transparent)]
pub fn ComponentRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/component") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=ComponentIndex />
            <Route path=path!("/sparkline") view=sparkline::SparklineQa />
            <Route path=path!("/stat-card") view=stat_card::StatCardQa />
            <Route path=path!("/num") view=num::NumQa />
            <Route path=path!("/status-pill") view=status_pill::StatusPillQa />
            <Route path=path!("/tag-token") view=tag_token::TagTokenQa />
            <Route path=path!("/toml-view") view=toml_view::TomlViewQa />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page listing all shared component QA pages.
#[component]
pub fn ComponentIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// components"</h1>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title=sparkline::TITLE
                    path=sparkline::PATH
                    description=sparkline::DESCRIPTION
                />
                <QaCard
                    title=stat_card::TITLE
                    path=stat_card::PATH
                    description=stat_card::DESCRIPTION
                />
                <QaCard title=num::TITLE path=num::PATH description=num::DESCRIPTION />
                <QaCard
                    title=status_pill::TITLE
                    path=status_pill::PATH
                    description=status_pill::DESCRIPTION
                />
                <QaCard
                    title=tag_token::TITLE
                    path=tag_token::PATH
                    description=tag_token::DESCRIPTION
                />
                <QaCard
                    title=toml_view::TITLE
                    path=toml_view::PATH
                    description=toml_view::DESCRIPTION
                />
            </div>
        </div>
    }
}
