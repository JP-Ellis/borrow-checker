//! QA showcase for [`NewBudget`](super::NewBudget).

use leptos::prelude::*;

use super::NewBudget;

/// Renders the new budget creation form.
#[component]
pub fn NewBudgetQa() -> impl IntoView {
    let noop = Callback::new(|()| ());

    view! {
        <div style="display:flex;flex-direction:column;gap:24px;max-width:480px;padding:24px">
            <h3>"New budget form"</h3>
            <NewBudget on_created=noop on_cancel=noop />
        </div>
    }
}
