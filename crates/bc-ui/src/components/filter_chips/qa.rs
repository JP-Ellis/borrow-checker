//! QA showcase for [`FilterChips`].

use leptos::prelude::*;

use super::FilterChips;
use crate::filter_ctx::provide_filter_store;

/// QA fixture: seeds the filter store with one chip per kind of conjunct (an
/// or-group of accounts, a tag, a word, a date bound, two statuses and an
/// amount bound) and renders the chips, each label an edit button, plus a
/// "clear all" reset for re-checking the empty state.
#[component]
pub fn FilterChipsQa() -> impl IntoView {
    let store = provide_filter_store();
    store.filter.update(|f| {
        "(account:Assets:BankA:Holiday or account:Assets:BankC:Offset:Holiday) \
         tag:groceries amazon date:>=2026-01-01 status:flagged status:unbalanced amount:>=100"
            .clone_into(&mut f.query);
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
