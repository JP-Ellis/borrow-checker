//! Inline accrual spread editor for a posting.

#[cfg(debug_assertions)]
pub(crate) mod qa;

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "accrual.module.scss");

/// Inline editor for setting or clearing the accrual spread on a posting.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props require owned types"
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
    let from_input = RwSignal::new(spread_from.clone().unwrap_or_default());
    let until_input = RwSignal::new(spread_until.clone().unwrap_or_default());
    let saving = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);

    let posting_id_for_save = posting_id.clone();
    let save = move |_| {
        let pid = posting_id_for_save.clone();
        let from = from_input.get();
        let until = until_input.get();
        saving.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::set_posting_spread(&pid, &from, &until).await {
                Ok(()) => {
                    saving.set(false);
                    on_change.run(());
                }
                Err(e) => {
                    saving.set(false);
                    error.set(Some(e.to_string()));
                }
            }
        });
    };

    let posting_id_for_clear = posting_id.clone();
    let clear = move |_| {
        let pid = posting_id_for_clear.clone();
        saving.set(true);
        error.set(None);
        leptos::task::spawn_local(async move {
            match bc_ipc::client::clear_posting_spread(&pid).await {
                Ok(()) => {
                    saving.set(false);
                    on_change.run(());
                }
                Err(e) => {
                    saving.set(false);
                    error.set(Some(e.to_string()));
                }
            }
        });
    };

    view! {
        <div class=style::editor>
            <div class=style::field_row>
                <label class=style::field_label>"From"</label>
                <input
                    type="date"
                    class=style::date_input
                    prop:value=move || from_input.get()
                    on:input=move |ev| from_input.set(event_target_value(&ev))
                />
            </div>
            <div class=style::field_row>
                <label class=style::field_label>"Until (exclusive)"</label>
                <input
                    type="date"
                    class=style::date_input
                    prop:value=move || until_input.get()
                    on:input=move |ev| until_input.set(event_target_value(&ev))
                />
            </div>
            {move || {
                error
                    .get()
                    .map(|msg| {
                        view! { <p class=style::err>{msg}</p> }
                    })
            }}
            <div class=style::btn_row>
                <button class=style::btn_save disabled=move || saving.get() on:click=save>
                    {move || if saving.get() { "Saving…" } else { "Save spread" }}
                </button>
                {spread_from
                    .is_some()
                    .then(|| {
                        view! {
                            <button
                                class=style::btn_remove
                                disabled=move || saving.get()
                                on:click=clear
                            >
                                "Remove spread"
                            </button>
                        }
                    })}
            </div>
        </div>
    }
}
