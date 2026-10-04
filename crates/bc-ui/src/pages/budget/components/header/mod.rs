//! Budget page header — period navigation, granularity select, and verdict summary.

#[cfg(debug_assertions)]
pub(crate) mod qa;
/// Leptos-free verdict-line formatting — native-testable.
pub(crate) mod text;

use bc_ipc::BcError;
use bc_ipc::BudgetOverview;
use bc_ipc::BudgetSummary;
use leptos::prelude::*;
use stylance::import_style;

use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;
use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::money;

import_style!(style, "header.module.scss");

/// CSS colour for a verdict segment, or `None` for the default ink colour.
#[must_use]
fn kind_color(kind: text::VerdictKind) -> Option<&'static str> {
    match kind {
        text::VerdictKind::Red => Some("var(--bc-bad)"),
        text::VerdictKind::Warn => Some("var(--bc-warn)"),
        text::VerdictKind::Ok => None,
    }
}

/// Verdict line with red and warn counts colour-coded; the OK count and
/// separators stay the default ink colour.
///
/// Renders [`text::verdict_parts`] — the same ordering, words and zero
/// filtering as the tested [`text::verdict_line`] string.
#[component]
fn VerdictLine(
    /// Rows with a red verdict.
    red: u32,
    /// Rows with a warn verdict.
    warn: u32,
    /// Rows with a green verdict.
    green: u32,
) -> impl IntoView {
    let segments = text::verdict_parts(red, warn, green);
    let mut parts: Vec<AnyView> = Vec::new();
    for (n, word, kind) in segments {
        if !parts.is_empty() {
            parts.push(view! { <span>" \u{00b7} "</span> }.into_any());
        }
        let text = format!("{n} {word}");
        parts.push(match kind_color(kind) {
            Some(c) => view! { <span style=format!("color: {c}")>{text}</span> }.into_any(),
            None => view! { <span>{text}</span> }.into_any(),
        });
    }
    if parts.is_empty() {
        parts.push(view! { <span>"no verdicts"</span> }.into_any());
    }
    view! { <span class=style::verdict_line>{parts}</span> }
}

/// Verdict line and per-root unbudgeted totals rendered from a loaded
/// [`BudgetSummary`].
#[component]
fn SummaryRow(
    /// The budget summary for the display window.
    summary: Option<BudgetSummary>,
) -> impl IntoView {
    let currencies = crate::currency_ctx::use_currency_store();
    let has_unvalued = summary.as_ref().is_some_and(|s| s.has_unvalued);
    let (red, warn, green) = summary
        .as_ref()
        .map_or((0, 0, 0), |s| (s.red, s.warn, s.green));
    let unbudgeted = summary.map_or_else(Vec::new, |s| s.unbudgeted);

    view! {
        <div class=style::summary_row>
            <VerdictLine red=red warn=warn green=green />
            <div class=style::unbudgeted_list>
                {unbudgeted
                    .into_iter()
                    .map(|(root, amount)| {
                        let label = format!(
                            "{} unbudgeted {}",
                            money::fmt(&amount, &currencies.get()),
                            root.to_lowercase(),
                        );
                        view! { <span class=style::unbudgeted_item>{label}</span> }
                    })
                    .collect_view()}
            </div>
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

/// Header strip showing period navigation controls and the verdict summary.
///
/// Reads [`BudgetPageCtx`] from context for reactive period and mode state.
/// The `overview` resource drives [`SummaryRow`] via [`Suspense`].
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
    let inert_hint = Signal::derive(move || {
        filter_store
            .filter
            .with(crate::pages::budget::query::inert_hint)
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
                <Show when=move || inert_hint.with(Option::is_some)>
                    <span class=style::date_hint>
                        {move || inert_hint.get().unwrap_or_default()}
                    </span>
                </Show>
            </div>

            <Suspense fallback=move || {
                view! { <div class=style::summary_row>"\u{2013}"</div> }
            }>
                {move || {
                    overview
                        .get()
                        .map(|result| {
                            let summary = result.ok().map(|o| o.summary);
                            view! { <SummaryRow summary=summary /> }
                        })
                }}
            </Suspense>
        </div>
    }
}
