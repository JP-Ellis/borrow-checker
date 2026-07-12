//! Budget page header — period navigation, granularity select, and KPI summary tiles.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::Amount;
use bc_ipc::BcError;
use bc_ipc::BudgetSummary;
use bc_ipc::BudgetTreeNode;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::budget::BudgetPageCtx;

import_style!(style, "header.module.scss");

/// Formats an optional [`Amount`] for display, returning `"–"` when `None`.
///
/// The symbol is resolved from `currencies` (the served commodity set) using the
/// amount's own `currency_code`.
fn format_amount(amount: Option<&Amount>, currencies: &[bc_ipc::CommodityInfo]) -> String {
    amount.map_or_else(
        || "\u{2013}".into(),
        |a| {
            let (sym, after) = crate::currency_ctx::short_symbol(&a.currency_code, currencies);
            a.format_short(sym.as_deref(), after)
        },
    )
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
    let currencies = crate::currency_ctx::use_currency_store();

    move || {
        let currencies = currencies.get();
        let budgeted = format_amount(
            summary.as_ref().and_then(|s| s.total_budgeted.as_ref()),
            &currencies,
        );
        let spent = format_amount(
            summary.as_ref().and_then(|s| s.total_spent.as_ref()),
            &currencies,
        );
        let remaining = format_amount(
            summary.as_ref().and_then(|s| s.total_remaining.as_ref()),
            &currencies,
        );
        let (net, net_class) = match summary.as_ref().and_then(|s| s.total_remaining.as_ref()) {
            None => ("\u{2013}".to_owned(), style::kpi_value),
            Some(a) if a.value < rust_decimal::Decimal::ZERO => {
                let (sym, after) = crate::currency_ctx::short_symbol(&a.currency_code, &currencies);
                (a.format_short(sym.as_deref(), after), style::kpi_value_bad)
            }
            Some(a) => {
                let (sym, after) = crate::currency_ctx::short_symbol(&a.currency_code, &currencies);
                (a.format_short(sym.as_deref(), after), style::kpi_value_good)
            }
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

    let filter_store = crate::filter_ctx::use_filter_store();
    let date_hint_visible = Signal::derive(move || {
        filter_store
            .filter
            .with(crate::pages::budget::query::date_filter_active)
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
                <Show when=move || date_hint_visible.get()>
                    <span class=style::date_hint>
                        "Date filter doesn\u{2019}t apply to budgets \u{2014} using the selected period."
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
                            let summary = result.ok().map(|(s, _)| s);
                            view! { <KpiTileRow summary=summary /> }
                        })
                }}
            </Suspense>
        </div>
    }
}
