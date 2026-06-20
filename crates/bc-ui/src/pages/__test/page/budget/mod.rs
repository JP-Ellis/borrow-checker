//! Route entries for budget page QA (`/__test/page/budget/*`).

pub mod component;

/// Display name shown in the QA index.
pub const TITLE: &str = "budget";
/// Route path.
pub const PATH: &str = "/__test/page/budget";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Budget page and sub-components.";

use leptos::prelude::*;
use leptos_router::MatchNestedRoutes;
use leptos_router::any_nested_route::IntoAnyNestedRoute as _;
use leptos_router::components::Outlet;
use leptos_router::components::ParentRoute;
use leptos_router::components::Route;
use leptos_router::path;

use crate::pages::__test::index::QaCard;

/// All `/__test/page/budget/*` routes.
#[component(transparent)]
pub fn BudgetRoutes() -> impl MatchNestedRoutes + Clone + Send + 'static {
    view! {
        <ParentRoute path=path!("/budget") view=|| view! { <Outlet /> }>
            <Route path=path!("") view=BudgetIndex />
            <Route path=path!("/header") view=component::header::BudgetHeaderQa />
            <Route path=path!("/sticky-bar") view=component::sticky_bar::StickyBarQa />
            <Route path=path!("/budget-tree") view=component::budget_tree::BudgetTreeQa />
            <Route path=path!("/budget-row") view=component::budget_row::BudgetRowQa />
            <Route path=path!("/budget-detail") view=component::budget_detail::BudgetDetailQa />
            <Route
                path=path!("/native-period-list")
                view=component::native_period_list::NativePeriodListQa
            />
            <Route path=path!("/accrual-editor") view=component::accrual_editor::AccrualEditorQa />
            <Route path=path!("/revision-form") view=component::revision_form::RevisionFormQa />
        </ParentRoute>
    }
    .into_inner()
    .into_any_nested_route()
}

/// Index page listing all budget page QA pages.
#[component]
pub fn BudgetIndex() -> impl IntoView {
    view! {
        <div style="padding:24px;max-width:960px">
            <h1 style="font-family:var(--bc-font-mono);font-size:14px;color:var(--bc-ink-mute);\
            margin-bottom:24px;">"// pages / budget"</h1>
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin-bottom:12px;letter-spacing:.08em;text-transform:uppercase;">"sub-components"</h2>
            <div style="display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));\
            gap:8px;">
                <QaCard
                    title=component::header::TITLE
                    path=component::header::PATH
                    description=component::header::DESCRIPTION
                />
                <QaCard
                    title=component::sticky_bar::TITLE
                    path=component::sticky_bar::PATH
                    description=component::sticky_bar::DESCRIPTION
                />
                <QaCard
                    title=component::budget_tree::TITLE
                    path=component::budget_tree::PATH
                    description=component::budget_tree::DESCRIPTION
                />
                <QaCard
                    title=component::budget_row::TITLE
                    path=component::budget_row::PATH
                    description=component::budget_row::DESCRIPTION
                />
                <QaCard
                    title=component::budget_detail::TITLE
                    path=component::budget_detail::PATH
                    description=component::budget_detail::DESCRIPTION
                />
                <QaCard
                    title=component::native_period_list::TITLE
                    path=component::native_period_list::PATH
                    description=component::native_period_list::DESCRIPTION
                />
                <QaCard
                    title=component::accrual_editor::TITLE
                    path=component::accrual_editor::PATH
                    description=component::accrual_editor::DESCRIPTION
                />
                <QaCard
                    title=component::revision_form::TITLE
                    path=component::revision_form::PATH
                    description=component::revision_form::DESCRIPTION
                />
            </div>
        </div>
    }
}
