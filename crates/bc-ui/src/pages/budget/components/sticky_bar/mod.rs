//! Sticky period-control bar that remains visible while scrolling.

use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

/// Period selector and mode-toggle bar fixed below the page header.
#[component]
#[expect(unused_variables, reason = "stub — used in later tasks")]
pub fn StickyBar(
    /// Budget overview resource supplying the summary and tree.
    overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>>,
) -> impl IntoView {
    view! { <div class="budget-sticky-bar" /> }
}
