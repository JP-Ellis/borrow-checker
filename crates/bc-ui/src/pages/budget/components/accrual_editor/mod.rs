//! Inline accrual spread editor for a posting.
// MARK: Stubs
// Stubs only — props are intentionally unused until later tasks.
#![allow(
    dead_code,
    reason = "stub component — fields will be read when this component is implemented"
)]

use leptos::prelude::*;

/// Inline editor for setting or clearing the accrual spread on a posting.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    unused_variables,
    reason = "stub — props will be consumed in later tasks"
)]
pub fn AccrualEditor(
    /// ID of the posting being edited.
    #[prop(into)]
    posting_id: String,
    /// Current spread start date (ISO-8601), if one is set.
    #[prop(into, optional)]
    spread_from: Option<String>,
    /// Current spread end date (ISO-8601), if one is set.
    #[prop(into, optional)]
    spread_until: Option<String>,
    /// Callback invoked after a successful save or clear.
    on_change: Callback<()>,
) -> impl IntoView {
    view! { <div class="accrual-editor" /> }
}
