//! QA showcase for [`FilterChips`].

use leptos::prelude::*;

use super::FilterChips;
use crate::filter_ctx::provide_filter_store;

/// QA fixture: seeds the filter store with one chip per kind of conjunct (an
/// or-group of picked accounts, a tag, a word, a date bound, two statuses and
/// an amount bound) and renders the chips, plus a "clear all" reset for
/// re-checking the empty state.
#[component]
pub fn FilterChipsQa() -> impl IntoView {
    let store = provide_filter_store();
    /* Use the same entry points as the palette so account labels resolve. */
    store.add_account("Assets:BankA:Holiday", "BankA:Holiday");
    store.add_account("Assets:BankC:Offset:Holiday", "Offset:Holiday");
    store.add_tag("groceries");
    store.filter.update(|f| {
        f.query = format!(
            "{} amazon date:>=2026-01-01 status:flagged status:unbalanced amount:>=100",
            f.query
        );
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
