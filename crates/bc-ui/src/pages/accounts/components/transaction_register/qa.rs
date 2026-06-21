//! QA page for [`super::TransactionRegister`].

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::Posting;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::TransactionRegister;

/// Returns sample transactions for the Smart Access account QA showcase.
#[expect(
    clippy::expect_used,
    reason = "QA fixture — timestamp literals are valid"
)]
fn sample_transactions() -> Vec<Transaction> {
    vec![
        Transaction::new(
            "tx-coles-2026-04-30",
            jiff::civil::Date::constant(2026, 4, 30),
            "Coles Carlton",
            "",
            None::<&str>,
            vec![],
            Reconciliation::Reconciled,
            vec!["shared".to_owned()],
            vec![
                Posting::new(
                    "posting-coles-debit",
                    AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                    Some(Amount::new(Decimal::new(-8_420, 2), "AUD")),
                    None::<&str>,
                    vec![],
                    None,
                    None,
                ),
                Posting::new(
                    "posting-coles-groceries",
                    AccountRef::new("groceries", "Expenses :: Groceries"),
                    Some(Amount::new(Decimal::new(8_420, 2), "AUD")),
                    None::<&str>,
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
        ),
        Transaction::new(
            "tx-salary-2026-04-30",
            jiff::civil::Date::constant(2026, 4, 30),
            "Salary — Atlassian",
            "",
            None::<&str>,
            vec![],
            Reconciliation::Reconciled,
            vec!["work".to_owned()],
            vec![
                Posting::new(
                    "posting-salary-income",
                    AccountRef::new("income-salary", "Income :: Salary"),
                    Some(Amount::new(Decimal::new(-846_154, 2), "AUD")),
                    Some("gross pay"),
                    vec![],
                    None,
                    None,
                ),
                Posting::new(
                    "posting-salary-takehome",
                    AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                    Some(Amount::new(Decimal::new(428_055, 2), "AUD")),
                    Some("take-home"),
                    vec![],
                    None,
                    None,
                ),
            ],
            vec![],
        ),
    ]
}

/// Renders [`TransactionRegister`] with full and empty data sets.
#[component]
pub fn TransactionRegisterQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "typical — Smart Access transactions (use j/k/Enter to navigate)"
                </p>
                <TransactionRegister
                    transactions=Signal::derive(sample_transactions)
                    viewing_account_id="cb-smart-access"
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "empty — no transactions"
                </p>
                <TransactionRegister
                    transactions=Signal::derive(Vec::new)
                    viewing_account_id="cb-smart-access"
                />
            </section>

        </div>
    }
}
