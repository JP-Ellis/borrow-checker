//! Route entry for `/__test/page/budget/new-budget`.

pub use crate::pages::budget::components::new_budget::qa::NewBudgetQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "NewBudget";
/// Route path.
pub const PATH: &str = "/__test/page/budget/new-budget";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Budget creation form with account picker.";
