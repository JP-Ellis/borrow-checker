//! Route entry for `/__test/component/new-transaction`.

pub use crate::components::new_transaction::qa::NewTransactionQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "NewTransaction";
/// Route path.
pub const PATH: &str = "/__test/component/new-transaction";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Global new-transaction modal, unseeded and seeded with an account.";
