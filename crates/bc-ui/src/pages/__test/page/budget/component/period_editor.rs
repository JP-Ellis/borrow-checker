//! Route entry for `/__test/page/budget/period-editor`.

pub use crate::pages::budget::components::period_editor::qa::PeriodEditorQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "PeriodEditor";
/// Route path.
pub const PATH: &str = "/__test/page/budget/period-editor";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Period kind dropdown with per-kind fields and inline errors.";
