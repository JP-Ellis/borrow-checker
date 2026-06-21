//! QA page for [`super::TransactionRow`].

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use bc_ipc::TxStatus;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::TransactionRow;

/// Returns a simple cleared transaction for QA display.
#[expect(
    clippy::expect_used,
    reason = "QA fixture — timestamp literals are valid"
)]
fn tx_simple() -> Transaction {
    Transaction::new(
        "tx-coles-qa",
        jiff::civil::Date::constant(2026, 4, 30),
        "Coles Carlton",
        TxStatus::Cleared,
        vec!["shared".to_owned()],
        vec![
            Posting::new(
                "posting-coles-debit",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                Amount::new(Decimal::new(-8_420, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-coles-groceries",
                AccountRef::new("groceries", "Expenses :: Groceries"),
                Amount::new(Decimal::new(8_420, 2), "AUD"),
                None::<&str>,
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

/// Returns a multi-posting salary transaction for QA display.
#[expect(
    clippy::expect_used,
    reason = "QA fixture — timestamp literals are valid"
)]
fn tx_multi_posting() -> Transaction {
    Transaction::new(
        "tx-salary-qa",
        jiff::civil::Date::constant(2026, 4, 30),
        "Salary — Atlassian Pty Ltd",
        TxStatus::Cleared,
        vec!["work".to_owned()],
        vec![
            Posting::new(
                "posting-salary-income",
                AccountRef::new("income-salary", "Income :: Salary"),
                Amount::new(Decimal::new(-846_154, 2), "AUD"),
                Some("gross pay"),
                None,
                None,
            ),
            Posting::new(
                "posting-salary-takehome",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                Amount::new(Decimal::new(428_055, 2), "AUD"),
                Some("take-home"),
                None,
                None,
            ),
        ],
        vec![AuditEntry::new(
            "2026-04-30T09:04:00Z"
                .parse::<jiff::Timestamp>()
                .expect("valid timestamp"),
            "import",
            "from commbank-au.wasm@1.4.2",
        )],
    )
}

/// Returns a multi-split same-type transaction for QA display.
fn tx_split_siblings() -> Transaction {
    Transaction::new(
        "tx-amazon-qa",
        jiff::civil::Date::constant(2026, 6, 13),
        "Amazon",
        TxStatus::Pending,
        vec![],
        vec![
            Posting::new(
                "posting-amazon-debit",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                Amount::new(Decimal::new(-30_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-amazon-groceries",
                AccountRef::new("exp-groceries", "Expenses :: Groceries"),
                Amount::new(Decimal::new(10_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-amazon-healthcare",
                AccountRef::new("exp-healthcare", "Expenses :: Healthcare"),
                Amount::new(Decimal::new(10_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-amazon-household",
                AccountRef::new("exp-household", "Expenses :: Household"),
                Amount::new(Decimal::new(10_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
        ],
        vec![],
    )
}

/// Returns a multi-split cross-type transaction for QA display.
fn tx_split_cross_type() -> Transaction {
    Transaction::new(
        "tx-cross-qa",
        jiff::civil::Date::constant(2026, 6, 13),
        "Cross-Type Merchant",
        TxStatus::Pending,
        vec![],
        vec![
            Posting::new(
                "posting-cross-debit",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                Amount::new(Decimal::new(-30_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-cross-groceries",
                AccountRef::new("exp-groceries", "Expenses :: Groceries"),
                Amount::new(Decimal::new(10_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-cross-healthcare",
                AccountRef::new("exp-healthcare", "Expenses :: Healthcare"),
                Amount::new(Decimal::new(10_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
            Posting::new(
                "posting-cross-interest",
                AccountRef::new("inc-interest", "Income :: Interest"),
                Amount::new(Decimal::new(10_000, 2), "AUD"),
                None::<&str>,
                None,
                None,
            ),
        ],
        vec![],
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

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "multi-split, same type → shell expansion"
                </p>
                <TransactionRow
                    tx=tx_split_siblings()
                    viewing_account_id="cb-smart-access"
                    selected=Signal::derive(|| false)
                    expanded=Signal::derive(|| false)
                    on_toggle=Callback::new(|()| {})
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "multi-split, cross-type → split transaction placeholder"
                </p>
                <TransactionRow
                    tx=tx_split_cross_type()
                    viewing_account_id="cb-smart-access"
                    selected=Signal::derive(|| false)
                    expanded=Signal::derive(|| false)
                    on_toggle=Callback::new(|()| {})
                />
            </section>

        </div>
    }
}
