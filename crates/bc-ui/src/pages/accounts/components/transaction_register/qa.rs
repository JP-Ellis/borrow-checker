//! QA page for [`super::TransactionRegister`].

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::FilteredTransaction;
use bc_ipc::Posting;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::TransactionRegister;

/// Returns sample transactions for the Smart Access account QA showcase.
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
    )
}

/// Returns the sample salary transaction.
fn salary_transaction() -> Transaction {
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
    )
}

/// Returns a three-posting split transaction with one leg deliberately left
/// out of `matched_postings`, to demonstrate the non-matching leg dimming.
fn partially_matched_transaction() -> FilteredTransaction {
    let tx = Transaction::new(
        "tx-dinner-split-2026-05-02",
        jiff::civil::Date::constant(2026, 5, 2),
        "Dinner — Chin Chin",
        "",
        None::<&str>,
        vec![],
        Reconciliation::Reconciled,
        vec!["shared".to_owned()],
        vec![
            Posting::new(
                "posting-dinner-debit",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                Some(Amount::new(Decimal::new(-12_000, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
            Posting::new(
                "posting-dinner-dining",
                AccountRef::new("dining", "Expenses :: Dining"),
                Some(Amount::new(Decimal::new(8_000, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
            Posting::new(
                "posting-dinner-shared",
                AccountRef::new("shared-owed", "Assets :: Owed by Roommate"),
                Some(Amount::new(Decimal::new(4_000, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
        ],
        vec![],
    );
    /* Only the debit and dining legs matched the active filter — the
    "owed by roommate" leg renders dimmed in the expanded detail. */
    let matched = vec![
        "posting-dinner-debit".to_owned(),
        "posting-dinner-dining".to_owned(),
    ];
    FilteredTransaction::new(tx, matched)
}

/// Renders a [`TransactionRegister`] whose sole row is a partial-match
/// transaction — expanding it demonstrates the non-matching leg rendering
/// dimmed in the detail editor.
#[component]
fn DimmedRegisterShowcase() -> impl IntoView {
    let period = RwSignal::new(bc_ipc::Period::Monthly);
    let window_start = RwSignal::new(jiff::Zoned::now().date());

    view! {
        <TransactionRegister
            transactions=Signal::derive(|| vec![partially_matched_transaction()])
            viewing_account_id="cb-smart-access"
            period=period
            window_start=window_start
        />
    }
}

/// Renders [`TransactionRegister`] with full and empty data sets.
#[component]
pub fn TransactionRegisterQa() -> impl IntoView {
    let period = RwSignal::new(bc_ipc::Period::Monthly);
    let window_start = RwSignal::new(jiff::Zoned::now().date());

    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "typical — Smart Access transactions (use j/k/Enter to navigate)"
                </p>
                <TransactionRegister
                    transactions=Signal::derive(sample_transactions)
                    viewing_account_id="cb-smart-access"
                    period=period
                    window_start=window_start
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "empty — no transactions"
                </p>
                <TransactionRegister
                    transactions=Signal::derive(Vec::new)
                    viewing_account_id="cb-smart-access"
                    period=period
                    window_start=window_start
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "dimmed — expand to see the non-matching leg dimmed (dinner split, one leg unmatched)"
                </p>
                <DimmedRegisterShowcase />
            </section>

        </div>
    }
}
