//! Route entry for `/__test/component/transaction-row`.

pub use crate::components::transaction_row::qa::TransactionRowQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "TransactionRow";
/// Route path.
pub const PATH: &str = "/__test/component/transaction-row";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Shared collapsed transaction row with perspective-aware amount.";
