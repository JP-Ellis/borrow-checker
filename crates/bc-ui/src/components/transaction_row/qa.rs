//! QA showcase for [`super::TransactionRow`] — collapsed row plus the hybrid
//! expanded detail view across all perspectives and states.

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::AuditEntry;
use bc_ipc::Posting;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::RowPerspective;
use super::TransactionRow;

/// Builds a posting with a concrete amount in AUD minor units.
fn leg(id: &str, acct_id: &str, acct_name: &str, minor: i64) -> Posting {
    Posting::new(
        id,
        AccountRef::new(acct_id, acct_name),
        Some(Amount::new(Decimal::new(minor, 2), "AUD")),
        None::<&str>,
        vec![],
        None,
        None,
    )
}

/// Builds an elided posting (no concrete amount → renders an `auto` token).
fn elided(id: &str, acct_id: &str, acct_name: &str) -> Posting {
    Posting::new(
        id,
        AccountRef::new(acct_id, acct_name),
        None,
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

/// A transaction with one concrete leg and one elided leg (shows `auto`).
fn elided_tx() -> Transaction {
    tx(
        "tx-elided",
        "Salary",
        "Monthly pay",
        Reconciliation::Reconciled,
        vec!["income".to_owned()],
        vec![
            leg("p-1", "checking", "Assets :: Checking", 500_000),
            elided("p-2", "salary", "Income :: Salary"),
        ],
    )
}

/// A spread posting whose window covers roughly half its range (Budget).
fn spread_tx() -> Transaction {
    let mut spread = leg("p-1", "insurance", "Expenses :: Insurance", 30_000);
    spread.spread_from = Some(jiff::civil::Date::constant(2026, 6, 1));
    spread.spread_until = Some(jiff::civil::Date::constant(2026, 6, 30));
    spread.note = Some("annual premium spread monthly".to_owned());
    tx(
        "tx-spread",
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

/// Renders [`TransactionRow`] across perspectives and states for inspection.
///
/// Covers the Account perspective (balanced, split, unbalanced, single-elided),
/// the Budget perspective (prorated spread headline), the Global perspective,
/// flagged and unreconciled glyphs, and a payee/description-less em-dash row.
#[component]
pub fn TransactionRowQa() -> impl IntoView {
    let account = |id: &str| RowPerspective::Account {
        account_id: id.to_owned(),
    };
    view! {
        <div>
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
                tx=spread_tx()
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
        </div>
    }
}
