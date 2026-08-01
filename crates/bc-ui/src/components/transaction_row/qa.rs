//! QA showcase for [`super::TransactionRow`] — collapsed row plus the hybrid
//! expanded detail view across all perspectives and states.

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::Posting;
use bc_ipc::Reconciliation;
use bc_ipc::TagInfo;
use bc_ipc::Transaction;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::RowPerspective;
use super::TransactionRow;
use super::posting_row::PostingsList;
use crate::components::transaction_row::edit_ctx::TxEditCtx;
use crate::components::transaction_row::editable::EditablePosting;
use crate::components::transaction_row::editable::EditableTransaction;

/* ── helpers ─────────────────────────────────────────────────────────────── */

/// Builds a posting with a concrete amount in AUD minor units.
fn leg(id: &str, acct_id: &str, acct_name: &str, minor: i64) -> Posting {
    Posting::new(
        id,
        AccountRef::new(acct_id, acct_name),
        bc_ipc::PostingAmount::Stored(Amount::new(Decimal::new(minor, 2), "AUD")),
        None::<&str>,
        vec![],
        None,
        None,
    )
}

/// Builds an elided posting whose amount is derived to a single-commodity
/// residual of `residual_minor` cents (renders as ghost/inferred).
fn elided(id: &str, acct_id: &str, acct_name: &str, residual_minor: i64) -> Posting {
    Posting::new(
        id,
        AccountRef::new(acct_id, acct_name),
        bc_ipc::PostingAmount::Derived(vec![Amount::new(Decimal::new(residual_minor, 2), "AUD")]),
        None::<&str>,
        vec![],
        None,
        None,
    )
}

/// Assembles a transaction with sensible QA defaults.
fn tx(
    id: &str,
    payee: &str,
    description: &str,
    reconciliation: Reconciliation,
    tags: Vec<String>,
    postings: Vec<Posting>,
) -> Transaction {
    Transaction::new(
        id,
        jiff::civil::Date::constant(2026, 6, 1),
        payee,
        description,
        None::<&str>,
        vec![],
        reconciliation,
        tags,
        postings,
        vec![
            AuditEntry::new(
                jiff::Timestamp::UNIX_EPOCH,
                "import",
                "imported from statement.csv",
            ),
            AuditEntry::new(
                jiff::Timestamp::UNIX_EPOCH,
                "autocat",
                "auto-categorised by rule",
            ),
        ],
    )
}

/// Shared account list for QA — exercised by the `AccountPicker` in edit mode.
fn qa_accounts() -> Vec<AccountRef> {
    vec![
        AccountRef::new("checking", "Assets :: Checking"),
        AccountRef::new("groceries", "Expenses :: Groceries"),
        AccountRef::new("household", "Expenses :: Household"),
        AccountRef::new("misc", "Expenses :: Misc"),
        AccountRef::new("salary", "Income :: Salary"),
        AccountRef::new("insurance", "Expenses :: Insurance"),
    ]
}

/// Shared tag list for QA — exercised by the `TagPicker` in edit mode.
fn qa_tags() -> Vec<TagInfo> {
    vec![
        TagInfo::new("tag-groceries", "groceries"),
        TagInfo::new("tag-shopping", "shopping"),
        TagInfo::new("tag-income", "income"),
        TagInfo::new("tag-insurance", "insurance"),
        TagInfo::new("tag-person-josh", "person:josh"),
    ]
}

/* ── transactions ─────────────────────────────────────────────────────────── */

/// A balanced two-posting transaction (Account perspective).
fn balanced_tx() -> Transaction {
    tx(
        "tx-balanced",
        "Coles Carlton",
        "POS purchase",
        Reconciliation::Reconciled,
        vec!["groceries".to_owned()],
        vec![
            leg("p-1", "checking", "Assets :: Checking", -4_200),
            leg("p-2", "groceries", "Expenses :: Groceries", 4_200),
        ],
    )
}

/// A multi-posting split transaction.
fn split_tx() -> Transaction {
    tx(
        "tx-split",
        "Costco",
        "Mixed basket",
        Reconciliation::Reconciled,
        vec!["shopping".to_owned()],
        vec![
            leg("p-1", "checking", "Assets :: Checking", -12_000),
            leg("p-2", "groceries", "Expenses :: Groceries", 7_000),
            leg("p-3", "household", "Expenses :: Household", 5_000),
        ],
    )
}

/// A one-sided, unbalanced import (single concrete leg).
fn unbalanced_tx() -> Transaction {
    tx(
        "tx-unbalanced",
        "Unknown Merchant",
        "Pending import",
        Reconciliation::Unreconciled,
        vec![],
        vec![leg("p-1", "checking", "Assets :: Checking", -5_000)],
    )
}

