//! Route entry for `/__test/component/stat-card`.

pub use crate::components::stat_card::qa::StatCardQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "StatCard";
/// Route path.
pub const PATH: &str = "/__test/component/stat-card";
/// One-line description for the index card.
pub const DESCRIPTION: &str = "KPI tile: label, value, optional sub-line, tone variants.";
