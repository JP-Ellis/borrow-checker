//! Single row in the budget allocation tree.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::components::budget_detail::BudgetDetail;
use crate::pages::budget::components::native_period_list::NativePeriodList;
use crate::pages::budget::period_nav;

import_style!(style, "row.module.scss");

/// Row status derived from spend vs. target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    /// Spend is ≤ 85% of target.
    Good,
    /// Spend is > 85% but ≤ 105% of target (allowing minor overage).
    Warn,
    /// Spend exceeds 105% of target.
    Bad,
    /// Budget has a target but nothing has been spent yet.
    Dim,
    /// No target, or tracking-only mode.
    Mute,
}

/// Derives a [`Status`] from the node's spend and target figures.
///
/// Uses integer arithmetic to avoid floating-point.
/// Good: spent/target ≤ 0.85, Warn: > 0.85 and ≤ 1.05, Bad: > 1.05.
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
        Some(target) => {
            let spent = node.spent.minor_units;
            let tgt = target.minor_units;
            /* Integer thresholds: > 105% → Bad, > 85% → Warn, else Good. */
            if spent * 100 > tgt * 105 {
                Status::Bad
            } else if spent * 100 > tgt * 85 {
                Status::Warn
            } else {
                Status::Good
            }
        }
    }
}

/// Raw fill percentage (0–125), computed with integer arithmetic.
///
/// Returns 0 when there is no target or when target minor-units are zero.
/// Capped at 125 so that an overshoot of > 25 % collapses to the same
/// maximum bar width and is distinguished only by the status colour.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    clippy::as_conversions,
    clippy::cast_sign_loss,
    reason = "budget minor-unit arithmetic is bounded; .max(0) guarantees non-negative before cast; integer division for percentage is intentional; clamped to [0,125]"
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
    let pct = (spent * 125 / tgt).min(125);
    pct as u32
}

/// Maps a raw fill percentage (0–125) to the bar's visual position (0–100).
///
/// The bar track represents 0–125 % of the budget target:
/// 100 % budget spend = 80 % of the visual bar width.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "pct is 0–125; multiplication by 4 fits u32; division by 5 is intentional"
)]
fn bar_display_pct(pct: u32) -> u32 {
    pct * 4 / 5
}

/// Returns the CSS `left` style for the prorated-time marker, or `None` when
/// the display window does not include today (period is past or future).
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    clippy::as_conversions,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation,
    reason = "elapsed and total are bounded day counts; today >= start ensures non-negative elapsed; .min(100) constrains to [0,100] which fits u32"
)]
fn prorated_marker_style(ctx: Option<BudgetPageCtx>) -> Option<String> {
    let c = ctx?;
    let today = jiff::Zoned::now().date();
    let start = c.window_start.get_untracked();
    let period = c.display_period.get_untracked();
    let end = period_nav::period_end(&period, start);
    if today < start || today >= end {
        return None;
    }
    let total = i64::from((end - start).get_days());
    if total <= 0 {
        return None;
    }
    let elapsed = i64::from((today - start).get_days());
    let time_pct = (elapsed * 100 / total).min(100) as u32;
    let display = bar_display_pct(time_pct);
    Some(format!("left: {display}%"))
}

/// Formats the amounts column string for a node.
///
/// In `pct_mode`, returns `"N%"` (integer, spent ÷ target × 100).
/// Falls back to absolute amounts when tracking-only or when target is zero.
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "pct calculation: minor-unit values are bounded; integer division is intentional for percentage display"
)]
fn display_str(node: &BudgetTreeNode, pct_mode: bool) -> String {
    if node.is_tracking_only {
        return format!("{} \u{00b7} tracking", node.spent.format_short());
    }
    match &node.effective_target {
        None => node.spent.format_short(),
        Some(target) if pct_mode => {
            if target.minor_units == 0 {
                "\u{2013}".into()
            } else {
                format!(
                    "{}%",
                    (node.spent.minor_units.max(0) * 100) / target.minor_units
                )
            }
        }
        Some(target) => format!("{} / {}", node.spent.format_short(), target.format_short()),
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

    let indent_style = format!("--row-depth:{depth}");
    let fill_display = bar_display_pct(pct);
    let fill_style = format!("width: {fill_display}%; height: 100%");
    let prorated_style = prorated_marker_style(ctx);

    let display_name = node
        .name
        .clone()
        .unwrap_or_else(|| node.account_name.clone());
    let node_sv = StoredValue::new(node.clone());
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
                        <div class=bar_class style=fill_style.clone() />
                        <div class=style::bar_target_mark />
                        {prorated_style
                            .clone()
                            .map(|s| view! { <div class=style::bar_prorated_mark style=s /> })}
                    </div>
                    <span class=style::amounts>
                        {move || display_str(
                            &node_sv.get_value(),
                            ctx.is_some_and(|c| c.pct_mode.get()),
                        )}
                    </span>
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
                        key=|child| format!("{}:{}", child.account_id, child.depth)
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
                        <div class=bar_class style=fill_style.clone() />
                        <div class=style::bar_target_mark />
                        {prorated_style
                            .clone()
                            .map(|s| view! { <div class=style::bar_prorated_mark style=s /> })}
                    </div>
                    <span class=style::amounts>
                        {move || display_str(
                            &node_sv.get_value(),
                            ctx.is_some_and(|c| c.pct_mode.get()),
                        )}
                    </span>
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
