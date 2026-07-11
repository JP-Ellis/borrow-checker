//! QA showcase for [`FilterChips`](super::FilterChips).

use leptos::prelude::*;

use super::FilterChips;
use crate::filter_ctx::provide_filter_store;

/// QA fixture: seeds the filter store with one chip per active value (named
/// accounts/tags, separate date and amount bounds, text, status) and renders the
/// chips, plus a "clear all" reset for re-checking the empty state.
#[component]
pub fn FilterChipsQa() -> impl IntoView {
    let store = provide_filter_store();
    /* Use the same entry points as the palette so account/tag labels resolve. */
    store.add_account("acc-food".to_owned(), "Food".to_owned());
    store.add_account("acc-transport".to_owned(), "Transport".to_owned());
    store.add_tag("tag-groceries".to_owned(), "groceries".to_owned());
    store.filter.update(|f| {
        f.text = Some("amazon".to_owned());
        f.date_from = "2026-01-01".parse().ok();
        f.reconciliation = Some(bc_ipc::Reconciliation::Flagged);
        let mut amount = bc_ipc::AmountFilter::default();
        amount.min = "100".parse().ok();
        f.amount = Some(amount);
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
