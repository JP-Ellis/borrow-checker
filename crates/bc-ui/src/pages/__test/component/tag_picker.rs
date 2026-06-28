//! Route entry for `/__test/component/tag-picker`.

pub use crate::components::tag_picker::qa::TagPickerQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "TagPicker";
/// Route path.
pub const PATH: &str = "/__test/component/tag-picker";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Multi-select tag input with autocomplete and inline creation.";
