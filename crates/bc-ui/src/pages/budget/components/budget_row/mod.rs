//! Single row in the budget allocation tree.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::Amount;
use bc_ipc::BudgetTreeNode;
use bc_ipc::RowKind;
use bc_ipc::Verdict;
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use stylance::import_style;

use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;
use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::bar;
use crate::pages::budget::components::budget_detail::BudgetDetail;
use crate::pages::budget::components::native_period_list::NativePeriodList;
use crate::pages::budget::money;
use crate::pages::budget::unvalued::unvalued_label;

import_style!(style, "row.module.scss");

/// CSS colour value for a verdict, used as the bar segments' `--seg` variable.
#[must_use]
fn verdict_color(verdict: Option<Verdict>) -> &'static str {
    match verdict {
        Some(Verdict::Good) => "var(--bc-good)",
        Some(Verdict::Warn) => "var(--bc-warn)",
        Some(Verdict::Bad) => "var(--bc-bad)",
        _ => "var(--bc-ink-mute)",
    }
}

/// CSS class colouring the Actual figure by verdict.
#[must_use]
fn verdict_class(verdict: Option<Verdict>) -> &'static str {
    match verdict {
        Some(Verdict::Good) => style::status_good,
        Some(Verdict::Warn) => style::status_warn,
        Some(Verdict::Bad) => style::status_bad,
        _ => style::status_mute,
    }
}

/// Tooltip naming what a leftover row represents, or `None` for a non-leftover row.
fn leftover_title(kind: RowKind, parent_label: &str) -> Option<String> {
    if kind == RowKind::Unallocated {
        return Some(format!(
            "{parent_label}'s budget not claimed by a sub-budget"
        ));
    }
    if kind == RowKind::Unbudgeted {
        return Some(format!("Spend under {parent_label} with no budget"));
    }
    None
}

