//! Route entry for `/__test/component/status-pill`.

pub use crate::components::status_pill::qa::StatusPillQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "StatusPill";
/// Route path.
pub const PATH: &str = "/__test/component/status-pill";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Semantic status badge: Good, Warn, Bad tones.";
