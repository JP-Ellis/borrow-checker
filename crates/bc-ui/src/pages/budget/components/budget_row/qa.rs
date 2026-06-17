//! QA page for [`super::BudgetRow`].

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

use super::BudgetRow;

/// Constructs a sample budget tree node for QA display.
fn sample_node() -> BudgetTreeNode {
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
        .build()
}

/// Renders [`BudgetRow`] with fixture data in stub state.
///
/// Note: this component is a stub. The QA page will be expanded when the
/// component is implemented in a later task.
#[component]
pub fn BudgetRowQa() -> impl IntoView {
    view! {
        <div style="padding:24px">
            <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                "stub — will be expanded when BudgetRow is implemented"
            </p>
            <BudgetRow node=sample_node() />
        </div>
    }
}
