//! Route entry for `/__test/component/sparkline`.

pub use crate::components::sparkline::qa::SparklineQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "Sparkline";
/// Route path.
pub const PATH: &str = "/__test/component/sparkline";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "SVG cash-flow sparkline: income vs expenses over time.";
