//! Expandable detail panel for a single budget line.
// Stub only — node field is intentionally unused until this component is implemented.
#![allow(
    dead_code,
    reason = "stub component — fields will be read when this component is implemented"
)]

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "detail.module.scss");

/// Expanded detail panel showing transactions and accrual data for a budget.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    unused_variables,
    reason = "stub — props will be consumed in later tasks"
)]
pub fn BudgetDetail(
    /// The tree node whose detail is being displayed.
    node: BudgetTreeNode,
) -> impl IntoView {
    view! { <div class=style::root /> }
}
