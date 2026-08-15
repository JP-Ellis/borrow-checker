//! QA showcase for [`MetaEditor`](super::MetaEditor).
//!
//! Every value type plus the six states a row can be in that are not simply
//! "typed and fine": mismatched, unknown account, tombstone, untyped key, a
//! value with no key, and a key the backend would reject. All data here is
//! invented.

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::CommodityInfo;
use bc_ipc::MetaEntryDto;
use bc_ipc::MetaKeyDefDto;
use bc_ipc::MetaTypeDto;
use bc_ipc::MetaValueDto;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::MetaEditor;
use super::model::MetaDraft;
use super::model::MetaRow;
use super::model::emit_rows;
use super::model::rows_from_entries;
use crate::currency_ctx::CurrencyStore;
use crate::meta_keys_ctx::MetaKeyStore;

/// A fake account id in the backend's shape; no such account exists.
const MISSING_ACCOUNT: &str = "account_00000000000000000000000000";

/// The accounts the picker offers.
fn qa_accounts() -> Vec<AccountRef> {
    vec![
        AccountRef::new("checking", "Assets :: Checking"),
        AccountRef::new("groceries", "Expenses :: Groceries"),
        AccountRef::new("household", "Expenses :: Household"),
    ]
}

/// A registry snapshot typing every key the showcase uses, except `shipment`,
/// which is deliberately absent so its row renders untyped.
fn qa_keys() -> Vec<MetaKeyDefDto> {
    vec![
        MetaKeyDefDto::new("payee", MetaTypeDto::Text),
        MetaKeyDefDto::new("invoice", MetaTypeDto::Number),
        MetaKeyDefDto::new("reimbursed", MetaTypeDto::Boolean),
        MetaKeyDefDto::new("cleared", MetaTypeDto::Date),
        MetaKeyDefDto::new("seen-at", MetaTypeDto::Timestamp),
        MetaKeyDefDto::new("budgeted", MetaTypeDto::Amount),
        MetaKeyDefDto::new("offset", MetaTypeDto::Account),
        MetaKeyDefDto::new("settled", MetaTypeDto::Date),
    ]
}

/// The commodities the amount row's selector offers.
fn qa_currencies() -> Vec<CommodityInfo> {
    vec![
        CommodityInfo::new("c1", "AUD", Some("A$".to_owned()), vec![], 2, true, false),
        CommodityInfo::new("c2", "USD", Some("US$".to_owned()), vec![], 2, false, false),
    ]
}

/// One row of each value type, then one of each broken state.
fn qa_rows() -> Vec<MetaRow> {
    let mut rows = rows_from_entries(&[
        MetaEntryDto::new("payee", MetaValueDto::Text("Generic Grocer".to_owned())),
        MetaEntryDto::new("invoice", MetaValueDto::Number(Decimal::new(150_250, 2))),
        MetaEntryDto::new("reimbursed", MetaValueDto::Boolean(true)),
        MetaEntryDto::new("cleared", MetaValueDto::Date(jiff::civil::date(2026, 5, 2))),
        MetaEntryDto::new(
            "seen-at",
            MetaValueDto::Timestamp(jiff::Timestamp::UNIX_EPOCH),
        ),
        MetaEntryDto::new(
            "budgeted",
            MetaValueDto::Amount(Amount::new(Decimal::new(4_200, 2), "AUD")),
        ),
        MetaEntryDto::new("offset", MetaValueDto::Account("groceries".to_owned())),
        // The store could not fit this value into `settled`'s registered `date`.
        MetaEntryDto::flagged("settled", "sometime in May"),
        // An account entry pointing at an id the account tree does not hold.
        MetaEntryDto::new("offset", MetaValueDto::Account(MISSING_ACCOUNT.to_owned())),
        // An `account` key holding text: the account was deleted and the path
        // froze where the foreign key used to be.
        MetaEntryDto::new(
            "offset",
            MetaValueDto::Text("Expenses :: Retired Account".to_owned()),
        ),
        // A key no registry snapshot knows — untyped, read-only, preserved.
        MetaEntryDto::new("shipment", MetaValueDto::Text("in transit".to_owned())),
    ]);
    // A value with no key: pruned on save, hinted while editing.
    rows.push(MetaRow {
        uid: 99,
        source: None,
        draft: Some(MetaDraft {
            key: String::new(),
            ty: MetaTypeDto::Text,
            text: "orphaned value".to_owned(),
            boolean: false,
            commodity: "AUD".to_owned(),
            account_id: String::new(),
        }),
    });
    // A key the backend would reject: hinted with the rule it broke, offered no
    // create row, and pruned on save.
    rows.push(MetaRow {
        uid: 100,
        source: None,
        draft: Some(MetaDraft {
            key: "due date".to_owned(),
            ty: MetaTypeDto::Text,
            text: "next tuesday".to_owned(),
            boolean: false,
            commodity: "AUD".to_owned(),
            account_id: String::new(),
        }),
    });
    rows
}

/// Renders the editor against an invented buffer, with the entries it would
/// save printed underneath.
#[component]
pub fn MetaEditorQa() -> impl IntoView {
    provide_context(CurrencyStore(RwSignal::new(qa_currencies())));
    provide_context(MetaKeyStore(RwSignal::new(qa_keys())));
    let rows: RwSignal<Vec<MetaRow>> = RwSignal::new(qa_rows());

    view! {
        <div style="max-width: 52rem; padding: 1rem;">
            <MetaEditor
                rows=rows.read_only().into()
                on_change=Callback::new(move |next: Vec<MetaRow>| rows.set(next))
                accounts=qa_accounts()
                default_commodity=Signal::derive(|| "AUD".to_owned())
            />
            <p style="margin-top: 1rem; font-family: var(--bc-font-mono); font-size: var(--bc-text-caption); color: var(--bc-ink-mute);">
                "saves as:"
            </p>
            <ul style="font-family: var(--bc-font-mono); font-size: var(--bc-text-caption); color: var(--bc-ink-mute);">
                {move || {
                    emit_rows(&rows.get())
                        .into_iter()
                        .map(|entry| {
                            view! { <li>{format!("{} = {:?}", entry.key, entry.value)}</li> }
                        })
                        .collect::<Vec<_>>()
                }}
            </ul>
        </div>
    }
}
