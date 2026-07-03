//! Route entry for `/__test/component/period-nav`.

pub use crate::components::period_nav::qa::PeriodNavQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "PeriodNav";
/// Route path.
pub const PATH: &str = "/__test/component/period-nav";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "Shared period stepper: prev/next window nav + granularity select.";
