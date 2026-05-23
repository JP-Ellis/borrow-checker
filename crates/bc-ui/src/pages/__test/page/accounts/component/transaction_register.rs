//! Route entry for `/__test/page/accounts/transaction-register`.

pub use crate::pages::accounts::components::transaction_register::qa::TransactionRegisterQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "TransactionRegister";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/transaction-register";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "Full register with filter bar, column headers, and keyboard navigation.";
