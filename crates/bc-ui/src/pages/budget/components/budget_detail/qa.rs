//! QA page for [`super::BudgetDetail`].

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;

use super::BudgetDetail;
use crate::pages::budget::BudgetPageCtx;

/// Constructs a sample budget tree node for QA display.
fn sample_node() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("groceries")
        .account_id("everyday")
        .account_name("Everyday")
        .depth(0)
        .name("Groceries")
        .effective_target(Amount::from_minor(80_000, "AUD", 2))
        .spent(Amount::from_minor(52_300, "AUD", 2))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .rollover(bc_ipc::RolloverPolicy::ResetToZero)
        .is_tracking_only(false)
        .build()
}

/// Constructs a tracking-only sample node (no target, no rollover).
fn tracking_node() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("utilities")
        .account_id("bills")
        .account_name("Bills")
        .depth(0)
        .name("Utilities")
        .spent(Amount::from_minor(15_000, "AUD", 2))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .is_tracking_only(true)
        .build()
}

/// Constructs a sample node with a tag filter applied.
fn tagged_node() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("person-me-food")
        .account_id("everyday")
        .account_name("Everyday")
        .depth(0)
        .name("My Food")
        .effective_target(Amount::from_minor(40_000, "AUD", 2))
        .spent(Amount::from_minor(38_500, "AUD", 2))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .rollover(bc_ipc::RolloverPolicy::CarryForward)
        .tag_filter("person:me")
        .is_tracking_only(false)
        .build()
}

/// Renders [`BudgetDetail`] in three realistic states: normal, tracking-only, and tagged.
#[component]
pub fn BudgetDetailQa() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    view! {
        <div style="padding:24px; display:flex; flex-direction:column; gap:32px;">
            <div>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Normal budget — with target, rollover=ResetToZero"
                </p>
                <BudgetDetail node=sample_node() />
            </div>
            <div>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Tracking-only — no target, no rollover"
                </p>
                <BudgetDetail node=tracking_node() />
            </div>
            <div>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Tagged sub-budget — tag_filter=person:me, rollover=CarryForward"
                </p>
                <BudgetDetail node=tagged_node() />
            </div>
        </div>
    }
}
