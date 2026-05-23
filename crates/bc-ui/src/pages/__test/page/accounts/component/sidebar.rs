//! Route entry for `/__test/page/accounts/sidebar`.

pub use crate::pages::accounts::components::sidebar::qa::AccountSidebarQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "AccountSidebar";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/sidebar";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Account tree sidebar: expanded tree and collapsed dot-rail.";
