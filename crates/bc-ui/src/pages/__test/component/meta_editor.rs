//! Route entry for `/__test/component/meta-editor`.

pub use crate::components::meta_editor::qa::MetaEditorQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "MetaEditor";
/// Route path.
pub const PATH: &str = "/__test/component/meta-editor";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Typed key-value metadata rows: all seven value types, plus \
                               mismatched, tombstoned, unknown-account, untyped and keyless \
                               states.";
