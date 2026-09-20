//! Route entry for `/__test/component/balance-cell`.

pub use crate::components::balance_cell::qa::BalanceCellQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "BalanceCell";
/// Route path.
pub const PATH: &str = "/__test/component/balance-cell";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "Running balance over an in-cell bar: straddling, one-sided, degenerate, mixed.";
