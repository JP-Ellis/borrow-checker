//! Route entry for `/__test/page/accounts/add-transaction`.

pub use crate::pages::accounts::components::add_transaction::qa::AddTransactionFormQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "AddTransactionForm";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/add-transaction";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "Inline transaction creation form: default, IPC error, and validation states.";
