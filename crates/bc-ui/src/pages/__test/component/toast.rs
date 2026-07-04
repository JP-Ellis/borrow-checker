//! Route entry for `/__test/component/toast`.

pub use crate::components::toast::qa::ToastQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "Toast";
/// Route path.
pub const PATH: &str = "/__test/component/toast";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Top-layer transient notifications with optional actions.";
