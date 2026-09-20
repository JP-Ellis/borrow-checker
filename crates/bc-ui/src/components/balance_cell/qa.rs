//! QA page for [`super::BalanceCellView`].

use leptos::prelude::*;
use rust_decimal::Decimal;

use super::BalanceCell;
use super::BalanceCellView;

/// One labelled cell at a fixed column width.
#[component]
fn Case(
    /// Row label shown in the left column.
    label: &'static str,
    /// The cell to render, or `None` for the mixed-commodity case.
    cell: Option<BalanceCell>,
) -> impl IntoView {
    view! {
        <div style="display:grid;grid-template-columns:200px 150px;gap:16px;align-items:center;height:32px">
            <span style="font-family:var(--bc-font-mono);font-size:12px;color:var(--bc-ink-mute)">
                {label}
            </span>
            <BalanceCellView cell=Signal::derive(move || cell.clone()) />
        </div>
    }
}

/// Renders every axis shape the register can produce.
#[component]
pub fn BalanceCellQa() -> impl IntoView {
    let aud = |cents: i64| bc_ipc::Amount::new(Decimal::new(cents, 2), "AUD");
    let straddle = (Decimal::new(-15_000, 2), Decimal::new(10_000, 2));
    let positive = (Decimal::ZERO, Decimal::new(20_000, 2));
    let negative = (Decimal::new(-20_000, 2), Decimal::ZERO);
    view! {
        <div style="display:flex;flex-direction:column;gap:8px;padding:24px;max-width:480px">
            <Case
                label="straddle −120"
                cell=Some(BalanceCell {
                    amount: aud(-12_000),
                    axis: straddle,
                })
            />
            <Case
                label="straddle +80"
                cell=Some(BalanceCell {
                    amount: aud(8_000),
                    axis: straddle,
                })
            />
            <Case
                label="all positive 50/200"
                cell=Some(BalanceCell {
                    amount: aud(5_000),
                    axis: positive,
                })
            />
            <Case
                label="all negative −50/−200"
                cell=Some(BalanceCell {
                    amount: aud(-5_000),
                    axis: negative,
                })
            />
            <Case
                label="degenerate 0"
                cell=Some(BalanceCell {
                    amount: aud(0),
                    axis: (Decimal::ZERO, Decimal::ZERO),
                })
            />
            <Case label="mixed commodity" cell=None />
        </div>
    }
}
