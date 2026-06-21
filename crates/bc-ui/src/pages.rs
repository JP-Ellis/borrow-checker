//! Top-level page components — one per route.

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod __test;
pub mod accounts;
pub mod budget;
#[cfg(target_arch = "wasm32")]
pub mod dashboard;
#[cfg(target_arch = "wasm32")]
pub mod plugins;
#[cfg(target_arch = "wasm32")]
pub mod reports;
#[cfg(target_arch = "wasm32")]
pub mod settings;

#[cfg(target_arch = "wasm32")]
pub use accounts::Accounts;
#[cfg(target_arch = "wasm32")]
pub use budget::Budget;
#[cfg(target_arch = "wasm32")]
pub use dashboard::Dashboard;
#[cfg(target_arch = "wasm32")]
pub use plugins::Plugins;
#[cfg(target_arch = "wasm32")]
pub use reports::Reports;
#[cfg(target_arch = "wasm32")]
pub use settings::Settings;
