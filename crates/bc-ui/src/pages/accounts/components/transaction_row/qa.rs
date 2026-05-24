//! QA page for [`super::TransactionRow`].

use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;

use super::TransactionRow;

/// Returns a simple cleared transaction for QA display.
fn tx_simple() -> Transaction {
    Transaction::new(
        "tx-coles-qa",
        "2026-04-30",
        "Coles Carlton",
        TxStatus::Cleared,
        vec!["shared".to_owned()],
        vec![
            Posting::new(
                "cb-smart-access",
                "Assets :: Smart Access",
                Amount::new(-8_420, "AUD", 2),
                None::<&str>,
            ),
            Posting::new(
                "groceries",
                "Expenses :: Groceries",
                Amount::new(8_420, "AUD", 2),
                None::<&str>,
            ),
        ],
        vec![AuditEntry::new(
            "14:21",
            "import",
            "from commbank-au.wasm@1.4.2",
        )],
    )
}

/// Returns a multi-posting salary transaction for QA display.
fn tx_multi_posting() -> Transaction {
    Transaction::new(
        "tx-salary-qa",
        "2026-04-30",
        "Salary — Atlassian Pty Ltd",
        TxStatus::Cleared,
        vec!["work".to_owned()],
        vec![
            Posting::new(
                "income-salary",
                "Income :: Salary",
                Amount::new(-846_154, "AUD", 2),
                Some("gross pay"),
            ),
            Posting::new(
                "cb-smart-access",
                "Assets :: Smart Access",
                Amount::new(428_055, "AUD", 2),
                Some("take-home"),
            ),
        ],
        vec![AuditEntry::new(
            "09:04",
            "import",
            "from commbank-au.wasm@1.4.2",
        )],
    )
}

/// Renders [`TransactionRow`] in collapsed, selected, and expanded states.
#[component]
pub fn TransactionRowQa() -> impl IntoView {
    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "collapsed, unselected"
                </p>
                <TransactionRow
                    tx=tx_simple()
                    viewing_account_id="cb-smart-access"
                    selected=Signal::derive(|| false)
                    expanded=Signal::derive(|| false)
                    on_toggle=Callback::new(|()| {})
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "collapsed, selected (keyboard focus)"
                </p>
                <TransactionRow
                    tx=tx_simple()
                    viewing_account_id="cb-smart-access"
                    selected=Signal::derive(|| true)
                    expanded=Signal::derive(|| false)
                    on_toggle=Callback::new(|()| {})
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "expanded with detail panel (multi-posting salary)"
                </p>
                <TransactionRow
                    tx=tx_multi_posting()
                    viewing_account_id="cb-smart-access"
                    selected=Signal::derive(|| true)
                    expanded=Signal::derive(|| true)
                    on_toggle=Callback::new(|()| {})
                />
            </section>

        </div>
    }
}
