//! Sticky period-control bar that remains visible while scrolling.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::period_nav;

import_style!(style, "sticky_bar.module.scss");

/// Formats an optional [`Amount`] for compact display, returning `"–"` when `None`.
fn format_amount(amount: Option<&Amount>) -> String {
    amount.map_or_else(|| "\u{2013}".into(), Amount::format_short)
}

/// Sticky single-row summary bar that stays below the app top bar once the
/// expanded header scrolls off-screen.
///
/// Shows ◀ / ▶ period navigation, the current period label, and compact KPI
/// values inline. The bar is `position: sticky` via CSS — it is always
/// rendered; CSS controls visibility.
#[component]
pub fn StickyBar(
    /// Budget overview resource supplying the summary and tree.
    overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>>,
) -> impl IntoView {
    let ctx = expect_context::<BudgetPageCtx>();
    let period = ctx.display_period;
    let window_start = ctx.window_start;

    let period_label = move || period_nav::window_label(&period.get(), window_start.get());

    view! {
        <div class=style::sticky_bar>
            <button
                class=style::nav_btn
                on:click=move |_| {
                    window_start
                        .update(|ws| *ws = period_nav::step_window(&period.get(), *ws, false));
                }
            >
                "\u{25C0}"
            </button>
            <span class=style::label>{period_label}</span>
            <button
                class=style::nav_btn
                on:click=move |_| {
                    window_start
                        .update(|ws| *ws = period_nav::step_window(&period.get(), *ws, true));
                }
            >
                "\u{25B6}"
            </button>

            <div class=style::kpi_compact>
                <Suspense fallback=move || {
                    view! { <span class=style::kpi_item>"\u{2013}"</span> }
                }>
                    {move || {
                        overview
                            .get()
                            .map(|result| {
                                let summary = result.ok().map(|(s, _)| s);
                                let kpi = match summary.as_ref() {
                                    None => "\u{2013}".into(),
                                    Some(s) if s.has_mixed_commodities => "mixed currencies".into(),
                                    Some(s) => {
                                        let b = format_amount(s.total_budgeted.as_ref());
                                        let sp = format_amount(s.total_spent.as_ref());
                                        let r = format_amount(s.total_remaining.as_ref());
                                        let n = s.overspent_count;
                                        format!("B {b} | S {sp} | R {r} | {n} over")
                                    }
                                };
                                view! { <span class=style::kpi_item>{kpi}</span> }
                            })
                    }}
                </Suspense>
            </div>
        </div>
    }
}
