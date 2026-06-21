//! BorrowChecker desktop GUI — Tauri application library.
//!
//! Exposes [`run`], called by both `main.rs` (desktop) and the Tauri mobile
//! entry point (future work). All command handlers are registered here.

pub mod commands;
pub(crate) mod ipc;

use tauri::Manager as _;

/// Application state held in Tauri's managed-state system.
///
/// Pre-built services share the underlying SQLite pool via internal cloning.
/// Stored here rather than a raw pool so `bc-app` need not name `sqlx` types.
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "fields are crate-internal; getters add no value for an app-internal state bag"
)]
pub(crate) struct AppState {
    /// Account projection service.
    pub(crate) accounts: bc_core::AccountService,
    /// Transaction service.
    pub(crate) transactions: bc_core::TransactionService,
    /// Balance engine — computes running balances and cash-flow aggregations.
    pub(crate) balance_engine: bc_core::BalanceEngine,
    /// Budget CRUD service.
    pub(crate) budgets: bc_core::BudgetService,
    /// Budget tree and overview service.
    pub(crate) budget_tree: bc_core::BudgetTreeService,
    /// Snapshot of installed plugin metadata, collected at startup.
    ///
    /// `PluginRegistry` is not `Clone` (Wasmtime components are not `Clone`),
    /// so we eagerly collect plain [`bc_ipc::PluginInfo`] values and store
    /// them here for zero-cost repeated reads.
    pub(crate) plugins: Vec<bc_ipc::PluginInfo>,
}

/// Initialise and run the Tauri application.
///
/// # Panics
///
/// Panics if Tauri cannot initialise the `WebView` runtime. This is
/// unrecoverable for a desktop GUI.
#[expect(
    clippy::expect_used,
    reason = "Tauri startup failure is unrecoverable for a desktop GUI"
)]
#[expect(
    clippy::exit,
    reason = "tauri::generate_context!() macro internally calls process::exit"
)]
#[inline]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            commands::accounts::list_accounts,
            commands::accounts::list_transactions,
            commands::accounts::create_transaction,
            commands::accounts::reverse_transaction,
            commands::accounts::get_account_stats,
            commands::accounts::get_account_sparkline,
            commands::accounts::get_posting_count,
            commands::plugins::list_plugins,
            commands::settings::get_settings,
            commands::budget::get_budget_overview,
            commands::budget::get_native_periods,
            commands::budget::get_budget_transactions,
            commands::budget::list_budget_revisions,
            commands::budget::resolve_effective_date,
            commands::budget::revise_budget,
            commands::budget::remove_budget_revision,
            commands::budget::archive_budget,
            commands::budget::create_budget,
            commands::budget::set_posting_spread,
            commands::budget::clear_posting_spread,
        ])
        .setup(|app| {
            let db_path = std::env::var("BC_DB_PATH")
                .map_or_else(|_| bc_config::default_db_path(), std::path::PathBuf::from);

            let pool = tauri::async_runtime::block_on(bc_core::open_db_at(&db_path))?;

            let plugins = commands::plugins::collect_plugin_info();
            let fx = bc_core::noop_fx();

            app.manage(AppState {
                accounts: bc_core::AccountService::new(pool.clone()),
                transactions: bc_core::TransactionService::new(pool.clone()),
                balance_engine: bc_core::BalanceEngine::new(pool.clone()),
                budgets: bc_core::BudgetService::new(pool.clone()),
                budget_tree: bc_core::BudgetTreeService::new(pool, fx),
                plugins,
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running borrow-checker");
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
