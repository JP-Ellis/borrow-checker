//! QA showcase for the transfer-suggestion card (debug builds only).

use bc_ipc::Amount;
use bc_ipc::TransferSuggestion;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::SuggestionCard;

/// Renders a sample suggestion card for visual QA.
#[component]
pub fn TransfersPanelQa() -> impl IntoView {
    let sample = TransferSuggestion::new(
        "txn_debit",
        "txn_credit",
        Amount::new(Decimal::new(50000, 2), "AUD"),
        "2026-07-05",
        "2026-07-06",
        "Assets:Savings",
        "Liabilities:Mortgage",
        "TFR TO MORTGAGE 4471",
        "TRANSFER RECEIVED",
    );
    let noop_merge = Callback::new(|_: TransferSuggestion| {});
    let noop_dismiss = Callback::new(|_: TransferSuggestion| {});
    view! {
        <div style="padding:24px;max-width:34rem">
            <h2 style="font-family:var(--bc-font-mono);font-size:11px;color:var(--bc-ink-dim);\
            margin-bottom:16px;">"Transfer suggestion card"</h2>
            <SuggestionCard suggestion=sample on_merge=noop_merge on_dismiss=noop_dismiss />
        </div>
    }
}
