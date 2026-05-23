//! QA page for [`super::StatusPill`].

use leptos::prelude::*;

use super::StatusPill;
use super::Tone;

/// Renders [`StatusPill`] across all [`Tone`] variants.
#[component]
pub fn StatusPillQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:24px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "typical labels — one per tone"
                </p>
                <div style="display:flex;flex-direction:column;gap:8px">
                    <StatusPill label="synced".to_owned() tone=Tone::Good />
                    <StatusPill label="pending".to_owned() tone=Tone::Warn />
                    <StatusPill label="error".to_owned() tone=Tone::Bad />
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "inline row — all tones side by side"
                </p>
                <div style="display:flex;gap:8px">
                    <StatusPill label="good".to_owned() tone=Tone::Good />
                    <StatusPill label="warn".to_owned() tone=Tone::Warn />
                    <StatusPill label="bad".to_owned() tone=Tone::Bad />
                </div>
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "long label — overflow"
                </p>
                <StatusPill label="a very long status label".to_owned() tone=Tone::Warn />
            </section>

        </div>
    }
}
