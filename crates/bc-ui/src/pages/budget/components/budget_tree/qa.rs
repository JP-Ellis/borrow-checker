//! QA page for [`super::BudgetTree`].

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

use super::BudgetTree;

/// Constructs a flat list of sample budget tree nodes for QA display.
fn sample_nodes() -> Vec<BudgetTreeNode> {
    vec![
        BudgetTreeNode::builder()
            .id("groceries")
            .account_id("everyday")
            .account_name("Everyday")
            .depth(0)
            .name("Groceries")
            .effective_target(Amount::new(80_000, "AUD", 2))
            .spent(Amount::new(52_300, "AUD", 2))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(bc_ipc::RolloverPolicy::ResetToZero)
            .is_tracking_only(false)
            .build(),
        BudgetTreeNode::builder()
            .id("dining")
            .account_id("everyday")
            .account_name("Everyday")
            .depth(0)
            .name("Dining Out")
            .effective_target(Amount::new(30_000, "AUD", 2))
            .spent(Amount::new(31_200, "AUD", 2))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(bc_ipc::RolloverPolicy::ResetToZero)
            .is_tracking_only(false)
            .build(),
    ]
}

/// Renders [`BudgetTree`] with fixture nodes in stub state.
///
/// Note: this component is a stub. The QA page will be expanded when the
/// component is implemented in a later task.
#[component]
pub fn BudgetTreeQa() -> impl IntoView {
    view! {
        <div style="padding:24px">
            <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                "stub — will be expanded when BudgetTree is implemented"
            </p>
            <BudgetTree nodes=sample_nodes() />
        </div>
    }
}
