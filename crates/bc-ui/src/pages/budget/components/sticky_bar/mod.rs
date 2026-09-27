//! Sticky period-control bar that remains visible while scrolling.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BcError;
use bc_ipc::BudgetOverview;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::period_nav::PeriodNav;
use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::components::header::text::verdict_line;

import_style!(style, "sticky_bar.module.scss");

/// Sticky single-row summary bar that stays below the app top bar once the
/// expanded header scrolls off-screen.
///
/// Shows ◀ / ▶ period navigation, the current period label, and the verdict
/// line inline. The bar is `position: sticky` via CSS — it is always
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
                                    None => "\u{2013}".to_owned(),
                                    Some(o) => {
                                        verdict_line(o.summary.red, o.summary.warn, o.summary.green)
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
