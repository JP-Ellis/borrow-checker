//! QA page for [`super::BudgetTree`].

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RolloverPolicy;
use leptos::prelude::*;

use super::BudgetTree;

/// Returns an empty node list (zero-budget state).
fn empty_nodes() -> Vec<BudgetTreeNode> {
    vec![]
}

/// Returns a small set of nodes: one under-budget, one over-budget.
fn two_nodes() -> Vec<BudgetTreeNode> {
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
            .rollover(RolloverPolicy::ResetToZero)
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
            .rollover(RolloverPolicy::ResetToZero)
            .is_tracking_only(false)
            .build(),
    ]
}

/// Returns three nodes: under-budget, over-budget, and a tracking-only row.
fn three_nodes() -> Vec<BudgetTreeNode> {
    let mut nodes = two_nodes();
    nodes.push(
        BudgetTreeNode::builder()
            .id("transport")
            .account_id("expenses")
            .account_name("Expenses")
            .depth(0)
            .name("Transport")
            .effective_target(Amount::new(15_000, "AUD", 2))
            .spent(Amount::new(7_400, "AUD", 2))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(RolloverPolicy::CarryForward)
            .is_tracking_only(true)
            .build(),
    );
    nodes
}

/// Renders [`BudgetTree`] across three realistic states.
///
/// States shown:
/// - Empty tree (no budgets configured)
/// - Two nodes (one under-budget, one over-budget)
/// - Three nodes (adds a tracking-only row)
#[component]
pub fn BudgetTreeQa() -> impl IntoView {
    view! {
        <div style="padding:24px;display:flex;flex-direction:column;gap:32px;">
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Empty tree — no budgets configured"
                </p>
                <BudgetTree nodes=empty_nodes() />
            </section>
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Two nodes — one under-budget (Groceries), one over-budget (Dining Out)"
                </p>
                <BudgetTree nodes=two_nodes() />
            </section>
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Three nodes — adds a tracking-only carry-over row (Transport)"
                </p>
                <BudgetTree nodes=three_nodes() />
            </section>
        </div>
    }
}
