//! QA page for [`super::NativePeriodList`].
//!
//! Shows a live component state plus a static fixture preview of all status
//! colours (good / warn / bad / mute). The live component performs a real IPC
//! call which fails in the QA harness, showing the error state after loading.

use bc_ipc::Amount;
use bc_ipc::NativePeriodRow;
use leptos::prelude::*;

use super::NativePeriodList;
use super::style;
use crate::pages::budget::BudgetPageCtx;

/// Builds a fixture [`NativePeriodRow`] with an explicit target.
fn row_with_target(
    label: &str,
    period_start: &str,
    period_end: &str,
    spent: i64,
    target: i64,
) -> NativePeriodRow {
    NativePeriodRow::new(
        label,
        period_start,
        period_end,
        Some(Amount::new(target, "AUD", 2)),
        Amount::new(spent, "AUD", 2),
    )
}

/// Builds a fixture [`NativePeriodRow`] with no target (tracking).
fn row_no_target(label: &str, period_start: &str, period_end: &str, spent: i64) -> NativePeriodRow {
    NativePeriodRow::new(
        label,
        period_start,
        period_end,
        None,
        Amount::new(spent, "AUD", 2),
    )
}

/// Renders [`NativePeriodList`] in several fixture states.
///
/// States shown:
/// 1. Component wired into a real [`BudgetPageCtx`] — will show the loading
///    fallback while the (always-failing in QA) IPC call is in flight, then
///    fall through to the error state.
/// 2. Inline static rows illustrating the three status colours (good / warn /
///    bad) and the tracking/no-target (mute) variant, rendered directly without
///    the async fetch so they are always visible.
#[component]
pub fn NativePeriodListQa() -> impl IntoView {
    let ctx = BudgetPageCtx::new();
    provide_context(ctx);

    /* Fixture rows for the inline static preview. */
    let row_good = row_with_target("w24 · 9–15 Jun", "2026-06-09", "2026-06-16", 41_600, 80_000);
    let row_warn = row_with_target(
        "w25 · 16–22 Jun",
        "2026-06-16",
        "2026-06-23",
        68_000,
        80_000,
    );
    let row_bad = row_with_target(
        "w26 · 23–29 Jun",
        "2026-06-23",
        "2026-06-30",
        96_000,
        80_000,
    );
    let row_mute = row_no_target("w23 · 2–8 Jun", "2026-06-02", "2026-06-09", 12_400);

    view! {
        <div style="padding: 24px; max-width: 900px">
            <h2 style="font-size: var(--bc-text-body); margin-bottom: var(--bc-space-4)">
                "NativePeriodList QA"
            </h2>

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-bottom: var(--bc-space-3)">
                "live component (IPC will fail in QA → shows error state after loading)"
            </p>
            <NativePeriodList budget_id="groceries" depth=0 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-6); margin-bottom: var(--bc-space-3)">
                "static fixture rows: good / warn / bad / mute (depth=1)"
            </p>
            <NativePeriodRowPreview rows=vec![row_good, row_warn, row_bad, row_mute] depth=1 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-4); margin-bottom: var(--bc-space-3)">
                "depth=2 indentation"
            </p>
            <NativePeriodList budget_id="dining" depth=2 />

            <p style="font-size: var(--bc-text-caption); color: var(--bc-ink-mute); margin-top: var(--bc-space-6); margin-bottom: var(--bc-space-3)">
                "static loading / error state appearance"
            </p>
            <div class=style::loading>"Loading periods…"</div>
            <div class=style::error>"Error: IPC command not available in QA"</div>
        </div>
    }
}

/// Renders a static list of [`NativePeriodRow`] fixture data without IPC,
/// reusing the same CSS as [`NativePeriodList`].
#[component]
fn NativePeriodRowPreview(
    /// Fixture rows to display.
    rows: Vec<NativePeriodRow>,
    /// Nesting depth for indentation.
    depth: u32,
) -> impl IntoView {
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "depth is bounded (u32 with small values) and 32 + depth * 16 cannot realistically overflow"
    )]
    let pad_left = 32_u32 + depth * 16_u32;
    let indent_style = format!("padding-left: {pad_left}px");

    let rows_view = rows
        .into_iter()
        .map(|row| {
            let row_status = super::row_status(&row);
            let pct = super::fill_percent(&row);
            let fill_style = format!("width: {pct}%; height: 100%");
            let amounts = super::display_str(&row);
            let label = row.label.clone();

            let status_class = match row_status {
                super::Status::Good => style::status_good,
                super::Status::Warn => style::status_warn,
                super::Status::Bad => style::status_bad,
                super::Status::Dim => style::status_dim,
                super::Status::Mute => style::status_mute,
            };

            let bar_class = match row_status {
                super::Status::Good => style::bar_good,
                super::Status::Warn => style::bar_warn,
                super::Status::Bad => style::bar_bad,
                super::Status::Dim | super::Status::Mute => style::bar_mute,
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

    view! { <div>{rows_view}</div> }
}
