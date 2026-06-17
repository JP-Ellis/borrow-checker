//! QA page for [`super::BudgetRow`].

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RolloverPolicy;
use leptos::prelude::*;

use super::BudgetRow;
use crate::pages::budget::BudgetPageCtx;

/// Builds a leaf node with a given name, spent, and an explicit target.
fn leaf_with_target(id: &str, name: &str, spent: i64, target: i64, mixed: bool) -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id(id)
        .account_id("everyday")
        .account_name("Everyday")
        .depth(0)
        .name(name)
        .spent(Amount::new(spent, "AUD", 2))
        .effective_target(Amount::new(target, "AUD", 2))
        .native_period_label("monthly")
        .has_mixed_period(mixed)
        .rollover(RolloverPolicy::ResetToZero)
        .is_tracking_only(false)
        .build()
}

/// Builds a leaf node with no target (tracking-only or no allocation).
fn leaf_no_target(id: &str, name: &str, spent: i64, tracking: bool) -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id(id)
        .account_id("everyday")
        .account_name("Everyday")
        .depth(0)
        .name(name)
        .spent(Amount::new(spent, "AUD", 2))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .rollover(RolloverPolicy::ResetToZero)
        .is_tracking_only(tracking)
        .build()
}

/// Renders [`BudgetRow`] across multiple fixture states.
#[component]
pub fn BudgetRowQa() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    /* leaf-good: 52% of $800 target */
    let leaf_good = leaf_with_target("groceries", "Groceries", 41_600, 80_000, false);

    /* leaf-warn: 85% of target */
    let leaf_warn = leaf_with_target("dining", "Dining", 68_000, 80_000, false);

    /* leaf-bad: 120% of target */
    let leaf_bad = leaf_with_target("transport", "Transport", 96_000, 80_000, false);

    /* leaf-dim: target set but $0 spent */
    let leaf_dim = leaf_with_target("entertainment", "Entertainment", 0, 50_000, false);

    /* leaf-tracking: tracking-only (no target) */
    let leaf_tracking = leaf_no_target("subscriptions", "Subscriptions", 24_900, true);

    /* mixed-period badge: leaf with has_mixed_period */
    let leaf_mixed = leaf_with_target("rent", "Rent", 150_000, 200_000, true);

    /* parent-with-children: aggregates groceries + dining */
    let parent_node = BudgetTreeNode::builder()
        .id("food")
        .account_id("everyday")
        .account_name("Everyday")
        .depth(0)
        .name("Food")
        .spent(Amount::new(109_600, "AUD", 2))
        .effective_target(Amount::new(160_000, "AUD", 2))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .is_tracking_only(false)
        .children(vec![
            leaf_with_target("groceries-child", "Groceries", 41_600, 80_000, false),
            leaf_with_target("dining-child", "Dining", 68_000, 80_000, false),
        ])
        .build();

    view! {
        <div style="padding: 24px; max-width: 900px">
            <h2 style="font-size: var(--bc-text-body); margin-bottom: var(--bc-space-4)">
                "BudgetRow QA"
            </h2>

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-bottom: var(--bc-space-3)">
                "leaf-good (52% spent)"
            </p>
            <BudgetRow node=leaf_good />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-warn (85% spent)"
            </p>
            <BudgetRow node=leaf_warn />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-bad (120% spent)"
            </p>
            <BudgetRow node=leaf_bad />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-dim ($0 spent, target set)"
            </p>
            <BudgetRow node=leaf_dim />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-tracking (tracking-only)"
            </p>
            <BudgetRow node=leaf_tracking />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "mixed-period badge (click badge to expand)"
            </p>
            <BudgetRow node=leaf_mixed />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "parent-with-children (click chevron to collapse)"
            </p>
            <BudgetRow node=parent_node />
        </div>
    }
}
