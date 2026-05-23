//! Route entry for `/__test/page/accounts/hero`.

pub use crate::pages::accounts::dashboard::qa::AccountDashboardQa;

/// Display name shown in the QA index.
pub const TITLE: &str = "AccountDashboard (hero)";
/// Route path.
pub const PATH: &str = "/__test/page/accounts/hero";
/// One-line description for the index card.
pub const DESCRIPTION: &str =
    "Per-account dashboard hero: breadcrumb, balance, stat tiles, sparkline.";
