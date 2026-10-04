//! Expandable list of native sub-periods for a mixed-period budget row.

/// Leptos-free percentage label, native-tested through `components_tests`.
mod pct;
#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BcError;
use bc_ipc::NativePeriodRow;
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use stylance::import_style;

use self::pct::pct_label;
use crate::components::period_nav;
use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;
use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::unvalued::unvalued_label;
use crate::pages::budget::verdict::VerdictTone;
use crate::pages::budget::verdict::verdict_tone;

import_style!(pub(crate) style, "native.module.scss");

/// The row's spend, zero when it has none.
fn spent_value(row: &NativePeriodRow) -> Decimal {
    row.spent.as_ref().map_or(Decimal::ZERO, |s| s.value)
}

/// Progress bar fill percentage (0–100), computed with integer arithmetic.
///
/// Returns 0 when there is no target or when target minor-units are zero.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "budget Decimal arithmetic is bounded; .max(ZERO) guarantees non-negative; clamped to [0,100]"
)]
fn fill_percent(row: &NativePeriodRow) -> u32 {
    let Some(target) = row.effective_target.as_ref() else {
        return 0;
    };
    if target.value <= Decimal::ZERO {
        return 0;
    }
    let spent = spent_value(row).max(Decimal::ZERO);
    let pct = (spent * Decimal::from(100_i64) / target.value).min(Decimal::from(100_i64));
    pct.to_u32().unwrap_or(0)
}

/// Formats the ACTUAL cell for a native period row.
///
/// In `pct_mode`, returns core's paced ratio as `"N%"`, or `–` when core sent
/// none. Falls back to the spent amount when tracking-only, or `—` when the
/// row has no spend to state.
fn actual_str(
    row: &NativePeriodRow,
    pct_mode: bool,
    currencies: &[bc_ipc::CommodityInfo],
) -> String {
    if pct_mode && row.effective_target.is_some() {
        pct_label(row.ratio)
    } else {
        row.spent.as_ref().map_or_else(
            || "\u{2014}".to_owned(),
            |s| crate::pages::budget::money::fmt(s, currencies),
        )
    }
}

/// Formats the TARGET cell for a native period row: empty when tracking-only.
///
/// The target resolves its own `currency_code`, which may differ from the
/// spend's.
fn target_str(row: &NativePeriodRow, currencies: &[bc_ipc::CommodityInfo]) -> String {
    row.effective_target.as_ref().map_or_else(String::new, |t| {
        crate::pages::budget::money::fmt(t, currencies)
    })
}

/// Inline expandable list showing native period breakdown for a mixed-period budget.
///
/// Reads [`BudgetPageCtx`] from context for the current display period and window
/// start, then fetches sub-period rows via IPC.
#[component]
pub fn NativePeriodList(
    /// ID of the budget whose native periods are being displayed.
    #[prop(into)]
    budget_id: String,
    /// Nesting depth of this list (used for indentation).
    depth: u32,
) -> impl IntoView {
    let ctx = expect_context::<BudgetPageCtx>();
    let period = ctx.display_period;
    let window_start = ctx.window_start;
    let pct_mode = ctx.pct_mode;
    let data_version = ctx.data_version;
    let currencies = crate::currency_ctx::use_currency_store();
    let filter_store = crate::filter_ctx::use_filter_store();

    let rows: LocalResource<Result<Vec<NativePeriodRow>, BcError>> =
        LocalResource::new(move || {
            let bid = budget_id.clone();
            data_version.get();
            let p = period.get();
            let start = window_start.get();
            let end = period_nav::step_window(&p, start, true);
            let eff = filter_store
                .filter
                .with(crate::pages::budget::query::budget_effective_filter);
            async move {
                let filter = (eff != bc_ipc::Filter::default()).then_some(eff);
                bc_ipc::client::get_native_periods(&bid, start, end, filter.as_ref()).await
            }
        });

    let indent_style = format!("--row-depth:{depth}");

    view! {
        <Suspense fallback=move || {
            view! { <div class=style::loading>"Loading periods…"</div> }
        }>
            {move || {
                let pct = pct_mode.get();
                let currencies = currencies.get();
                rows.get()
                    .map(|result| match result {
                        Err(e) => {
                            view! { <div class=style::error>{format!("Error: {e}")}</div> }
                                .into_any()
                        }
                        Ok(period_rows) => {
                            let rows_view = period_rows
                                .into_iter()
                                .map(|row| {
                                    let tone = verdict_tone(row.verdict);
                                    let fill_pct = fill_percent(&row);
                                    let fill_style = format!("width: {fill_pct}%; height: 100%");
                                    let actual = actual_str(&row, pct, &currencies);
                                    let target = target_str(&row, &currencies);
                                    let label = row.label.clone();
                                    let unvalued_pill = unvalued_label(&row.unvalued)
                                        .map(|l| {
                                            view! {
                                                <span class=style::unvalued>
                                                    <StatusPill label=l tone=Tone::Warn />
                                                </span>
                                            }
                                        });
                                    let status_class = match tone {
                                        VerdictTone::Good => style::status_good,
                                        VerdictTone::Warn => style::status_warn,
                                        VerdictTone::Bad => style::status_bad,
                                        VerdictTone::Mute => style::status_mute,
                                    };
                                    let bar_class = match tone {
                                        VerdictTone::Good => style::bar_good,
                                        VerdictTone::Warn => style::bar_warn,
                                        VerdictTone::Bad => style::bar_bad,
                                        VerdictTone::Mute => style::bar_mute,
                                    };

                                    view! {
                                        <div class=style::sub_row style=indent_style.clone()>
                                            <span class=status_class>{label}</span>
                                            <div class=style::bar_track>
                                                <div class=bar_class style=fill_style />
                                            </div>
                                            <span class=style::amounts>{actual}</span>
                                            <span class=style::amounts>{target}</span>
                                            {unvalued_pill}
                                        </div>
                                    }
                                })
                                .collect_view();
                            view! { <div>{rows_view}</div> }.into_any()
                        }
                    })
            }}
        </Suspense>
    }
}
