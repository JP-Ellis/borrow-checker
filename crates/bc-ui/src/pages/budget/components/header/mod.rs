//! Budget page header — period navigation, granularity select, and KPI summary tiles.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use bc_ipc::Period;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::period_nav;

import_style!(style, "header.module.scss");

/// Parses a period granularity from the select element value attribute.
fn parse_period(val: &str) -> Period {
    match val {
        "weekly" => Period::Weekly,
        "fortnightly" => Period::Fortnightly,
        "quarterly" => Period::Quarterly,
        "financial_quarter" => Period::FinancialQuarter {
            start_month: 7,
            start_day: 1,
        },
        "financial_year" => Period::FinancialYear {
            start_month: 7,
            start_day: 1,
        },
        "calendar_year" => Period::CalendarYear,
        _ => Period::Monthly,
    }
}

/// Converts a [`Period`] back to the `<select>` option value string.
fn period_to_str(p: &Period) -> &'static str {
    match p {
        Period::Weekly => "weekly",
        Period::Fortnightly => "fortnightly",
        Period::Quarterly => "quarterly",
        Period::FinancialQuarter { .. } => "financial_quarter",
        Period::FinancialYear { .. } => "financial_year",
        Period::CalendarYear => "calendar_year",
        Period::Monthly | Period::Daily | _ => "monthly",
    }
}

/// Formats an optional [`Amount`] for display, returning `"–"` when `None`.
fn format_amount(amount: Option<&Amount>) -> String {
    amount.map_or_else(|| "\u{2013}".into(), Amount::format_short)
}

/// A single KPI tile showing a label and a value.
#[component]
fn KpiTile(
    /// Short uppercase label describing the metric.
    #[prop(into)]
    label: &'static str,
    /// Formatted value string to display in large monospace text.
    value: String,
    /// CSS class for the value span. Defaults to `style::kpi_value`.
    #[prop(optional)]
    value_class: Option<&'static str>,
) -> impl IntoView {
    let vclass = value_class.unwrap_or(style::kpi_value);
    view! {
        <div class=style::kpi_tile>
            <span class=style::kpi_label>{label}</span>
            <span class=vclass>{value}</span>
        </div>
    }
}

/// The four KPI tiles rendered from a loaded [`BudgetSummary`].
#[component]
fn KpiTileRow(
    /// The budget summary containing aggregated totals.
    summary: Option<BudgetSummary>,
) -> impl IntoView {
    let budgeted = format_amount(summary.as_ref().and_then(|s| s.total_budgeted.as_ref()));
    let spent = format_amount(summary.as_ref().and_then(|s| s.total_spent.as_ref()));
    let remaining = format_amount(summary.as_ref().and_then(|s| s.total_remaining.as_ref()));
    let (net, net_class) = match summary.as_ref().and_then(|s| s.total_remaining.as_ref()) {
        None => ("\u{2013}".to_owned(), style::kpi_value),
        Some(a) if a.value < rust_decimal::Decimal::ZERO => {
            (a.format_short(), style::kpi_value_bad)
        }
        Some(a) => (a.format_short(), style::kpi_value_good),
    };

    view! {
        <div class=style::kpi_row>
            <KpiTile label="Budgeted" value=budgeted />
            <KpiTile label="Spent" value=spent />
            <KpiTile label="Remaining" value=remaining />
            <KpiTile label="Net" value=net value_class=net_class />
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
    overview: LocalResource<Result<(BudgetSummary, Vec<BudgetTreeNode>), BcError>>,
) -> impl IntoView {
    let ctx = expect_context::<BudgetPageCtx>();
    let period = ctx.display_period;
    let window_start = ctx.window_start;
    let pct_mode = ctx.pct_mode;

    let period_label = move || period_nav::window_label(&period.get(), window_start.get());

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
                <button
                    class=style::nav_btn
                    on:click=move |_| {
                        window_start
                            .update(|ws| *ws = period_nav::step_window(&period.get(), *ws, false));
                    }
                >
                    "\u{25C0}"
                </button>
                <span class=style::nav_label>{period_label}</span>
                <button
                    class=style::nav_btn
                    on:click=move |_| {
                        window_start
                            .update(|ws| *ws = period_nav::step_window(&period.get(), *ws, true));
                    }
                >
                    "\u{25B6}"
                </button>

                <select
                    class=style::period_select
                    prop:value=move || period_to_str(&period.get())
                    on:change=move |ev| {
                        let val = event_target_value(&ev);
                        period.set(parse_period(&val));
                    }
                >
                    <option value="weekly">"Weekly"</option>
                    <option value="fortnightly">"Fortnightly"</option>
                    <option value="monthly">"Monthly"</option>
                    <option value="quarterly">"Quarterly"</option>
                    <option value="financial_quarter">"Financial Quarter"</option>
                    <option value="financial_year">"Financial Year"</option>
                    <option value="calendar_year">"Calendar Year"</option>
                </select>

                <button
                    class=agg_class
                    on:click=move |_| {
                        pct_mode.update(|m| *m = !*m);
                    }
                >
                    {agg_label}
                </button>
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
                            let summary = result.ok().map(|(s, _)| s);
                            view! { <KpiTileRow summary=summary /> }
                        })
                }}
            </Suspense>
        </div>
    }
}
