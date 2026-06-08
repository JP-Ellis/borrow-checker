//! Accounts page — account tree sidebar and transaction register.

mod types;

pub(crate) mod components;
pub(crate) mod dashboard;

use bc_ipc::Amount;
use bc_ipc::NewPosting;
use bc_ipc::NewTransaction;
use bc_ipc::TxStatus;
use components::sidebar::AccountSidebar;
use components::sticky_bar::StickyAccountBar;
use components::transaction_register::TransactionRegister;
use dashboard::AccountDashboard;
use leptos::prelude::*;
use leptos::web_sys;
use leptos_router::hooks::use_params_map;
use stylance::import_style;

import_style!(style, "accounts.module.scss");

/// Accounts page — sidebar + scrollable account dashboard and register.
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands verbosely; logic is straightforward"
)]
pub fn Accounts() -> impl IntoView {
    let params = use_params_map();
    let selected_id = Signal::derive(move || params.with(|p| p.get("id")));

    // Initialise collapsed on narrow viewports (≤ 480px, matching $bp-sm).
    let sidebar_collapsed = {
        let narrow = web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .is_some_and(|w| w <= 480.0_f64);
        RwSignal::new(narrow)
    };
    let toggle_sidebar = move |_: web_sys::MouseEvent| {
        sidebar_collapsed.update(|c| *c = !*c);
    };

    let main_ref = NodeRef::<leptos::html::Div>::new();
    let dashboard_scrolled = RwSignal::new(false);
    let on_scroll = move |_: web_sys::Event| {
        if let Some(el) = main_ref.get() {
            dashboard_scrolled.set(el.scroll_top() > 180_i32);
        }
    };

    // MARK: Live data

    // Monotonic counter bumped after any mutation that changes account or
    // transaction state (create, amend, void, import, …).  All resources that
    // show derived data subscribe to this signal so they re-fetch automatically
    // when anything changes — new actions only need to bump the counter.
    let data_version = RwSignal::new(0_u32);

    // LocalResource is required because bc_ipc::client futures are not Send
    // (they use js_sys::futures::JsFuture internally).
    let accounts_resource = LocalResource::new(move || {
        data_version.get(); // re-fetch whenever any mutation lands
        bc_ipc::client::list_accounts()
    });

    // Re-fetches whenever the selected account or data_version changes.
    // Note: `LocalResource::new` requires `Fn() -> Future`, which async closures
    // (`async move ||`) do not satisfy when they capture from the environment;
    // the `move || async move {}` form is required here.
    let transactions_resource = LocalResource::new(move || async move {
        data_version.get();
        match selected_id.get() {
            Some(ref id) => bc_ipc::client::list_transactions(id).await,
            None => Ok(vec![]),
        }
    });

    // Derive a flat signal from the resource for TransactionRegister.
    let transactions_signal = Signal::derive(move || {
        transactions_resource
            .get()
            .and_then(Result::ok)
            .unwrap_or_default()
    });

    // Derive selected node as a Signal so StickyAccountBar can receive it.
    let selected_node = Signal::derive(move || {
        let id = selected_id.get()?;
        let accounts = accounts_resource.get()?.ok()?;
        accounts.into_iter().find(|a| a.id == id)
    });

    let create_tx = Action::new_unsync(|tx: &NewTransaction| {
        let tx = tx.clone();
        async move { bc_ipc::client::create_transaction(&tx).await }
    });

    // After any successful mutation, bump data_version — all subscribed
    // resources react automatically.  Future actions (amend, void, import)
    // only need to add one line here.
    Effect::new(move |_| {
        if create_tx.value().with(|v| matches!(v, Some(Ok(_)))) {
            data_version.update(|v| *v = v.wrapping_add(1));
        }
    });

    view! {
        <div class=style::shell>
            // Sidebar
            <div class=move || {
                if sidebar_collapsed.get() {
                    format!("{} {}", style::sidebar, style::sidebar_collapsed)
                } else {
                    style::sidebar.to_owned()
                }
            }>
                // Inner scroll wrapper — keeps overflow-y: auto off the outer
                // sidebar so the absolutely-positioned toggle button can hang
                // outside the right edge without triggering a scrollbar.
                <div class=style::sidebar_content>
                    {move || match accounts_resource.get() {
                        None => {
                            view! { <div class=style::empty_state>"Loading accounts…"</div> }
                                .into_any()
                        }
                        Some(Err(e)) => {
                            view! { <div class=style::empty_state>{format!("Error: {e}")}</div> }
                                .into_any()
                        }
                        Some(Ok(accounts)) => {
                            view! {
                                <AccountSidebar
                                    nodes=accounts
                                    selected_id=selected_id
                                    collapsed=sidebar_collapsed.read_only()
                                />
                            }
                                .into_any()
                        }
                    }}
                </div>
                <button
                    class=style::sidebar_toggle
                    on:click=toggle_sidebar
                    aria-label="toggle sidebar"
                >
                    {move || if sidebar_collapsed.get() { "›" } else { "‹" }}
                </button>
            </div>

            // Main scrollable column
            <div class=style::main node_ref=main_ref on:scroll=on_scroll>
                <StickyAccountBar node=selected_node visible=dashboard_scrolled.read_only() />

                {move || match selected_node.get() {
                    None => {
                        view! {
                            <div class=style::empty_state>
                                "// select an account from the sidebar"
                            </div>
                        }
                            .into_any()
                    }
                    Some(node) => {
                        let offset_id = accounts_resource
                            .get()
                            .and_then(Result::ok)
                            .and_then(|accounts| {
                                accounts
                                    .into_iter()
                                    .find(|a| {
                                        matches!(a.account_type, bc_ipc::AccountType::Expense)
                                    })
                                    .map(|a| a.id)
                            })
                            .unwrap_or_default();
                        let debit_id = node.id.clone();

                        view! {
                            <AccountDashboard
                                node=node.clone()
                                data_version=data_version.read_only()
                            />

                            // TODO(ipc): replace with real add-transaction form
                            <button
                                data-testid="add-test-transaction"
                                on:click=move |_| {
                                    create_tx
                                        .dispatch(
                                            NewTransaction::new(
                                                "2026-05-23",
                                                "Test Payee",
                                                TxStatus::Pending,
                                                vec![],
                                                vec![
                                                    NewPosting::new(
                                                        debit_id.clone(),
                                                        Amount::new(-1_000, "AUD", 2),
                                                        None::<&str>,
                                                    ),
                                                    NewPosting::new(
                                                        offset_id.clone(),
                                                        Amount::new(1_000, "AUD", 2),
                                                        None::<&str>,
                                                    ),
                                                ],
                                            ),
                                        );
                                }
                            >
                                "Add Test Transaction"
                            </button>

                            <TransactionRegister
                                transactions=transactions_signal
                                viewing_account_id=node.id.clone()
                            />
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}
