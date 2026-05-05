//! Top-level page components — one per route.

pub mod accounts;
pub mod budget;
pub mod dashboard;
pub mod plugins;
pub mod reports;
pub mod settings;

pub use accounts::Accounts;
pub use budget::Budget;
pub use dashboard::Dashboard;
pub use plugins::Plugins;
pub use reports::Reports;
pub use settings::Settings;