/// A transaction with one concrete leg and one elided leg (ghost/inferred amount).
fn elided_tx() -> Transaction {
    tx(
        "tx-elided",
        "Salary",
        "Monthly pay",
        Reconciliation::Reconciled,
        vec!["income".to_owned()],
        vec![
            leg("p-1", "checking", "Assets :: Checking", 500_000),
            elided("p-2", "salary", "Income :: Salary", -500_000),
        ],
    )
}

/// A spread posting whose `spread_from` equals the transaction date.
///
/// The spread chip renders the arrow-only form `⤳ <until>` (same-start case).
fn spread_same_tx() -> Transaction {
    let mut spread = leg("p-1", "insurance", "Expenses :: Insurance", 30_000);
    spread.spread_from = Some(jiff::civil::Date::constant(2026, 6, 1));
    spread.spread_until = Some(jiff::civil::Date::constant(2026, 6, 30));
    spread.note = Some("annual premium spread monthly".to_owned());
    tx(
        "tx-spread-same",
        "ACME Insurance",
        "Annual premium",
        Reconciliation::Reconciled,
        vec!["insurance".to_owned()],
        vec![
            spread,
            leg("p-2", "checking", "Assets :: Checking", -30_000),
        ],
    )
}

/// A spread posting whose `spread_from` differs from the transaction date.
///
/// The spread chip renders both endpoints `<from> ⤳ <until>` (diff-start case).
fn spread_diff_tx() -> Transaction {
    let mut spread = leg("p-1", "insurance", "Expenses :: Insurance", 30_000);
    spread.spread_from = Some(jiff::civil::Date::constant(2026, 7, 1));
    spread.spread_until = Some(jiff::civil::Date::constant(2026, 12, 31));
    spread.note = Some("deferred accrual from next month".to_owned());
    tx(
        "tx-spread-diff",
        "ACME Insurance",
        "Deferred premium",
        Reconciliation::Unreconciled,
        vec!["insurance".to_owned()],
        vec![
            spread,
            leg("p-2", "checking", "Assets :: Checking", -30_000),
        ],
    )
}

/// A transaction where the first posting has a plain note (no spread).
fn note_tx() -> Transaction {
    let mut noted = leg("p-1", "groceries", "Expenses :: Groceries", 4_200);
    noted.note = Some("organic produce only".to_owned());
    tx(
        "tx-note",
        "Harris Farm",
        "Weekly groceries",
        Reconciliation::Unreconciled,
        vec!["groceries".to_owned()],
        vec![leg("p-2", "checking", "Assets :: Checking", -4_200), noted],
    )
}

/// A flagged transaction (warning glyph).
fn flagged_tx() -> Transaction {
    tx(
        "tx-flagged",
        "Flagged Merchant",
        "Needs review",
        Reconciliation::Flagged,
        vec![],
        vec![
            leg("p-1", "checking", "Assets :: Checking", -8_900),
            leg("p-2", "misc", "Expenses :: Misc", 8_900),
        ],
    )
}

/// A payee-less, description-less transaction (em-dash fallback).
fn nameless_tx() -> Transaction {
    tx(
        "tx-nameless",
        "",
        "",
        Reconciliation::Unreconciled,
        vec![],
        vec![
            leg("p-1", "checking", "Assets :: Checking", -1_500),
            leg("p-2", "misc", "Expenses :: Misc", 1_500),
        ],
    )
}

/* ── showcase components ─────────────────────────────────────────────────── */

/// Renders the editable [`PostingsList`] against a seeded working buffer.
#[component]
pub fn PostingsListEditQa() -> impl IntoView {
    let seed = EditableTransaction {
        id: "tx-qa".to_owned(),
        date: "2026-06-01".to_owned(),
        payee: "QA Merchant".to_owned(),
        description: "Editor showcase".to_owned(),
        note: String::new(),
        reconciliation: Reconciliation::Unreconciled,
        tags: vec![],
        extra_dates: vec![],
        postings: vec![
            EditablePosting {
                id: Some("p-1".to_owned()),
                uid: 0,
                account_id: "checking".to_owned(),
                account_name: "Assets :: Checking".to_owned(),
                amount: "-42.00".to_owned(),
                currency: "AUD".to_owned(),
                note: String::new(),
                tags: vec![],
                spread_from: None,
                spread_until: None,
            },
            EditablePosting {
                id: Some("p-2".to_owned()),
                uid: 1,
                account_id: "groceries".to_owned(),
                account_name: "Expenses :: Groceries".to_owned(),
                amount: "42.00".to_owned(),
                currency: "AUD".to_owned(),
                note: String::new(),
                tags: vec![],
                spread_from: None,
                spread_until: None,
            },
        ],
    };
    let ctx = TxEditCtx::new(seed, qa_accounts(), None);
    ctx.all_tags.set(qa_tags());
    provide_context(ctx.clone());
    view! {
        <div>
            <h3>"PostingsList — editable, two-posting seed"</h3>
            <PostingsList />
        </div>
    }
}

