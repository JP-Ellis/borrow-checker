//! QA showcase for [`FilterChips`].

use bc_query::catalog::PathEntry;
use bc_query::catalog::Snapshot;
use leptos::prelude::*;

use super::FilterChips;
use crate::filter_ctx::provide_filter_store;
use crate::query_catalog_ctx::provide_fixed_query_catalog;

/// QA fixture: serves a fixed catalog and seeds the filter store with one chip
/// per kind of conjunct (an or-group of accounts, a tag, a word, a date bound,
/// two statuses, an amount bound and an account the catalog lacks). It renders
/// the chips, each label an edit button, plus a "clear all" reset for
/// re-checking the empty state.
///
/// The account chips show their shortest endings. The missing account's chip
/// carries an error.
#[component]
pub fn FilterChipsQa() -> impl IntoView {
    let _catalog = provide_fixed_query_catalog(Snapshot::new(
        vec![
            PathEntry::new("a1", ["Assets", "BankA", "Holiday"]),
            PathEntry::new("a2", ["Assets", "BankC", "Offset", "Holiday"]),
        ],
        vec![PathEntry::new("t1", ["groceries"])],
        Vec::new(),
        Vec::new(),
    ));
    let store = provide_filter_store();
    store.filter.update(|f| {
        "(account:Assets:BankA:Holiday or account:Assets:BankC:Offset:Holiday) \
         tag:groceries amazon date:>=2026-01-01 status:flagged status:unbalanced amount:>=100 \
         account:Assets:Gone"
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
