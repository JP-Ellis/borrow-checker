//! Budget allocation tree — renders the full list of budget rows.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::components::budget_row::BudgetRow;

import_style!(style, "tree.module.scss");

/// Renders the complete hierarchy of budget allocation rows.
#[component]
pub fn BudgetTree(
    /// Flat list of tree nodes returned by the overview IPC call.
    nodes: Vec<BudgetTreeNode>,
) -> impl IntoView {
    view! {
        <div class=style::tree aria-label="budget tree">
            <div class=style::col_headers>
                <span class=style::col_account>"ACCOUNT"</span>
                <span class=style::col_progress>"PROGRESS"</span>
                <span class=style::col_amounts>"SPENT / TARGET"</span>
            </div>
            <For
                each=move || nodes.clone()
                key=|node| format!("{}:{}", node.account_id, node.depth)
                children=move |node| view! { <BudgetRow node=node /> }
            />
        </div>
    }
}
