//! QA page for [`super::BudgetTree`].

use bc_ipc::Amount;
use bc_ipc::BudgetIntent;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RolloverPolicy;
use bc_ipc::RowKind;
use bc_ipc::Verdict;
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

/// Returns an envelope (`Food`) whose sub-budgets leave spend `↳ unallocated`.
fn food_envelope_with_unallocated() -> BudgetTreeNode {
    let dining = BudgetTreeNode::builder()
        .id("food-dining")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("Dining")
        .actual(Amount::new(Decimal::new(15_000, 2), "AUD"))
        .target(Amount::new(Decimal::new(20_000, 2), "AUD"))
        .intent(BudgetIntent::Limit)
        .verdict(Verdict::Good)
        .native_period_label("monthly")
        .rollover(RolloverPolicy::ResetToZero)
        .build();
    let groceries = BudgetTreeNode::builder()
        .id("food-groceries")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("Groceries")
        .actual(Amount::new(Decimal::new(43_100, 2), "AUD"))
        .target(Amount::new(Decimal::new(50_000, 2), "AUD"))
        .intent(BudgetIntent::Limit)
        .verdict(Verdict::Good)
        .native_period_label("monthly")
        .rollover(RolloverPolicy::ResetToZero)
        .build();
    let unallocated = BudgetTreeNode::builder()
        .id("food-unallocated")
        .kind(RowKind::Unallocated)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("\u{21b3} unallocated")
        .actual(Amount::new(Decimal::new(3_100, 2), "AUD"))
        .target(Amount::new(Decimal::new(10_000, 2), "AUD"))
        .unallocated(Decimal::new(3_100, 2))
        .verdict(Verdict::Good)
        .native_period_label("monthly")
        .build();
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

/// Returns an account (`Pets`) whose `↳ unbudgeted` spend overflows the bar.
fn pets_account_overflowing() -> BudgetTreeNode {
    let grooming = BudgetTreeNode::builder()
        .id("pets-grooming")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("pets")
        .label("Grooming")
        .actual(Amount::new(Decimal::new(9_500, 2), "AUD"))
        .target(Amount::new(Decimal::new(8_000, 2), "AUD"))
        .intent(BudgetIntent::Limit)
        .verdict(Verdict::Bad)
        .native_period_label("monthly")
        .rollover(RolloverPolicy::ResetToZero)
        .build();
    let pets_unbudgeted = BudgetTreeNode::builder()
        .id("pets-unbudgeted")
        .kind(RowKind::Unbudgeted)
        .default_intent(BudgetIntent::Limit)
        .account_id("pets")
        .label("\u{21b3} unbudgeted")
        .actual(Amount::new(Decimal::new(12_000, 2), "AUD"))
        .unbudgeted(Decimal::new(12_000, 2))
        .native_period_label("monthly")
        .build();
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
        .children(vec![grooming, pets_unbudgeted])
        .build()
}

/// Returns an `Expenses` root: an envelope (`Food`) whose sub-budgets leave
/// spend `↳ unallocated`, and an account (`Pets`) whose `↳ unbudgeted` spend
/// overflows the bar.
fn expenses_root_with_leftovers() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("expenses-root")
        .kind(RowKind::Account)
        .default_intent(BudgetIntent::Limit)
        .account_id("expenses")
        .label("Expenses")
        .actual(Amount::new(Decimal::new(826_700, 2), "AUD"))
        .target(Amount::new(Decimal::new(1_070_000, 2), "AUD"))
        .claimed(Decimal::new(58_100, 2))
        .native_period_label("monthly")
        .children(vec![
            food_envelope_with_unallocated(),
            pets_account_overflowing(),
        ])
        .build()
}

/// Returns an `Income` root whose children mix `Goal` and `Estimate` intents.
fn income_root_mixed_intents() -> BudgetTreeNode {
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

/// Returns the `Expenses` and `Income` roots for the leftover/overflow states.
fn leftover_and_overflow_nodes() -> Vec<BudgetTreeNode> {
    vec![expenses_root_with_leftovers(), income_root_mixed_intents()]
}

/// Renders [`BudgetTree`] across four realistic states.
///
/// States shown:
/// - Empty tree (no budgets configured)
/// - Two nodes (one under-budget, one over-budget)
/// - Three nodes (adds a tracking-only row)
/// - Leftover rows and overflow (envelope with `↳ unallocated`, account with
///   an overflowing `↳ unbudgeted`, and an `Income` root mixing intents)
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
            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Leftover rows and overflow — Expenses (Food's \u{21b3} unallocated, Pets' overflowing \u{21b3} unbudgeted) and Income (mixed Goal/Estimate intents)"
                </p>
                <BudgetTree nodes=leftover_and_overflow_nodes() />
            </section>
        </div>
    }
}
