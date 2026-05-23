//! Route entry for `/__test/page/accounts/sticky-bar`.

pub use crate::pages::accounts::components::sticky_bar::qa::StickyAccountBarQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "StickyAccountBar";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/sticky-bar";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Compact sticky header: hidden state and visible state.";
