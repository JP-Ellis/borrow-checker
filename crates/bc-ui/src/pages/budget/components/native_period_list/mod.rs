//! Expandable list of native sub-periods for a mixed-period budget row.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use bc_ipc::BcError;
use bc_ipc::NativePeriodRow;
use leptos::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use stylance::import_style;

use crate::pages::budget::BudgetPageCtx;
use crate::pages::budget::period_nav;

import_style!(pub(crate) style, "native.module.scss");

/// Row status for a native period sub-row, derived from spend vs. target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Status {
    /// Spend is ≤ 80% of target.
    Good,
    /// Spend is > 80% but ≤ 100% of target.
    Warn,
    /// Spend exceeds the target.
    Bad,
    /// Budget has a target but nothing has been spent yet.
    Dim,
    /// No target set.
    Mute,
}

/// Derives a [`Status`] from a native period row's spend and target figures.
///
/// Uses integer arithmetic for the 80% threshold to avoid floating-point.
/// The comparison `spent * 5 > target * 4` is equivalent to `spent/target > 0.8`
/// and safe because budget minor-unit values fit comfortably in i64.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "budget Decimal values are bounded and cannot overflow or panic"
)]
fn row_status(row: &NativePeriodRow) -> Status {
    match &row.effective_target {
        None => Status::Mute,
        Some(_) if row.spent.value == Decimal::ZERO => Status::Dim,
        Some(target) if row.spent.value > target.value => Status::Bad,
        Some(target) => {
            if row.spent.value * Decimal::from(5_i64) > target.value * Decimal::from(4_i64) {
                Status::Warn
            } else {
                Status::Good
            }
        }
    }
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
    let spent = row.spent.value.max(Decimal::ZERO);
    let pct = (spent * Decimal::from(100_i64) / target.value).min(Decimal::from(100_i64));
    pct.to_u32().unwrap_or(0)
}

/// Formats the amounts column string for a native period row.
///
/// In `pct_mode`, returns `"N%"` (integer, spent ÷ target × 100).
/// Falls back to absolute amounts when tracking-only or when target is zero.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "pct calculation: budget Decimal values are bounded; cannot overflow or panic"
)]
fn display_str(row: &NativePeriodRow, pct_mode: bool) -> String {
    match &row.effective_target {
        None => format!("{} \u{00b7} tracking", row.spent.format_short()),
        Some(target) if pct_mode => {
            if target.value == Decimal::ZERO {
                "\u{2013}".into()
            } else {
                let pct = (row.spent.value.max(Decimal::ZERO) * Decimal::from(100_i64)
                    / target.value)
                    .to_i64()
                    .unwrap_or(0);
                format!("{pct}%")
            }
        }
        Some(target) => format!("{} / {}", row.spent.format_short(), target.format_short()),
    }
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

    let rows: LocalResource<Result<Vec<NativePeriodRow>, BcError>> =
        LocalResource::new(move || {
            let bid = budget_id.clone();
            data_version.get();
            let p = period.get();
            let start = window_start.get();
            let end = period_nav::step_window(&p, start, true);
            async move { bc_ipc::client::get_native_periods(&bid, start, end).await }
        });

    let indent_style = format!("--row-depth:{depth}");

    view! {
        <Suspense fallback=move || {
            view! { <div class=style::loading>"Loading periods…"</div> }
        }>
            {move || {
                let pct = pct_mode.get();
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
                                    let status = row_status(&row);
                                    let fill_pct = fill_percent(&row);
                                    let fill_style = format!("width: {fill_pct}%; height: 100%");
                                    let amounts = display_str(&row, pct);
                                    let label = row.label.clone();
                                    let status_class = match status {
                                        Status::Good => style::status_good,
                                        Status::Warn => style::status_warn,
                                        Status::Bad => style::status_bad,
                                        Status::Dim => style::status_dim,
                                        Status::Mute => style::status_mute,
                                    };
                                    let bar_class = match status {
                                        Status::Good => style::bar_good,
                                        Status::Warn => style::bar_warn,
                                        Status::Bad => style::bar_bad,
                                        Status::Dim | Status::Mute => style::bar_mute,
                                    };

                                    view! {
                                        <div class=style::sub_row style=indent_style.clone()>
                                            <span class=status_class>{label}</span>
                                            <div class=style::bar_track>
                                                <div class=bar_class style=fill_style />
                                            </div>
                                            <span class=style::amounts>{amounts}</span>
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
