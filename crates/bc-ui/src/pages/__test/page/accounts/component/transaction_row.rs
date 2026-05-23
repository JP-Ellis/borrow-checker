//! Route entry for `/__test/page/accounts/transaction-row`.

pub use crate::pages::accounts::components::transaction_row::qa::TransactionRowQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "TransactionRow";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/transaction-row";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Register row: collapsed, selected, and expanded states.";
