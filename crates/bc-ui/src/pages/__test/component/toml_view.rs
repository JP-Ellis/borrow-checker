//! Route entry for `/__test/component/toml-view`.

pub use crate::components::toml_view::qa::TomlViewQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "TomlView";
/// Route path.
pub const PATH: &str = "/__test/component/toml-view";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "TOML-like read-only primitives: sections, key-values, postings, audit entries.";
