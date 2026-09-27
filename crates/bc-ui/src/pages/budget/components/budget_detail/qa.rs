//! QA page for [`super::BudgetDetail`].

use bc_ipc::Amount;
use bc_ipc::BudgetIntent;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RowKind;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::BudgetDetail;
use crate::pages::budget::BudgetPageCtx;

/// Constructs a sample budget tree node for QA display.
fn sample_node() -> BudgetTreeNode {
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
        .rollover(bc_ipc::RolloverPolicy::ResetToZero)
        .build()
}

/// Constructs a tracking-only sample node (no target, no rollover).
fn tracking_node() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("utilities")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("bills")
        .label("Utilities")
        .actual(Amount::new(Decimal::new(15_000, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .build()
}

/// Constructs a sample node with a tag filter applied.
fn tagged_node() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("person-me-food")
        .kind(RowKind::Budget)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("My Food")
        .target(Amount::new(Decimal::new(40_000, 2), "AUD"))
        .actual(Amount::new(Decimal::new(38_500, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(false)
        .rollover(bc_ipc::RolloverPolicy::CarryForward)
        .tag_filter("person:me")
        .build()
}

/// Constructs a sample `↳ unallocated` leftover row: no revisions or actions,
/// only a transactions column headed by its tooltip text.
fn unallocated_node() -> BudgetTreeNode {
    BudgetTreeNode::builder()
        .id("unalloc:groceries")
        .kind(RowKind::Unallocated)
        .default_intent(BudgetIntent::Limit)
        .account_id("everyday")
        .label("\u{21b3} unallocated")
        .actual(Amount::new(Decimal::new(12_000, 2), "AUD"))
        .native_period_label("monthly")
        .has_mixed_period(false)
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
            <div>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "Leftover row — no revisions, no actions, transactions column only"
                </p>
                <BudgetDetail
                    node=unallocated_node()
                    leftover_title="Groceries's budget not claimed by a sub-budget"
                />
            </div>
        </div>
    }
}