/// One row in the budget allocation grid: ACCOUNT | PROGRESS | ACTUAL | TARGET.
///
/// Type roots render upper-cased; leftover rows (`Unallocated`, `Unbudgeted`)
/// render italic and muted. A chevron appears only when the row has children,
/// and clicking it toggles collapse without opening the detail panel.
/// Clicking the row body opens the detail panel for every kind except
/// `Account`, which only toggles collapse.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props must be owned values"
)]
#[expect(
    clippy::too_many_lines,
    reason = "one row assembles the label, stacked bar, actual/target cells, badges and pills"
)]
pub fn BudgetRow(
    /// The tree node this row represents.
    node: BudgetTreeNode,
    /// Nesting depth, 0 for a type root.
    depth: u32,
    /// The immediate parent's label, used for a leftover row's tooltip.
    /// `None` for a type root, which has no parent.
    parent_label: Option<String>,
    /// The immediate parent's target, used by an `Unbudgeted` row's bar (it
    /// has no target of its own). `None` for a type root.
    parent_target: Option<Amount>,
) -> impl IntoView {
    let ctx = use_context::<BudgetPageCtx>();
    let currencies = crate::currency_ctx::use_currency_store();

    let is_root = depth == 0;
    let is_leftover = matches!(node.kind, RowKind::Unallocated | RowKind::Unbudgeted);
    let has_children = !node.children.is_empty();
    let is_tag_only = !is_leftover && node.tag_filter.as_deref() == Some(node.label.as_str());

    let node_id = node.id.clone();
    let node_kind = node.kind;
    let node_label = node.label.clone();
    let node_verdict = node.verdict;

    let collapsed = RwSignal::new(false);
    let badge_expanded = RwSignal::new(false);

    let indent_style = format!("--row-depth:{depth}");
    let row_title = leftover_title(node_kind, parent_label.as_deref().unwrap_or_default());

    let detail_open = {
        let nid = node_id.clone();
        Signal::derive(move || ctx.is_some_and(|c| c.open_detail_id.get() == Some(nid.clone())))
    };

    let on_row_click = {
        let nid = node_id.clone();
        move |_ev: leptos::ev::MouseEvent| {
            if node_kind == RowKind::Account {
                if has_children {
                    collapsed.update(|c| *c = !*c);
                }
                return;
            }
            let Some(c) = ctx else { return };
            let current = c.open_detail_id.get_untracked();
            if current.as_deref() == Some(nid.as_str()) {
                c.open_detail_id.set(None);
            } else {
                c.open_detail_id.set(Some(nid.clone()));
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

    let row_classes_base = {
        let mut classes = vec![style::row];
        if is_root {
            classes.push(style::row_root);
        }
        if is_leftover {
            classes.push(style::row_leftover);
        }
        classes.join(" ")
    };
    let row_classes = move || {
        if detail_open.get() {
            format!("{row_classes_base} {}", style::row_open)
        } else {
            row_classes_base.clone()
        }
    };

    /* --- account cell --- */

    let chevron_view = has_children.then(|| {
        view! {
            <span class=style::chevron on:click=on_chevron_click>
                {move || if collapsed.get() { "\u{25b6}" } else { "\u{25be}" }}
            </span>
        }
    });

    let worst_descendant = node.worst_descendant;
    let dot_view = move || {
        (collapsed.get() && worst_descendant.is_some()).then(|| {
            view! { <span class=style::dot style=format!("background:{}", verdict_color(worst_descendant)) /> }
        })
    };

    let tag_chip = is_tag_only.then(|| format!("#{}", node.tag_filter.clone().unwrap_or_default()));
    let label_class = if is_tag_only {
        style::tag
    } else {
        style::label
    };
    let label_text = tag_chip.unwrap_or_else(|| {
        if is_root {
            node_label.to_uppercase()
        } else {
            node_label.clone()
        }
    });

    let has_mixed = node.has_mixed_period;
    let native_label = node.native_period_label.clone();
    let mixed_badge = has_mixed.then(|| {
        let lbl = native_label.clone();
        view! {
            <span
                class=move || if badge_expanded.get() { style::badge_active } else { style::badge }
                on:click=on_badge_click
            >
                {lbl}
                {move || if badge_expanded.get() { " \u{25be}" } else { " \u{25b8}" }}
            </span>
        }
    });
    let native_period_block = has_mixed.then(|| {
        let nid = node_id.clone();
        view! {
            <Show when=move || badge_expanded.get()>
                <NativePeriodList budget_id=nid.clone() depth=depth />
            </Show>
        }
    });

    /* --- progress bar --- */

    let bar_target: Option<Decimal> = if node_kind == RowKind::Unbudgeted {
        parent_target.as_ref().map(|a| a.value)
    } else {
        node.target.as_ref().map(|a| a.value)
    };
    let segs = bar::segments(bar_target, node.claimed, node.unallocated, node.unbudgeted);
    let seg_color = verdict_color(node_verdict);
    let claimed_style = format!("--seg:{seg_color};left:0%;width:{}%", segs.claimed);
    let unalloc_style = format!(
        "--seg:{seg_color};left:{}%;width:{}%",
        segs.claimed, segs.unallocated
    );
    let unbud_left = segs.claimed.saturating_add(segs.unallocated);
    let unbud_style = format!("left:{unbud_left}%;width:{}%", segs.unbudgeted);
    let fade_view = segs.overflows.then(|| view! { <div class=style::fade /> });
    let pace_view = move || {
        ctx.and_then(|c| c.elapsed_fraction.get())
            .filter(|e| *e < Decimal::ONE)
            .map(|e| {
                let left = bar::tick_percent(e);
                view! { <div class=style::pace_tick style=format!("left:{left}%") /> }
            })
    };

    /* --- actual / target cells --- */

    let node_ratio = node.ratio;
    let node_actual = node.actual.clone();
    let actual_class = format!("{} {}", style::amount, verdict_class(node_verdict));
    let actual_view = move || {
        if ctx.is_some_and(|c| c.pct_mode.get()) {
            node_ratio.map_or_else(
                || "\u{2013}".to_owned(),
                |r| {
                    #[expect(
                        clippy::arithmetic_side_effects,
                        reason = "ratio is a bounded Decimal; the product cannot overflow"
                    )]
                    let pct = (r * Decimal::from(100_u32)).round().to_i64().unwrap_or(0);
                    format!("{pct}%")
                },
            )
        } else {
            node_actual
                .as_ref()
                .map_or_else(|| "mixed".to_owned(), |a| money::fmt(a, &currencies.get()))
        }
    };

    let node_target = node.target.clone();
    let node_target_for_fx = node.target.clone();
    let node_target_expr = node.target_expr.clone();
    let node_intent = node.intent;
    let node_mixed = node.mixed;
    let target_text = move || -> String {
        if node_kind == RowKind::Unbudgeted {
            return String::new();
        }
        if node_mixed && node_target.is_none() {
            return "mixed".to_owned();
        }
        node_target.as_ref().map_or_else(String::new, |t| {
            format!(
                "{}{}",
                bar::intent_glyph(node_intent),
                money::fmt(t, &currencies.get())
            )
        })
    };
    let target_fx = move || {
        if node_kind == RowKind::Unbudgeted || (node_mixed && node_target_for_fx.is_none()) {
            return None;
        }
        node_target_expr.clone().map(|expr| {
            view! {
                <span class=style::target_fn title=expr>
                    "\u{192}"
                </span>
            }
        })
    };

    /* --- pills --- */

    let unvalued_vec = node.unvalued.clone();
    let node_double_counted = node.double_counted;
    let node_over_allocated = node.over_allocated;
    let node_sign_flip = node.sign_flip;
    let unvalued_pill = unvalued_label(&unvalued_vec);
    let has_pills =
        unvalued_pill.is_some() || node_double_counted || node_over_allocated || node_sign_flip;
    let pills_view = has_pills.then(|| {
        view! {
            <div class=style::pills>
                {unvalued_pill.map(|l| view! { <StatusPill label=l tone=Tone::Warn /> })}
                {node_double_counted
                    .then(|| {
                        view! { <StatusPill label="double-counted".to_owned() tone=Tone::Warn /> }
                    })}
                {node_over_allocated
                    .then(|| {
                        view! { <StatusPill label="over-allocated".to_owned() tone=Tone::Warn /> }
                    })}
                {node_sign_flip
                    .then(|| view! { <StatusPill label="sign flip".to_owned() tone=Tone::Warn /> })}
            </div>
        }
    });

    /* --- children --- */

    let node_for_detail = node.clone();
    let parent_label_for_children = node_label.clone();
    let parent_target_for_children = node.target.clone();
    let children_nodes = node.children.clone();

    view! {
        <div>
            <div class=row_classes style=indent_style title=row_title.clone() on:click=on_row_click>
                <span class=style::col_account>
                    {chevron_view} {dot_view} <span class=style::label_clip>
                        <span class=label_class>{label_text}</span>
                    </span> {mixed_badge}
                </span>
                <div class=style::bar_track>
                    <div class=style::seg_claimed style=claimed_style />
                    <div class=style::seg_unallocated style=unalloc_style />
                    <div class=style::seg_unbudgeted style=unbud_style />
                    <div class=style::bar_target_mark />
                    {pace_view}
                    {fade_view}
                </div>
                <span class=actual_class>{actual_view}</span>
                <span class=style::amount>{target_text} {target_fx}</span>
                {pills_view}
            </div>

            {native_period_block}

            <Show when=move || detail_open.get()>
                <BudgetDetail
                    node=node_for_detail.clone()
                    leftover_title=row_title.clone().unwrap_or_default()
                />
            </Show>

            <Show when=move || {
                !collapsed.get()
            }>
                {
                    let children_for_for = children_nodes.clone();
                    let parent_label_for_for = parent_label_for_children.clone();
                    let parent_target_for_for = parent_target_for_children.clone();
                    view! {
                        <For
                            each=move || children_for_for.clone()
                            key=|child| child.id.clone()
                            children=move |child| {
                                view! {
                                    <BudgetRow
                                        node=child
                                        depth=depth.saturating_add(1)
                                        parent_label=Some(parent_label_for_for.clone())
                                        parent_target=parent_target_for_for.clone()
                                    />
                                }
                                    .into_any()
                            }
                        />
                    }
                }
            </Show>
        </div>
    }
}
