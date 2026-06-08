//! Route entries for accounts page QA (`/__test/page/accounts/*`).

pub mod component;
pub mod full;
pub mod hero;

/// Display name shown in the QA index.
pub const TITLE: &str = "accounts";
/// Route path.
pub const PATH: &str = "/__test/page/accounts";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Full account view and sub-components.";

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;

/// All `/__test/page/accounts/*` routes.
#[component(transparent)]
pub fn AccountsRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/accounts") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=AccountsIndex />
            <Route path=path!("/hero") view=hero::AccountDashboardQa />
            <Route path=path!("/full") view=full::AccountFullQa />
            <Route
                path=path!("/transaction-row")
                view=component::transaction_row::TransactionRowQa
            />
            <Route
                path=path!("/transaction-register")
                view=component::transaction_register::TransactionRegisterQa
            />
            <Route path=path!("/sidebar") view=component::sidebar::AccountSidebarQa />
            <Route path=path!("/sticky-bar") view=component::sticky_bar::StickyAccountBarQa />
            <Route
                path=path!("/add-transaction")
                view=component::add_transaction::AddTransactionFormQa
            />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page listing all accounts page QA pages.
#[component]
pub fn AccountsIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// pages / accounts"</h1>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;margin-bottom:32px;">
                <QaCard title=full::TITLE path=full::PATH description=full::DESCRIPTION />
            </div>
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin-bottom:12px;letter-spacing:.08em;text-transform:uppercase;">"sub-components"</h2>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard title=hero::TITLE path=hero::PATH description=hero::DESCRIPTION />
                <QaCard
                    title=component::transaction_row::TITLE
                    path=component::transaction_row::PATH
                    description=component::transaction_row::DESCRIPTION
                />
                <QaCard
                    title=component::transaction_register::TITLE
                    path=component::transaction_register::PATH
                    description=component::transaction_register::DESCRIPTION
                />
                <QaCard
                    title=component::sidebar::TITLE
                    path=component::sidebar::PATH
                    description=component::sidebar::DESCRIPTION
                />
                <QaCard
                    title=component::sticky_bar::TITLE
                    path=component::sticky_bar::PATH
                    description=component::sticky_bar::DESCRIPTION
                />
                <QaCard
                    title=component::add_transaction::TITLE
                    path=component::add_transaction::PATH
                    description=component::add_transaction::DESCRIPTION
                />
            </div>
        </div>
    }
}
