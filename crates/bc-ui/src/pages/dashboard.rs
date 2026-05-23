//! Dashboard — net worth, cashflow, accounts summary, budget health.

use bc_ipc::Money;
use leptos::prelude::*;

use crate::components::num::Num;
use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;
use crate::components::tag_token::TagToken;

/// Dashboard page stub.
#[component]
pub fn Dashboard() -> impl IntoView {
    view! {
        <div class="page page-dashboard">
            <p>"dashboard — coming in phase 2"</p>
            // Primitive components rendered with neutral/placeholder values so
            // they are live in the WASM binary and visually testable in Phase 1.
            <Num money=Money::new(0, "USD") />
            <TagToken label="example:tag".to_owned() />
            <StatusPill label="ok".to_owned() tone=Tone::Good />
        </div>
    }
}
