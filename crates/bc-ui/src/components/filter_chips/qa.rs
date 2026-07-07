//! QA showcase for [`FilterChips`](super::FilterChips).

use leptos::prelude::*;

use super::FilterChips;
use crate::filter_ctx::provide_filter_store;

/// QA fixture: seeds the filter store with several active dimensions and
/// renders the chips, plus a "clear all" reset for re-checking the empty state.
#[component]
pub fn FilterChipsQa() -> impl IntoView {
    let store = provide_filter_store();
    store.filter.update(|f| {
        f.accounts = vec!["Expenses:Food".to_owned(), "Expenses:Transport".to_owned()];
        f.tags = vec!["t1".to_owned(), "t2".to_owned()];
        f.text = Some("amazon".to_owned());
    });

    view! {
        <div style="padding:24px;max-width:600px;display:flex;flex-direction:column;gap:16px;">
            <FilterChips />
            <button on:click=move |_| {
                store.filter.set(bc_ipc::Filter::default());
            }>"clear all"</button>
        </div>
    }
}
