//! QA page for [`super::TransactionRow`].

use std::sync::LazyLock;

use bc_ipc::AuditEntry;
use bc_ipc::Money;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;

use super::TransactionRow;

/// Simple cleared transaction for QA display.
static TX_SIMPLE: LazyLock<Transaction> = LazyLock::new(|| {
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
    )
});

/// Multi-posting salary transaction for QA display.
static TX_MULTI_POSTING: LazyLock<Transaction> = LazyLock::new(|| {
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
        vec![AuditEntry::new(
            "09:04",
            "import",
            "from commbank-au.wasm@1.4.2",
        )],
    )
});

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
                    tx=&*TX_SIMPLE
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
                    tx=&*TX_SIMPLE
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
                    tx=&*TX_MULTI_POSTING
                    viewing_account_id="cb-smart-access"
                    selected=Signal::derive(|| true)
                    expanded=Signal::derive(|| true)
                    on_toggle=Callback::new(|()| {})
                />
            </section>

        </div>
    }
}
