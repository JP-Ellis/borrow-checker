//! Route entry and QA component for `/__test/page/accounts/full`.
//!
//! Assembles the full accounts view: sidebar + dashboard hero + transaction register.

use bc_ipc::AccountNode;
use bc_ipc::AccountRef;
use bc_ipc::AccountType;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::FilteredTransaction;
use bc_ipc::Posting;
use bc_ipc::PostingAmount;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use leptos::prelude::*;
use stylance::import_style;

use crate::pages::accounts::components::sidebar::AccountSidebar;

import_style!(style, "full.module.scss");
use rust_decimal::Decimal;

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
        Some(Amount::new(Decimal::new(421_842, 2), "AUD")),
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
            "Bank",
            None::<&str>,
            Some(Amount::new(Decimal::new(6_421_000, 2), "AUD")),
            None::<&str>,
            AccountType::Asset,
            vec![],
        ),
        AccountNode::new(
            "amex-platinum",
            "Amex Platinum",
            Some("9001"),
            Some(Amount::new(Decimal::new(-244_000, 2), "AUD")),
            None::<&str>,
            AccountType::Liability,
            vec!["type:credit".to_owned()],
        ),
    ]
}

/// Returns sample transactions for the Smart Access account.
fn sample_transactions() -> Vec<FilteredTransaction> {
    vec![
        {
            let tx = coles_transaction();
            let matched = tx.postings.iter().map(|p| p.id.clone()).collect();
            FilteredTransaction::new(tx, matched)
        },
        {
            let tx = salary_transaction();
            let matched = tx.postings.iter().map(|p| p.id.clone()).collect();
            FilteredTransaction::new(tx, matched)
        },
    ]
}

/// Returns the sample Coles grocery transaction.
#[expect(
    clippy::expect_used,
    reason = "QA fixture — timestamp literals are valid"
)]
fn coles_transaction() -> Transaction {
    Transaction::new(
        "tx-coles-2026-04-30",
        jiff::civil::Date::constant(2026, 4, 30),
        "",
        vec![bc_ipc::MetaEntryDto::new(
            "payee",
            bc_ipc::MetaValueDto::Text("Generic Grocer".to_owned()),
        )],
        Reconciliation::Reconciled,
        vec!["shared".to_owned()],
        vec![
            Posting::new(
                "posting-coles-debit",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                PostingAmount::Stored(Amount::new(Decimal::new(-8_420, 2), "AUD")),
                vec![],
                vec![],
                None,
                None,
            ),
            Posting::new(
                "posting-coles-groceries",
                AccountRef::new("groceries", "Expenses :: Groceries"),
                PostingAmount::Stored(Amount::new(Decimal::new(8_420, 2), "AUD")),
                vec![],
                vec![],
                None,
                None,
            ),
        ],
        vec![AuditEntry::new(
            "2026-04-30T14:21:00Z"
                .parse::<jiff::Timestamp>()
                .expect("valid timestamp"),
            "import",
            "from commbank-au.wasm@1.4.2",
        )],
    )
}

/// Returns the sample salary transaction.
fn salary_transaction() -> Transaction {
    Transaction::new(
        "tx-salary-2026-04-30",
        jiff::civil::Date::constant(2026, 4, 30),
        "",
        vec![bc_ipc::MetaEntryDto::new(
            "payee",
            bc_ipc::MetaValueDto::Text("Generic Employer".to_owned()),
        )],
        Reconciliation::Reconciled,
        vec!["work".to_owned()],
        vec![
            Posting::new(
                "posting-salary-income",
                AccountRef::new("income-salary", "Income :: Salary"),
                PostingAmount::Stored(Amount::new(Decimal::new(-846_154, 2), "AUD")),
                vec![bc_ipc::MetaEntryDto::new(
                    "note",
                    bc_ipc::MetaValueDto::Text("gross pay".to_owned()),
                )],
                vec![],
                None,
                None,
            ),
            Posting::new(
                "posting-salary-takehome",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                PostingAmount::Stored(Amount::new(Decimal::new(428_055, 2), "AUD")),
                vec![bc_ipc::MetaEntryDto::new(
                    "note",
                    bc_ipc::MetaValueDto::Text("take-home".to_owned()),
                )],
                vec![],
                None,
                None,
            ),
        ],
        vec![],
    )
}

/// Full accounts view QA: sidebar, dashboard hero, and transaction register.
#[component]
pub fn AccountFullQa() -> impl IntoView {
    let selected_id: RwSignal<Option<String>> = RwSignal::new(Some("cb-smart-access".to_owned()));
    let (collapsed, _) = signal(false);
    let period = RwSignal::new(bc_ipc::Period::Monthly);
    let window_start = RwSignal::new(jiff::Zoned::now().date());

    view! {
        <div class=style::layout>
            <AccountSidebar
                nodes=sample_accounts()
                selected_id=selected_id.read_only().into()
                collapsed=collapsed
            />
            <div class=style::content>
                <AccountDashboard
                    node=smart_access_node()
                    stats=Signal::derive(|| None)
                    period_window=period.read_only().into()
                    window_start=window_start.read_only().into()
                />
                <TransactionRegister
                    transactions=Signal::derive(sample_transactions)
                    viewing_account_id="cb-smart-access"
                    period=period
                    window_start=window_start
                />
            </div>
        </div>
    }
}
