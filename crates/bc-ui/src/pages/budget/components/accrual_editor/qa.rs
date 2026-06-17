//! QA page for [`super::AccrualEditor`].

use leptos::prelude::*;

use super::AccrualEditor;

/// Renders [`AccrualEditor`] in all relevant states for visual QA.
#[component]
pub fn AccrualEditorQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "no existing spread (save only)"
                </p>
                <AccrualEditor posting_id="posting-001" on_change=Callback::new(|()| {}) />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "existing spread set (save + remove)"
                </p>
                <AccrualEditor
                    posting_id="posting-002"
                    spread_from="2026-06-01"
                    spread_until="2026-06-30"
                    on_change=Callback::new(|()| {})
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "saving in progress (buttons disabled)"
                </p>
                <AccrualEditorSavingState />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "error state"
                </p>
                <AccrualEditorErrorState />
            </section>

        </div>
    }
}

/// Renders the editor in a saving-in-progress state for QA purposes.
#[component]
fn AccrualEditorSavingState() -> impl IntoView {
    /* Simulates the disabled state by rendering with pre-filled dates.
    In real usage, saving=true disables both buttons. */
    view! {
        <div style="opacity:0.6;pointer-events:none">
            <AccrualEditor
                posting_id="posting-003"
                spread_from="2026-07-01"
                spread_until="2026-07-31"
                on_change=Callback::new(|()| {})
            />
        </div>
        <p style="font-size:10px;color:var(--bc-ink-mute);margin-top:4px;">
            "(buttons appear disabled when a network request is in flight)"
        </p>
    }
}

/// Renders the editor in an error state for QA purposes.
#[component]
fn AccrualEditorErrorState() -> impl IntoView {
    /* The real error signal is set internally on IPC failure.
    This wrapper triggers an error by using a clearly invalid posting ID
    format — shown here as a static display for visual reference. */
    view! {
        <div>
            <AccrualEditor
                posting_id="posting-invalid-error-qa"
                spread_from="2026-08-01"
                spread_until="2026-08-31"
                on_change=Callback::new(|()| {})
            />
            <p style="font-size:10px;color:var(--bc-ink-mute);margin-top:4px;">
                "(click Save spread to trigger an IPC error in dev mode)"
            </p>
        </div>
    }
}
