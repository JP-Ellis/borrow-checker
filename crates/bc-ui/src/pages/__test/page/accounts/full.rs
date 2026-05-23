//! Route entry and QA component for `/__test/page/accounts/full`.
//!
//! Assembles the full accounts view: sidebar + dashboard hero + transaction register.

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::Money;
use leptos::prelude::*;

use crate::pages::accounts::ACCOUNTS;
use crate::pages::accounts::TRANSACTIONS;
use crate::pages::accounts::components::sidebar::AccountSidebar;
use crate::pages::accounts::components::transaction_register::TransactionRegister;
use crate::pages::accounts::dashboard::AccountDashboard;

/// Display name shown in the QA index.
pub const TITLE: &str = "Full account view";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/full";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Sidebar + dashboard hero + transaction register composed together.";

/// Returns a sample `AccountNode` for the Smart Access account used in this QA view.
fn smart_access_node() -> AccountNode {
    AccountNode::new(
        "cb-smart-access",
        "Smart Access",
        Some("4421"),
        Money::new(421_842, "AUD"),
        Some("commbank"),
        AccountType::Asset,
        vec![
            "institution:commbank".to_owned(),
            "type:transactional".to_owned(),
        ],
    )
}

/// Full accounts view QA: sidebar, dashboard hero, and transaction register.
#[component]
pub fn AccountFullQa() -> impl IntoView {
    let selected_id: RwSignal<Option<String>> = RwSignal::new(Some("cb-smart-access".to_owned()));
    let (collapsed, _) = signal(false);

    view! {
        <div style="display:grid;grid-template-columns:200px 1fr;min-height:100vh">
            <AccountSidebar
                nodes=&*ACCOUNTS
                selected_id=selected_id.read_only().into()
                collapsed=collapsed
            />
            <div style="display:flex;flex-direction:column">
                <AccountDashboard node=smart_access_node() />
                <TransactionRegister
                    transactions=&*TRANSACTIONS
                    viewing_account_id="cb-smart-access"
                />
            </div>
        </div>
    }
}
