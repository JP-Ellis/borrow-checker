//! QA page for [`super::TransactionRegister`].

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::FilteredTransaction;
use bc_ipc::Posting;
use bc_ipc::PostingAmount;
use bc_ipc::Reconciliation;
use bc_ipc::RegisterCursor;
use bc_ipc::RegisterRow;
use bc_ipc::Transaction;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::TransactionRegister;
use crate::pages::accounts::register_pages::BalanceMode;
use crate::pages::accounts::register_pages::LoadedRegister;

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

/// Returns a three-posting split transaction with one leg deliberately left
/// out of `matched_postings`, to demonstrate the non-matching leg dimming.
fn partially_matched_transaction() -> FilteredTransaction {
    let tx = Transaction::new(
        "tx-dinner-split-2026-05-02",
        jiff::civil::Date::constant(2026, 5, 2),
        "",
        vec![bc_ipc::MetaEntryDto::new(
            "payee",
            bc_ipc::MetaValueDto::Text("Generic Diner".to_owned()),
        )],
        Reconciliation::Reconciled,
        vec!["shared".to_owned()],
        vec![
            Posting::new(
                "posting-dinner-debit",
                AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                PostingAmount::Stored(Amount::new(Decimal::new(-12_000, 2), "AUD")),
                vec![],
                vec![],
                None,
                None,
            ),
            Posting::new(
                "posting-dinner-dining",
                AccountRef::new("dining", "Expenses :: Dining"),
                PostingAmount::Stored(Amount::new(Decimal::new(8_000, 2), "AUD")),
                vec![],
                vec![],
                None,
                None,
            ),
            Posting::new(
                "posting-dinner-shared",
                AccountRef::new("shared-owed", "Assets :: Owed by Roommate"),
                PostingAmount::Stored(Amount::new(Decimal::new(4_000, 2), "AUD")),
                vec![],
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

/// Builds a [`LoadedRegister`] from sample [`FilteredTransaction`]s, all with
/// the same fake AUD balance and no further pages.
fn loaded_register(transactions: Vec<FilteredTransaction>) -> LoadedRegister {
    let rows: Vec<RegisterRow> = transactions
        .into_iter()
        .map(|ft| {
            RegisterRow::new(
                ft.transaction,
                ft.matched_postings,
                Some(Amount::new(Decimal::new(12_345, 2), "AUD")),
                None,
            )
        })
        .collect();
    let total = u32::try_from(rows.len()).unwrap_or(u32::MAX);
    LoadedRegister {
        rows,
        total,
        next_cursor: None,
        loading: false,
        failed: false,
        generation: 0,
    }
}

/// Renders a [`TransactionRegister`] whose sole row is a partial-match
/// transaction — expanding it demonstrates the non-matching leg rendering
/// dimmed in the detail editor.
#[component]
fn DimmedRegisterShowcase() -> impl IntoView {
    let window = RwSignal::new(crate::components::period_nav::DisplayWindow::AllTime);
    let loaded = loaded_register(vec![partially_matched_transaction()]);

    view! {
        <TransactionRegister
            register=Signal::derive(move || loaded.clone())
            on_load_more=Callback::new(|_| {})
            balance_mode=RwSignal::new(BalanceMode::Real)
            viewing_account_id="cb-smart-access"
            window=window
        />
    }
}

/// Renders [`TransactionRegister`] with full and empty data sets, and one with
/// more pages remaining to show the load-more button.
#[component]
pub fn TransactionRegisterQa() -> impl IntoView {
    let window = RwSignal::new(crate::components::period_nav::DisplayWindow::AllTime);
    let typical = loaded_register(sample_transactions());
    let empty = loaded_register(Vec::new());
    let mut paged = loaded_register(sample_transactions());
    paged.total = 250;
    paged.next_cursor = Some(RegisterCursor::new(
        jiff::civil::Date::constant(2026, 1, 1),
        "x".to_owned(),
    ));

    view! {
        <div style="display:flex;flex-direction:column;gap:32px;padding:24px">

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "typical — Smart Access transactions (use j/k/Enter to navigate)"
                </p>
                <TransactionRegister
                    register=Signal::derive(move || typical.clone())
                    on_load_more=Callback::new(|_| {})
                    balance_mode=RwSignal::new(BalanceMode::Real)
                    viewing_account_id="cb-smart-access"
                    window=window
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "empty — no transactions"
                </p>
                <TransactionRegister
                    register=Signal::derive(move || empty.clone())
                    on_load_more=Callback::new(|_| {})
                    balance_mode=RwSignal::new(BalanceMode::Real)
                    viewing_account_id="cb-smart-access"
                    window=window
                />
            </section>

            <section>
                <p style="font-size:11px;color:var(--bc-ink-mute);margin-bottom:8px;">
                    "paged — more rows remain, showing the load-more button"
                </p>
                <TransactionRegister
                    register=Signal::derive(move || paged.clone())
                    on_load_more=Callback::new(|_| {})
                    balance_mode=RwSignal::new(BalanceMode::Real)
                    viewing_account_id="cb-smart-access"
                    window=window
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
