//! Budget page header — period navigation, granularity select, and KPI summary tiles.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BcError;
use bc_ipc::BudgetOverview;
use bc_ipc::BudgetSummary;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;
use crate::pages::budget::BudgetPageCtx;

import_style!(style, "header.module.scss");

/// A single KPI tile showing a label and a value.
#[component]
fn KpiTile(
    /// Short uppercase label describing the metric.
    #[prop(into)]
    label: &'static str,
    /// Formatted value string to display in large monospace text.
    value: String,
) -> impl IntoView {
    view! {
        <div class=style::kpi_tile>
            <span class=style::kpi_label>{label}</span>
            <span class=style::kpi_value>{value}</span>
        </div>
    }
}

/// The four KPI tiles rendered from a loaded [`BudgetSummary`].
///
/// The summary carries verdict counts, not totals, so the amount tiles show
/// `–`.
#[component]
fn KpiTileRow(
    /// The budget summary for the display window.
    summary: Option<BudgetSummary>,
) -> impl IntoView {
    let has_unvalued = summary.as_ref().is_some_and(|s| s.has_unvalued);

    view! {
        <div class=style::kpi_row>
            <KpiTile label="Budgeted" value="\u{2013}".into() />
            <KpiTile label="Spent" value="\u{2013}".into() />
            <KpiTile label="Remaining" value="\u{2013}".into() />
            <KpiTile label="Net" value="\u{2013}".into() />
            {has_unvalued
                .then(|| {
                    view! {
                        <span class=style::unvalued>
                            <StatusPill label="unvalued".to_owned() tone=Tone::Warn />
                        </span>
                    }
                })}
        </div>
    }
}

/// Header strip showing period navigation controls and top-level budget KPI tiles.
///
/// Reads [`BudgetPageCtx`] from context for reactive period and mode state.
/// The `overview` resource drives the KPI tile row via [`Suspense`].
#[component]
pub fn BudgetHeader(
    /// Budget overview resource supplying the summary and tree.
    overview: LocalResource<Result<BudgetOverview, BcError>>,
) -> impl IntoView {
    let ctx = expect_context::<BudgetPageCtx>();
    let period = ctx.display_period;
    let window_start = ctx.window_start;
    let pct_mode = ctx.pct_mode;

    let filter_store = crate::filter_ctx::use_filter_store();
    let inert_hint_visible = Signal::derive(move || {
        filter_store
            .filter
            .with(crate::pages::budget::query::inert_filter_active)
    });

    let agg_label = move || {
        if pct_mode.get() {
            "% target"
        } else {
            "$ value"
        }
    };

    let agg_class = move || {
        if pct_mode.get() {
            style::agg_btn_active
        } else {
            style::agg_btn
        }
    };

    view! {
        <div class=style::header>
            <div class=style::nav_row>
                <crate::components::period_nav::PeriodNav period=period window_start=window_start />
                <button
                    class=agg_class
                    on:click=move |_| {
                        pct_mode.update(|m| *m = !*m);
                    }
                >
                    {agg_label}
                </button>
                <Show when=move || inert_hint_visible.get()>
                    <span class=style::date_hint>
                        "Date and balance filters don\u{2019}t apply to budgets \u{2014} using the selected period."
                    </span>
                </Show>
            </div>

            <Suspense fallback=move || {
                view! {
                    <div class=style::kpi_row>
                        <KpiTile label="Budgeted" value="\u{2013}".into() />
                        <KpiTile label="Spent" value="\u{2013}".into() />
                        <KpiTile label="Remaining" value="\u{2013}".into() />
                        <KpiTile label="Net" value="\u{2013}".into() />
                    </div>
                }
            }>
                {move || {
                    overview
                        .get()
                        .map(|result| {
                            let summary = result.ok().map(|o| o.summary);
                            view! { <KpiTileRow summary=summary /> }
                        })
                }}
            </Suspense>
        </div>
    }
}
