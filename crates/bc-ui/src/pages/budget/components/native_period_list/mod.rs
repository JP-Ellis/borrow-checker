//! Expandable list of native sub-periods for a mixed-period budget row.
// Stub only — fields are intentionally unused until this component is implemented.
#![allow(
    dead_code,
    reason = "stub component — fields will be read when this component is implemented"
)]

#[cfg(debug_assertions)]
pub(crate) mod qa;

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "native.module.scss");

/// Inline expandable list showing native period breakdown for a mixed-period budget.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    unused_variables,
    reason = "stub — props will be consumed in later tasks"
)]
pub fn NativePeriodList(
    /// ID of the budget whose native periods are being displayed.
    #[prop(into)]
    budget_id: String,
    /// Nesting depth of this list (used for indentation).
    depth: u32,
) -> impl IntoView {
    view! { <div class=style::root /> }
}
