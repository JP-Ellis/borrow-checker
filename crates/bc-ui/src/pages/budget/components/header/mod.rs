//! Budget page header — displays KPI summary cards.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "header.module.scss");

/// Header strip showing top-level budget KPIs (total budgeted, spent, remaining).
#[component]
#[expect(unused_variables, reason = "stub — used in later tasks")]
pub fn BudgetHeader(
    /// Budget overview resource supplying the summary and tree.
    overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>>,
) -> impl IntoView {
    view! { <div class=style::root /> }
}
