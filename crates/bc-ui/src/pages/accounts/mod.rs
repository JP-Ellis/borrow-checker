//! Accounts page — account tree sidebar and transaction register.

mod types;

pub use bc_ipc::AccountNode;
pub use bc_ipc::Transaction;
pub use types::ACCOUNTS;
pub use types::TRANSACTIONS;

pub(crate) mod components;
pub(crate) mod dashboard;

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
pub fn Accounts() -> impl IntoView {
    // MARK: Route param
    let params = use_params_map();
    let selected_id = Signal::derive(move || params.with(|p| p.get("id")));

    // MARK: Sidebar state
    let sidebar_collapsed = {
        // Initialise collapsed on narrow viewports (≤ 480px, matching $bp-sm).
        // A full hamburger drawer for very narrow screens is deferred pending
        // device testing; the dot-rail handles 480px–desktop widths.
        // TODO(mobile): implement hamburger drawer fallback for sub-480px if dot-rail
        // proves too small on actual devices.
        let narrow = web_sys::window()
            .and_then(|w| w.inner_width().ok())
            .and_then(|v| v.as_f64())
            .is_some_and(|w| w <= 480.0_f64);
        RwSignal::new(narrow)
    };
    let toggle_sidebar = move |_: web_sys::MouseEvent| {
        sidebar_collapsed.update(|c| *c = !*c);
    };

    // MARK: Scroll detection
    let main_ref = NodeRef::<leptos::html::Div>::new();
    let dashboard_scrolled = RwSignal::new(false);

    let on_scroll = move |_: web_sys::Event| {
        if let Some(el) = main_ref.get() {
            dashboard_scrolled.set(el.scroll_top() > 180_i32);
        }
    };

    // MARK: Selected account
    let selected_node = Signal::derive(move || {
        let id = selected_id.get()?;
        ACCOUNTS.iter().find(|a| a.id == id).cloned()
    });

    let accounts: &'static [AccountNode] = &ACCOUNTS;
    let transactions: &'static [Transaction] = &TRANSACTIONS;

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
                <AccountSidebar
                    nodes=accounts
                    selected_id=selected_id
                    collapsed=sidebar_collapsed.read_only()
                />
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
                    Some(node) => view! { <AccountDashboard node=node /> }.into_any(),
                    None => {
                        view! {
                            <div class=style::empty_state>
                                "// select an account from the sidebar"
                            </div>
                        }
                            .into_any()
                    }
                }}

                {move || {
                    selected_node
                        .get()
                        .map(|node| {
                            // TODO: TRANSACTIONS is a global static not keyed by account ID.
                            // When IPC is wired up, replace with a Resource parameterised by
                            // node.id using the Resource + Suspense pattern from
                            // component-standards.md.
                            view! {
                                <TransactionRegister
                                    transactions=transactions
                                    viewing_account_id=node.id.clone()
                                />
                            }
                        })
                }}
            </div>
        </div>
    }
}
