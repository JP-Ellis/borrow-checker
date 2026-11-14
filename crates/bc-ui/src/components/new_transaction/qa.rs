//! QA showcase for the new-transaction modal, one framed state per heading.

use bc_ipc::AccountNode;
use bc_ipc::AccountRef;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use bc_ipc::CommodityInfo;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::NewTransactionModal;
use crate::currency_ctx::CurrencyStore;

/// A transformed box is the containing block for the fixed overlay, so each
/// state renders in a frame of its own.
const FRAME: &str = "position:relative;height:720px;margin-bottom:var(--bc-space-6);\
                     transform:translateZ(0);border:1px solid var(--bc-border)";

/// Builds an account node, funded in `currency` when one is given.
fn account(id: &str, name: &str, parent: Option<&str>, currency: Option<&str>) -> AccountNode {
    AccountNode::new(
        id,
        name,
        None::<String>,
        currency.map(|c| Amount::new(Decimal::new(1_250, 2), c)),
        parent,
        AccountType::Asset,
        vec![],
        None,
        None,
    )
}

/// The fixture account tree: `Assets :: Cash` in AUD, `Expenses :: Food` in
/// AUD, and `Assets :: Empty` with no commodity.
fn qa_nodes() -> Vec<AccountNode> {
    vec![
        account("assets", "Assets", None, None),
        account("cash", "Cash", Some("assets"), Some("AUD")),
        account("empty", "Empty", Some("assets"), None),
        account("expenses", "Expenses", None, None),
        account("food", "Food", Some("expenses"), Some("AUD")),
    ]
}

/// The commodity set the fixture amounts resolve against.
fn qa_currencies() -> Vec<CommodityInfo> {
    vec![CommodityInfo::new(
        "c-aud",
        "AUD",
        Some("A$".to_owned()),
        vec![],
        2,
        true,
        false,
    )]
}

/// Renders the modal in each state, each in its own frame.
///
/// Cases:
/// - Unseeded: a blank draft, as the top bar opens it
/// - Seeded with `Assets :: Cash`: the first leg in that account and AUD, as a
///   budget detail opens it
/// - Loading: the account list has not arrived
/// - Error: the account list failed to load
#[component]
pub fn NewTransactionQa() -> impl IntoView {
    provide_context(CurrencyStore(RwSignal::new(qa_currencies())));
    let loaded = Signal::stored(Some(Ok(qa_nodes())));
    let on_close = Callback::new(|()| leptos::logging::log!("QA overlay closed"));
    // Each modal mounts an editor that provides its own context; a closure
    // gives each one an owner of its own.
    view! {
        <h3>"Unseeded"</h3>
        <div style=FRAME>
            {move || {
                view! { <NewTransactionModal seed=None accounts=loaded on_close=on_close /> }
            }}
        </div>

        <h3>"Seeded — Assets :: Cash"</h3>
        <div style=FRAME>
            {move || {
                view! {
                    <NewTransactionModal
                        seed=Some(AccountRef::new("cash", "Cash"))
                        accounts=loaded
                        on_close=on_close
                    />
                }
            }}
        </div>

        <h3>"Loading accounts"</h3>
        <div style=FRAME>
            {move || {
                view! {
                    <NewTransactionModal
                        seed=None
                        accounts=Signal::stored(None)
                        on_close=on_close
                    />
                }
            }}
        </div>

        <h3>"Account list failed"</h3>
        <div style=FRAME>
            {move || {
                view! {
                    <NewTransactionModal
                        seed=None
                        accounts=Signal::stored(Some(Err("connection refused".to_owned())))
                        on_close=on_close
                    />
                }
            }}
        </div>
    }
}
