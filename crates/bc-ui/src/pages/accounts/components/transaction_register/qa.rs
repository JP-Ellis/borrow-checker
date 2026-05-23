//! QA page for [`super::TransactionRegister`].

use bc_ipc::AuditEntry;
use bc_ipc::Money;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;

use super::TransactionRegister;

/// Returns sample transactions for the Smart Access account QA showcase.
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
                    "cb-smart-access",
                    "Assets :: Smart Access",
                    Money::new(-8_420, "AUD"),
                    None::<&str>,
                ),
                Posting::new(
                    "groceries",
                    "Expenses :: Groceries",
                    Money::new(8_420, "AUD"),
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
                    "income-salary",
                    "Income :: Salary",
                    Money::new(-846_154, "AUD"),
                    Some("gross pay"),
                ),
                Posting::new(
                    "cb-smart-access",
                    "Assets :: Smart Access",
                    Money::new(428_055, "AUD"),
                    Some("take-home"),
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
