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
            // MARK: accounts — see bc_ipc::commands for canonical name strings
            commands::accounts::list_accounts,
            commands::accounts::list_transactions,
            commands::accounts::create_transaction,
            commands::accounts::get_account_stats,
            commands::accounts::get_account_sparkline,
            commands::accounts::get_uncategorised_count,
        ])
        .setup(|app| {
            let db_path = std::env::var("BC_DB_PATH")
                .map_or_else(|_| bc_config::default_db_path(), std::path::PathBuf::from);

            let pool = tauri::async_runtime::block_on(bc_core::open_db_at(&db_path))?;
            app.manage(AppState {
                accounts: bc_core::AccountService::new(pool.clone()),
                transactions: bc_core::TransactionService::new(pool.clone()),
                balance_engine: bc_core::BalanceEngine::new(pool),
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
