//! Single row in the budget allocation tree.
// Stub only — node field is intentionally unused until this component is implemented.
#![allow(
    dead_code,
    reason = "stub component — fields will be read when this component is implemented"
)]

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

/// One row in the budget allocation grid, representing a single budget line.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    unused_variables,
    reason = "stub — props will be consumed in later tasks"
)]
pub fn BudgetRow(
    /// The tree node this row represents.
    node: BudgetTreeNode,
) -> impl IntoView {
    view! { <div class="budget-row" /> }
}
