//! Single row in the budget allocation tree.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::components::budget_detail::BudgetDetail;
use crate::pages::budget::components::native_period_list::NativePeriodList;

import_style!(style, "row.module.scss");

/// Row status derived from spend vs. target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    /// Spend is ≤ 80% of target.
    Good,
    /// Spend is > 80% but ≤ 100% of target.
    Warn,
    /// Spend exceeds the target.
    Bad,
    /// Budget has a target but nothing has been spent yet.
    Dim,
    /// No target, or tracking-only mode.
    Mute,
}

/// Derives a [`Status`] from the node's spend and target figures.
///
/// Uses integer arithmetic for the 80% threshold to avoid floating-point.
/// The comparison `spent * 5 > target * 4` is equivalent to `spent/target > 0.8`
/// and safe because budget minor-unit values fit comfortably in i64.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "budget minor-unit values are bounded and will not overflow i64"
)]
fn row_status(node: &BudgetTreeNode) -> Status {
    if node.is_tracking_only {
        return Status::Mute;
    }
    match &node.effective_target {
        None => Status::Mute,
        Some(_) if node.spent.minor_units == 0 => Status::Dim,
        Some(target) if node.spent.minor_units > target.minor_units => Status::Bad,
        Some(target) => {
            /* 80% check via integer: spent * 5 > target * 4 ↔ spent/target > 0.8 */
            if node.spent.minor_units * 5 > target.minor_units * 4 {
                Status::Warn
            } else {
                Status::Good
            }
        }
    }
}

/// Progress bar fill percentage (0–100), computed with integer arithmetic.
///
/// Returns 0 when there is no target or when target minor-units are zero.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    clippy::as_conversions,
    clippy::cast_sign_loss,
    reason = "budget minor-unit arithmetic is bounded; .max(0) guarantees non-negative before cast; integer division for percentage is intentional; clamped to [0,100]"
)]
fn fill_percent(node: &BudgetTreeNode) -> u32 {
    let Some(target) = node.effective_target.as_ref() else {
        return 0;
    };
    if target.minor_units <= 0 {
        return 0;
    }
    let spent = node.spent.minor_units.max(0) as u64;
    let tgt = target.minor_units as u64;
    let pct = (spent * 100 / tgt).min(100);
    pct as u32
}

/// Formats the amounts column string for a node.
fn display_str(node: &BudgetTreeNode) -> String {
    if node.is_tracking_only {
        format!("{} \u{00b7} tracking", node.spent.format_short())
    } else if let Some(target) = &node.effective_target {
        format!("{} / {}", node.spent.format_short(), target.format_short())
    } else {
        node.spent.format_short()
    }
}

