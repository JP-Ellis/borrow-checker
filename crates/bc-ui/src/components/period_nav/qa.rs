//! QA showcase for the shared [`PeriodNav`] and
//! [`WindowNav`] components.

use leptos::prelude::*;

use super::DisplayWindow;
use super::PeriodNav;
use super::WindowNav;

/// Renders `PeriodNav` in default and compact variants for visual QA.
#[component]
pub fn PeriodNavQa() -> impl IntoView {
    let period = RwSignal::new(bc_ipc::Period::Monthly);
    let start = RwSignal::new(jiff::civil::Date::constant(2026, 6, 1));
    let all_time = RwSignal::new(DisplayWindow::AllTime);
    let in_period = RwSignal::new(DisplayWindow::Period {
        period: bc_ipc::Period::Monthly,
        start: jiff::civil::Date::constant(2026, 6, 1),
    });
    view! {
        <div style="padding:var(--bc-space-6);display:flex;flex-direction:column;gap:var(--bc-space-6)">
            <h2>"PeriodNav — default"</h2>
            <PeriodNav period=period window_start=start />
            <h2>"PeriodNav — compact"</h2>
            <PeriodNav period=period window_start=start compact=true />
            <h2>"WindowNav — all time default"</h2>
            <WindowNav
                window=all_time
                on_change=Callback::new(move |(next, _)| all_time.set(next))
            />
            <h2>"WindowNav — period"</h2>
            <WindowNav
                window=in_period
                on_change=Callback::new(move |(next, _)| in_period.set(next))
                compact=true
            />
        </div>
    }
}
