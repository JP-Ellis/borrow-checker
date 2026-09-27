//! Sticky period-control bar that remains visible while scrolling.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BcError;
use bc_ipc::BudgetOverview;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::period_nav::PeriodNav;
use crate::pages::budget::BudgetPageCtx;

import_style!(style, "sticky_bar.module.scss");

/// Sticky single-row summary bar that stays below the app top bar once the
/// expanded header scrolls off-screen.
///
/// Shows ◀ / ▶ period navigation, the current period label, and compact KPI
/// values inline. The bar is `position: sticky` via CSS — it is always
/// rendered; CSS controls visibility.
#[component]
pub fn StickyBar(
    /// Budget overview resource supplying the summary and tree.
    overview: LocalResource<Result<BudgetOverview, BcError>>,
) -> impl IntoView {
    let ctx = expect_context::<BudgetPageCtx>();
    let period = ctx.display_period;
    let window_start = ctx.window_start;

    view! {
        <div class=style::sticky_bar>
            <PeriodNav period=period window_start=window_start compact=true />

            <div class=style::kpi_compact>
                <Suspense fallback=move || {
                    view! { <span class=style::kpi_item>"\u{2013}"</span> }
                }>
                    {move || {
                        overview
                            .get()
                            .map(|result| {
                                let kpi = match result.ok() {
                                    None => "\u{2013}".into(),
                                    Some(o) => {
                                        let n = o.summary.red;
                                        format!("B \u{2013} | S \u{2013} | R \u{2013} | {n} over")
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
