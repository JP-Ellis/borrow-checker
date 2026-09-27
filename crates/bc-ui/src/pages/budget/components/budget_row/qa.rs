//! QA page for [`super::BudgetRow`].

use bc_ipc::Amount;
use bc_ipc::BudgetIntent;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RolloverPolicy;
use bc_ipc::RowKind;
use bc_ipc::Verdict;
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

/// Builds an `Unallocated` leftover row under an envelope.
fn unallocated_child(id: &str, spent: i64, target: i64) -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id(id)
        .kind(RowKind::Unallocated)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("\u{21b3} unallocated")
        .actual(Amount::new(Decimal::new(spent, 2), "AUD"))
        .target(Amount::new(Decimal::new(target, 2), "AUD"))
        .unallocated(Decimal::new(spent, 2))
        .verdict(Verdict::Good)
        .native_period_label("monthly")
        .build()
}

/// Builds an `Unbudgeted` leftover row under an account.
fn unbudgeted_child(id: &str, spent: i64) -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id(id)
        .kind(RowKind::Unbudgeted)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("\u{21b3} unbudgeted")
        .actual(Amount::new(Decimal::new(spent, 2), "AUD"))
        .unbudgeted(Decimal::new(spent, 2))
        .native_period_label("monthly")
        .build()
}

/// An envelope (`Food`) whose sub-budgets leave some of its target unclaimed.
fn envelope_with_unallocated() -> BudgetTreeNode {
    let dining = leaf_with_target("food-dining", "Dining", 15_000, 20_000, false);
    let groceries = leaf_with_target("food-groceries", "Groceries", 43_100, 50_000, false);
    let unallocated = unallocated_child("food-unallocated", 3_100, 10_000);
    BudgetTreeNode::builder()
        .id("food-envelope")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("Food")
        .actual(Amount::new(Decimal::new(61_200, 2), "AUD"))
        .target(Amount::new(Decimal::new(80_000, 2), "AUD"))
        .intent(BudgetIntent::Limit)
        .claimed(Decimal::new(58_100, 2))
        .unallocated(Decimal::new(3_100, 2))
        .verdict(Verdict::Warn)
        .native_period_label("monthly")
        .children(vec![dining, groceries, unallocated])
        .build()
}

/// An account (`Transport`) with no budget of its own and unbudgeted spend.
fn account_with_unbudgeted() -> BudgetTreeNode {
    let fuel = leaf_with_target("transport-fuel", "Fuel", 12_000, 15_000, false);
    let unbudgeted = unbudgeted_child("transport-unbudgeted", 4_000);
    BudgetTreeNode::builder()
        .id("transport-account")
        .kind(RowKind::Account)
        .default_intent(BudgetIntent::Limit)
        .account_id("transport")
        .label("Transport")
        .actual(Amount::new(Decimal::new(16_000, 2), "AUD"))
        .target(Amount::new(Decimal::new(15_000, 2), "AUD"))
        .claimed(Decimal::new(12_000, 2))
        .unbudgeted(Decimal::new(4_000, 2))
        .worst_descendant(Verdict::Bad)
        .native_period_label("monthly")
        .children(vec![fuel, unbudgeted])
        .build()
}

/// An `Income` type root whose children mix `Goal` and `Estimate` intents.
fn income_root() -> BudgetTreeNode {
    let salary = BudgetTreeNode::builder()
        .id("income-salary")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Goal)
        .account_id("income")
        .label("Salary")
        .actual(Amount::new(Decimal::new(-318_800, 2), "AUD"))
        .target(Amount::new(Decimal::new(-650_000, 2), "AUD"))
        .intent(BudgetIntent::Goal)
        .claimed(Decimal::new(318_800, 2))
        .verdict(Verdict::Good)
        .native_period_label("monthly")
        .build();
    let interest = BudgetTreeNode::builder()
        .id("income-interest")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Goal)
        .account_id("income")
        .label("Interest")
        .actual(Amount::new(Decimal::new(-22_200, 2), "AUD"))
        .target(Amount::new(Decimal::new(-49_315, 2), "AUD"))
        .intent(BudgetIntent::Estimate)
        .claimed(Decimal::new(22_200, 2))
        .verdict(Verdict::Warn)
        .native_period_label("monthly")
        .build();
    BudgetTreeNode::builder()
        .id("income-root")
        .kind(RowKind::Account)
        .default_intent(BudgetIntent::Goal)
        .account_id("income")
        .label("Income")
        .actual(Amount::new(Decimal::new(-341_000, 2), "AUD"))
        .target(Amount::new(Decimal::new(-699_315, 2), "AUD"))
        .claimed(Decimal::new(341_000, 2))
        .native_period_label("monthly")
        .children(vec![salary, interest])
        .build()
}

/// An account (`Pets`) whose unbudgeted spend overflows the bar.
fn overflowing_account() -> BudgetTreeNode {
    let grooming = leaf_with_target("pets-grooming", "Grooming", 9_500, 8_000, false);
    let unbudgeted = unbudgeted_child("pets-unbudgeted", 12_000);
    BudgetTreeNode::builder()
        .id("pets-account")
        .kind(RowKind::Account)
        .default_intent(BudgetIntent::Limit)
        .account_id("pets")
        .label("Pets")
        .actual(Amount::new(Decimal::new(21_500, 2), "AUD"))
        .target(Amount::new(Decimal::new(8_000, 2), "AUD"))
        .claimed(Decimal::new(9_500, 2))
        .unbudgeted(Decimal::new(12_000, 2))
        .worst_descendant(Verdict::Bad)
        .native_period_label("monthly")
        .children(vec![grooming, unbudgeted])
        .build()
}

/// Renders [`BudgetRow`] across multiple fixture states.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "a QA showcase page listing every fixture state in one view"
)]
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
            <BudgetRow node=leaf_good depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-warn (85% spent)"
            </p>
            <BudgetRow node=leaf_warn depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-bad (120% spent)"
            </p>
            <BudgetRow node=leaf_bad depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-dim ($0 spent, target set)"
            </p>
            <BudgetRow node=leaf_dim depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-tracking (tracking-only)"
            </p>
            <BudgetRow node=leaf_tracking depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "mixed-period badge (click badge to expand)"
            </p>
            <BudgetRow node=leaf_mixed depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "leaf-unvalued (amber pill lists the excluded spend)"
            </p>
            <BudgetRow node=leaf_unvalued depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "parent-with-children (click chevron to collapse)"
            </p>
            <BudgetRow node=parent_node depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "envelope-with-unallocated (Food; \u{21b3} unallocated leftover row)"
            </p>
            <BudgetRow
                node=envelope_with_unallocated()
                depth=0
                parent_label=None
                parent_target=None
            />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "account-with-unbudgeted (Transport; \u{21b3} unbudgeted leftover row)"
            </p>
            <BudgetRow
                node=account_with_unbudgeted()
                depth=0
                parent_label=None
                parent_target=None
            />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "income-root (mixed Goal + Estimate intents)"
            </p>
            <BudgetRow node=income_root() depth=0 parent_label=None parent_target=None />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "overflowing-bar (Pets; unbudgeted spend runs off the bar)"
            </p>
            <BudgetRow node=overflowing_account() depth=0 parent_label=None parent_target=None />
        </div>
    }
}
