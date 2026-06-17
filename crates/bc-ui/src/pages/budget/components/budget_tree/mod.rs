//! Budget allocation tree — renders the full list of budget rows.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "tree.module.scss");

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
    view! { <div class=style::root /> }
}
