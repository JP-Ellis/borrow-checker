//! QA page for [`super::NativePeriodList`].

use leptos::prelude::*;

use super::NativePeriodList;

/// Renders [`NativePeriodList`] with fixture data in stub state.
///
/// Note: this component is a stub. The QA page will be expanded when the
/// component is implemented in a later task.
#[component]
pub fn NativePeriodListQa() -> impl IntoView {
    view! {
        <div style="padding:24px">
            <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                "stub — will be expanded when NativePeriodList is implemented"
            </p>
            <NativePeriodList budget_id="groceries" depth=0 />
        </div>
    }
}
