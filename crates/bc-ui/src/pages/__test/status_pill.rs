//! [`StatusPill`] component QA page — renders all three tones.

use leptos::prelude::*;

use crate::components::status_pill::StatusPill;
use crate::components::status_pill::Tone;

/// Tests the [`StatusPill`] component across all [`Tone`] variants.
#[component]
pub fn StatusPillTest() -> impl IntoView {
    view! {
        <div class="page">
            <h1 style="font-size: 20px; margin-bottom: 12px;">"StatusPill component"</h1>

            <div style="display: flex; flex-direction: column; gap: 8px;">
                <StatusPill label="synced".to_owned() tone=Tone::Good />
                <StatusPill label="pending".to_owned() tone=Tone::Warn />
                <StatusPill label="error".to_owned() tone=Tone::Bad />
            </div>

            <div style="margin-top: 16px; display: flex; gap: 8px;">
                <StatusPill label="good".to_owned() tone=Tone::Good />
                <StatusPill label="warn".to_owned() tone=Tone::Warn />
                <StatusPill label="bad".to_owned() tone=Tone::Bad />
            </div>
        </div>
    }
}
