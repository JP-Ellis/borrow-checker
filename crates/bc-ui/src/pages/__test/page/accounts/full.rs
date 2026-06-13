//! Route entry and QA component for `/__test/page/accounts/full`.
//!
//! Assembles the full accounts view: sidebar + dashboard hero + transaction register.

use bc_ipc::AccountNode;
use bc_ipc::AccountRef;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::accounts::components::sidebar::AccountSidebar;

import_style!(style, "full.module.scss");
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
        Some(Amount::new(421_842, "AUD", 2)),
        Some("commbank"),
        AccountType::Asset,
        vec![
            "institution:commbank".to_owned(),
            "type:transactional".to_owned(),
        ],
    )
}

/// Returns sample account nodes for the sidebar.
fn sample_accounts() -> Vec<AccountNode> {
    vec![
        smart_access_node(),
        AccountNode::new(
            "commbank",
            "CommBank",
            None::<&str>,
            Some(Amount::new(6_421_000, "AUD", 2)),
            None::<&str>,
            AccountType::Asset,
            vec![],
        ),
        AccountNode::new(
            "amex-platinum",
            "Amex Platinum",
            Some("9001"),
            Some(Amount::new(-244_000, "AUD", 2)),
            None::<&str>,
            AccountType::Liability,
            vec!["type:credit".to_owned()],
        ),
    ]
}

/// Returns sample transactions for the Smart Access account.
fn sample_transactions() -> Vec<Transaction> {
    vec![
        Transaction::new(
            "tx-coles-2026-04-30",
            "2026-04-30",
            "Coles Carlton",
            TxStatus::Cleared,
            vec!["shared".to_owned()],
            vec![
                Posting::new(
                    AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                    Amount::new(-8_420, "AUD", 2),
                    None::<&str>,
                ),
                Posting::new(
                    AccountRef::new("groceries", "Expenses :: Groceries"),
                    Amount::new(8_420, "AUD", 2),
                    None::<&str>,
                ),
            ],
            vec![AuditEntry::new(
                "14:21",
                "import",
                "from commbank-au.wasm@1.4.2",
            )],
        ),
        Transaction::new(
            "tx-salary-2026-04-30",
            "2026-04-30",
            "Salary — Atlassian",
            TxStatus::Cleared,
            vec!["work".to_owned()],
            vec![
                Posting::new(
                    AccountRef::new("income-salary", "Income :: Salary"),
                    Amount::new(-846_154, "AUD", 2),
                    Some("gross pay"),
                ),
                Posting::new(
                    AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                    Amount::new(428_055, "AUD", 2),
                    Some("take-home"),
                ),
            ],
            vec![],
        ),
    ]
}

/// Full accounts view QA: sidebar, dashboard hero, and transaction register.
#[component]
pub fn AccountFullQa() -> impl IntoView {
    let selected_id: RwSignal<Option<String>> = RwSignal::new(Some("cb-smart-access".to_owned()));
    let (collapsed, _) = signal(false);

    view! {
        <div class=style::layout>
            <AccountSidebar
                nodes=sample_accounts()
                selected_id=selected_id.read_only().into()
                collapsed=collapsed
            />
            <div class=style::content>
                <AccountDashboard node=smart_access_node() />
                <TransactionRegister
                    transactions=Signal::derive(sample_transactions)
                    viewing_account_id="cb-smart-access"
                />
            </div>
        </div>
    }
}