/// Renders expanded [`TransactionRow`] instances across all detail-view states.
///
/// The expanded detail follows the always-editable "M" layout:
///
/// 1. **PostingLine grid** — one row per posting; each has an account picker,
///    inline spread chip/editor, and a seamless amount field (ghost/italic for
///    the inferred leg).
/// 2. **Quiet balance line** — a single summary line below the postings
///    (`balances`, `balances — auto …`, or `unbalanced — Σ = …`).
/// 3. **Metamix bar** — date input, editable status pill cycling through
///    `Unreconciled / Flagged / Reconciled`, compact tag picker, and note field.
/// 4. **Savebar** — appears only when the working buffer is dirty.
///
/// Cases:
/// - Balanced 2-leg (Reconciled) — accounts + tags lists populated
/// - Unbalanced single-leg (Unreconciled) — balance line shows mismatch
/// - Spread same-start — chip renders `⤳ <until>` (spread_from == tx date)
/// - Spread diff-start — chip renders `<from> ⤳ <until>` (spread_from ≠ tx date)
/// - Note posting — inline note field in the posting extras row
/// - >2-leg split (Reconciled) — three posting rows
/// - Single elided leg (Reconciled) — ghost/inferred amount in the amount field
/// - Flagged — status pill renders in the warning (Flagged) state
#[component]
pub fn ExpandedDetailQa() -> impl IntoView {
    let account = |id: &str| RowPerspective::Account {
        account_id: id.to_owned(),
    };

    /* Pre-expanded signals (static true — not toggleable in QA). */
    let exp = || Signal::from(RwSignal::new(true));

    view! {
        <div>
            <h3>"Expanded — balanced 2-leg, Reconciled (accounts + tags populated)"</h3>
            <TransactionRow
                tx=balanced_tx()
                perspective=account("checking")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — unbalanced single-leg, Unreconciled (balance mismatch)"</h3>
            <TransactionRow
                tx=unbalanced_tx()
                perspective=account("checking")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — spread same-start (⤳ until chip)"</h3>
            <TransactionRow
                tx=spread_same_tx()
                perspective=account("insurance")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — spread diff-start (from ⤳ until chip)"</h3>
            <TransactionRow
                tx=spread_diff_tx()
                perspective=account("insurance")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — note posting (inline note field, no spread)"</h3>
            <TransactionRow
                tx=note_tx()
                perspective=account("checking")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — >2-leg split, Reconciled (three posting rows)"</h3>
            <TransactionRow
                tx=split_tx()
                perspective=account("checking")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — single elided leg, Reconciled (ghost inferred amount)"</h3>
            <TransactionRow
                tx=elided_tx()
                perspective=account("checking")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />

            <h3>"Expanded — Flagged (status pill in warning state)"</h3>
            <TransactionRow
                tx=flagged_tx()
                perspective=account("checking")
                expanded=exp()
                accounts=qa_accounts()
                all_tags=qa_tags()
            />
        </div>
    }
}

/// Renders [`TransactionRow`] across perspectives and states for inspection.
///
/// Covers the Account perspective (balanced, split, unbalanced, single-elided),
/// the Budget perspective (prorated spread headline), the Global perspective,
/// flagged and unreconciled glyphs, and a payee/description-less em-dash row.
/// Also renders all expanded-detail states via [`ExpandedDetailQa`].
#[component]
pub fn TransactionRowQa() -> impl IntoView {
    let account = |id: &str| RowPerspective::Account {
        account_id: id.to_owned(),
    };
    view! {
        <div>
            <PostingsListEditQa />
            <h3>"Account — balanced 2-posting"</h3>
            <TransactionRow tx=balanced_tx() perspective=account("checking") />

            <h3>"Account — multi-posting split"</h3>
            <TransactionRow tx=split_tx() perspective=account("checking") />

            <h3>"Account — one-sided unbalanced"</h3>
            <TransactionRow tx=unbalanced_tx() perspective=account("checking") />

            <h3>"Account — single elided leg (auto)"</h3>
            <TransactionRow tx=elided_tx() perspective=account("checking") />

            <h3>"Budget — prorated spread (half window)"</h3>
            <TransactionRow
                tx=spread_same_tx()
                perspective=RowPerspective::Budget {
                    account_id: "insurance".to_owned(),
                    tag_filter: None,
                    window_start: jiff::civil::Date::constant(2026, 6, 1),
                    window_end: jiff::civil::Date::constant(2026, 6, 15),
                }
            />

            <h3>"Global perspective"</h3>
            <TransactionRow tx=balanced_tx() perspective=RowPerspective::Global />

            <h3>"Flagged"</h3>
            <TransactionRow tx=flagged_tx() perspective=account("checking") />

            <h3>"Unreconciled"</h3>
            <TransactionRow tx=unbalanced_tx() perspective=account("checking") />

            <h3>"Payee-less + description-less (em-dash)"</h3>
            <TransactionRow tx=nameless_tx() perspective=account("checking") />
            <ExpandedDetailQa />
        </div>
    }
}
