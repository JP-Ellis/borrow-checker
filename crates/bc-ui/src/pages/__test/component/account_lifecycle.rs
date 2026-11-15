//! Route entry for `/__test/component/account-lifecycle`.

pub use crate::components::account_lifecycle::qa::AccountLifecycleQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "AccountLifecycle";
/// Route path.
pub const PATH: &str = "/__test/component/account-lifecycle";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "Close and archive previews: cascade, balances, late postings, nudge.";
