//! QA showcase for the shared [`PeriodNav`](super::PeriodNav) component.

use leptos::prelude::*;

use super::PeriodNav;

/// Renders `PeriodNav` in default and compact variants for visual QA.
#[component]
pub fn PeriodNavQa() -> impl IntoView {
    let period = RwSignal::new(bc_ipc::Period::Monthly);
    let start = RwSignal::new(jiff::civil::Date::constant(2026, 6, 1));
    view! {
        <div style="padding:var(--bc-space-6);display:flex;flex-direction:column;gap:var(--bc-space-6)">
            <h2>"PeriodNav — default"</h2>
            <PeriodNav period=period window_start=start />
            <h2>"PeriodNav — compact"</h2>
            <PeriodNav period=period window_start=start compact=true />
        </div>
    }
}
