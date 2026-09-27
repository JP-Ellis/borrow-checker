//! QA page for [`super::BudgetTree`].

use bc_ipc::Amount;
use bc_ipc::BudgetIntent;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RolloverPolicy;
use bc_ipc::RowKind;
use leptos::prelude::*;
use rust_decimal::Decimal;

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
            .kind(RowKind::Budget)
            .default_intent(BudgetIntent::Limit)
            .account_id("everyday")
            .label("Groceries")
            .target(Amount::new(Decimal::new(80_000, 2), "AUD"))
            .actual(Amount::new(Decimal::new(52_300, 2), "AUD"))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(RolloverPolicy::ResetToZero)
            .build(),
        BudgetTreeNode::builder()
            .id("dining")
            .kind(RowKind::Budget)
            .default_intent(BudgetIntent::Limit)
            .account_id("everyday")
            .label("Dining Out")
            .target(Amount::new(Decimal::new(30_000, 2), "AUD"))
            .actual(Amount::new(Decimal::new(31_200, 2), "AUD"))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(RolloverPolicy::ResetToZero)
            .build(),
    ]
}

/// Returns three nodes: under-budget, over-budget, and a tracking-only row.
fn three_nodes() -> Vec<BudgetTreeNode> {
    let mut nodes = two_nodes();
    nodes.push(
        BudgetTreeNode::builder()
            .id("transport")
            .kind(RowKind::Budget)
            .default_intent(BudgetIntent::Limit)
            .account_id("expenses")
            .label("Transport")
            .actual(Amount::new(Decimal::new(7_400, 2), "AUD"))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(RolloverPolicy::CarryForward)
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
