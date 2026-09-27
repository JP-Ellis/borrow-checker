//! QA page for [`super::BudgetRow`].

use bc_ipc::Amount;
use bc_ipc::BudgetIntent;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RolloverPolicy;
use bc_ipc::RowKind;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::BudgetRow;
use crate::pages::budget::BudgetPageCtx;

/// Builds a leaf node with a given name, spent, and an explicit target.
fn leaf_with_target(id: &str, name: &str, spent: i64, target: i64, mixed: bool) -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id(id)
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label(name)
        .actual(Amount::new(Decimal::new(spent, 2), "AUD"))
        .target(Amount::new(Decimal::new(target, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(mixed)
        .rollover(RolloverPolicy::ResetToZero)
        .build()
}

/// Builds a tracking-only leaf node (a budget without a target).
fn leaf_no_target(id: &str, name: &str, spent: i64) -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id(id)
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label(name)
        .actual(Amount::new(Decimal::new(spent, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .rollover(RolloverPolicy::ResetToZero)
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
    let leaf_tracking = leaf_no_target("subscriptions", "Subscriptions", 24_900);

    /* mixed-period badge: leaf with has_mixed_period */
    let leaf_mixed = leaf_with_target("rent", "Rent", 150_000, 200_000, true);

    /* leaf-unvalued: an AUD budget with a USD posting no rate could value */
    let leaf_unvalued = BudgetTreeNode::builder()
        .id("imports")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("Imports")
        .actual(Amount::new(Decimal::new(12_000, 2), "AUD"))
        .target(Amount::new(Decimal::new(50_000, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .rollover(RolloverPolicy::ResetToZero)
        .unvalued(vec![Amount::new(Decimal::new(4_500, 2), "USD")])
        .build();

    /* parent-with-children: aggregates groceries + dining */
    let parent_node = BudgetTreeNode::builder()
        .id("food")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("Food")
        .actual(Amount::new(Decimal::new(109_600, 2), "AUD"))
        .target(Amount::new(Decimal::new(160_000, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(false)
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
            <BudgetRow node=leaf_good depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-warn (85% spent)"
            </p>
            <BudgetRow node=leaf_warn depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-bad (120% spent)"
            </p>
            <BudgetRow node=leaf_bad depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-dim ($0 spent, target set)"
            </p>
            <BudgetRow node=leaf_dim depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-tracking (tracking-only)"
            </p>
            <BudgetRow node=leaf_tracking depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "mixed-period badge (click badge to expand)"
            </p>
            <BudgetRow node=leaf_mixed depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-unvalued (amber pill lists the excluded spend)"
            </p>
            <BudgetRow node=leaf_unvalued depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "parent-with-children (click chevron to collapse)"
            </p>
            <BudgetRow node=parent_node depth=0 />
        </div>
    }
}
