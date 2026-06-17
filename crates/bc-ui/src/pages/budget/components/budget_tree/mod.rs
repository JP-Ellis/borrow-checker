//! Budget allocation tree — renders the full list of budget rows.

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

/// Renders the complete hierarchy of budget allocation rows.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    unused_variables,
    reason = "stub — props will be consumed in later tasks"
)]
pub fn BudgetTree(
    /// Flat list of tree nodes returned by the overview IPC call.
    nodes: Vec<BudgetTreeNode>,
) -> impl IntoView {
    view! { <div class="budget-tree" /> }
}
