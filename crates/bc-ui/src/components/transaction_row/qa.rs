//! QA showcase for [`super::TransactionRow`] — extended with the detail view in Task 4.

use bc_ipc::AccountRef;
use bc_ipc::Amount;
use bc_ipc::Posting;
use bc_ipc::Reconciliation;
use bc_ipc::Transaction;
use leptos::prelude::*;
use rust_decimal::Decimal;

use super::RowPerspective;
use super::TransactionRow;

/// Returns a simple two-leg transaction for QA use.
fn sample_tx(payee: &str, reconciliation: Reconciliation) -> Transaction {
    Transaction::new(
        "tx-qa-1",
        jiff::civil::Date::constant(2026, 6, 1),
        payee,
        "POS purchase",
        None::<&str>,
        vec![],
        reconciliation,
        vec!["groceries".to_owned()],
        vec![
            Posting::new(
                "p-1",
                AccountRef::new("checking", "Assets :: Checking"),
                Some(Amount::new(Decimal::new(-4_200, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
            Posting::new(
                "p-2",
                AccountRef::new("groceries", "Expenses :: Groceries"),
                Some(Amount::new(Decimal::new(4_200, 2), "AUD")),
                None::<&str>,
                vec![],
                None,
                None,
            ),
        ],
        vec![],
    )
}

/// Renders [`TransactionRow`] in representative states for visual inspection.
///
/// Covers: payee present, description-only (dim), unreconciled glyph, flagged glyph,
/// and global-perspective fallback. The expanded detail view is added in Task 4.
#[component]
pub fn TransactionRowQa() -> impl IntoView {
    view! {
        <div>
            <TransactionRow
                tx=sample_tx("Coles Carlton", Reconciliation::Unreconciled)
                perspective=RowPerspective::Account {
                    account_id: "checking".to_owned(),
                }
            />
            <TransactionRow
                tx=sample_tx("", Reconciliation::Reconciled)
                perspective=RowPerspective::Global
            />
            <TransactionRow
                tx=sample_tx("Flagged Merchant", Reconciliation::Flagged)
                perspective=RowPerspective::Account {
                    account_id: "checking".to_owned(),
                }
            />
            <TransactionRow
                tx=sample_tx("Budget Expense", Reconciliation::Unreconciled)
                perspective=RowPerspective::Budget {
                    account_id: "groceries".to_owned(),
                    tag_filter: None,
                    window_start: jiff::civil::Date::constant(2026, 6, 1),
                    window_end: jiff::civil::Date::constant(2026, 6, 30),
                }
            />
        </div>
    }
}