/// One row in the budget allocation grid, representing a single budget line.
///
/// Parent rows (those with children) show an expand/collapse chevron and
/// roll-up figures. Leaf rows show status-coloured progress bars and can open
/// an inline detail panel.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props must be owned values"
)]
#[expect(
    clippy::too_many_lines,
    reason = "complex budget row with parent/leaf branches, badges, and recursive children"
)]
pub fn BudgetRow(
    /// The tree node this row represents.
    node: BudgetTreeNode,
) -> impl IntoView {
    let ctx = use_context::<BudgetPageCtx>();

    let is_parent = !node.children.is_empty();
    let status = row_status(&node);
    let depth = node.depth;
    let pct = fill_percent(&node);

    #[expect(
        clippy::arithmetic_side_effects,
        reason = "depth is bounded (u32 with small values) and 32 + depth * 16 cannot realistically overflow"
    )]
    let pad_left = 32_u32 + depth * 16_u32;
    let indent_style = format!("padding-left: {pad_left}px");
    let fill_style = format!("width: {pct}%; height: 100%");

    let display_name = node
        .name
        .clone()
        .unwrap_or_else(|| node.account_name.clone());
    let amounts = display_str(&node);
    let has_mixed = node.has_mixed_period;
    let native_label = node.native_period_label.clone();
    let node_id = node.id.clone();

    /* Local reactive state. */
    let collapsed = RwSignal::new(false);
    let badge_expanded = RwSignal::new(false);

    /* --- Leaf detail panel toggle --- */
    let detail_open = {
        let nid = node_id.clone();
        Signal::derive(move || ctx.is_some_and(|c| c.open_detail_id.get() == Some(nid.clone())))
    };

    let on_row_click = {
        let nid = node_id.clone();
        move |_ev: leptos::ev::MouseEvent| {
            if let Some(c) = ctx {
                let current = c.open_detail_id.get_untracked();
                if current.as_deref() == Some(&nid) {
                    c.open_detail_id.set(None);
                } else {
                    c.open_detail_id.set(Some(nid.clone()));
                }
            }
        }
    };

    let on_chevron_click = move |ev: leptos::ev::MouseEvent| {
        ev.stop_propagation();
        collapsed.update(|c| *c = !*c);
    };

    let on_badge_click = move |ev: leptos::ev::MouseEvent| {
        ev.stop_propagation();
        badge_expanded.update(|b| *b = !*b);
    };

    /* --- CSS class helpers --- */
    let status_class = move || match status {
        Status::Good => style::status_good,
        Status::Warn => style::status_warn,
        Status::Bad => style::status_bad,
        Status::Dim => style::status_dim,
        Status::Mute => style::status_mute,
    };

    let bar_class = move || match status {
        Status::Good => style::bar_good,
        Status::Warn => style::bar_warn,
        Status::Bad => style::bar_bad,
        Status::Dim | Status::Mute => style::bar_mute,
    };

    let children_nodes = StoredValue::new(node.children.clone());
    let node_for_detail = node.clone();

    if is_parent {
        view! {
            <div>

                <div class=style::row_parent style=indent_style on:click=on_chevron_click>
                    <span class=style::status_parent>
                        {move || if collapsed.get() { "\u{25b6} " } else { "\u{25be} " }}
                        {display_name.clone()}
                        {has_mixed
                            .then(|| {
                                let lbl = native_label.clone();
                                view! {
                                    <span
                                        class=move || {
                                            if badge_expanded.get() {
                                                style::badge_active
                                            } else {
                                                style::badge
                                            }
                                        }
                                        on:click=on_badge_click
                                    >
                                        {lbl}
                                        {move || {
                                            if badge_expanded.get() { " \u{25be}" } else { " \u{25b8}" }
                                        }}
                                    </span>
                                }
                            })}
                    </span>
                    <div class=style::bar_track>
                        <div class=bar_class style=fill_style />
                    </div>
                    <span class=style::amounts>{amounts}</span>
                </div>

                {has_mixed
                    .then(|| {
                        let nid = node_id.clone();
                        view! {
                            <Show when=move || badge_expanded.get()>
                                <NativePeriodList budget_id=nid.clone() depth=depth />
                            </Show>
                        }
                    })}

                <Show when=move || !collapsed.get()>
                    <For
                        each=move || children_nodes.get_value()
                        key=|child| child.id.clone()
                        children=move |child| view! { <BudgetRow node=child /> }
                    />
                </Show>
            </div>
        }
        .into_any()
    } else {
        view! {
            <div>

                <div
                    class=move || {
                        if detail_open.get() { style::row_selected } else { style::row_leaf }
                    }
                    style=indent_style
                    on:click=on_row_click
                >
                    <span class=status_class>
                        {display_name.clone()}
                        {has_mixed
                            .then(|| {
                                let lbl = native_label.clone();
                                view! {
                                    <span
                                        class=move || {
                                            if badge_expanded.get() {
                                                style::badge_active
                                            } else {
                                                style::badge
                                            }
                                        }
                                        on:click=on_badge_click
                                    >
                                        {lbl}
                                        {move || {
                                            if badge_expanded.get() { " \u{25be}" } else { " \u{25b8}" }
                                        }}
                                    </span>
                                }
                            })}
                    </span>
                    <div class=style::bar_track>
                        <div class=bar_class style=fill_style />
                    </div>
                    <span class=style::amounts>{amounts}</span>
                </div>

                {has_mixed
                    .then(|| {
                        let nid = node_id.clone();
                        view! {
                            <Show when=move || badge_expanded.get()>
                                <NativePeriodList budget_id=nid.clone() depth=depth />
                            </Show>
                        }
                    })}

                <Show when=move || detail_open.get()>
                    <BudgetDetail node=node_for_detail.clone() />
                </Show>
            </div>
        }
        .into_any()
    }
}
