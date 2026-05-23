//! Route entry for `/__test/component/num`.

pub use crate::components::num::qa::NumQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "Num";
/// Route path.
pub const PATH: &str = "/__test/component/num";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Monetary amount display: positive, negative, zero, edge values.";
