//! QA page for [`super::AccrualEditor`].

use leptos::prelude::*;

use super::AccrualEditor;

/// Renders [`AccrualEditor`] in spread-set and spread-unset states.
///
/// Note: this component is a stub. The QA page will be expanded when the
/// component is implemented in a later task.
#[component]
pub fn AccrualEditorQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "no existing spread"
                </p>
                <AccrualEditor posting_id="posting-001" on_change=Callback::new(|()| {}) />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "existing spread set"
                </p>
                <AccrualEditor
                    posting_id="posting-002"
                    spread_from="2026-06-01"
                    spread_until="2026-06-30"
                    on_change=Callback::new(|()| {})
                />
            </section>

        </div>
    }
}
