//! BorrowChecker desktop GUI — Tauri application library.
//!
//! Exposes [`run`], called by both `main.rs` (desktop) and the Tauri mobile
//! entry point (future work). All command handlers are registered here.

pub mod commands;
pub(crate) mod ipc;

use bc_core::Importer as _;
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
            // MARK: accounts — see bc_ipc::commands for canonical name strings
            commands::accounts::list_accounts,
            commands::accounts::list_transactions,
            commands::accounts::create_transaction,
            commands::accounts::get_account_stats,
            commands::accounts::get_account_sparkline,
            commands::accounts::get_uncategorised_count,
            // MARK: plugins
            commands::plugins::list_plugins,
        ])
        .setup(|app| {
            let db_path = std::env::var("BC_DB_PATH")
                .map_or_else(|_| bc_config::default_db_path(), std::path::PathBuf::from);

            let pool = tauri::async_runtime::block_on(bc_core::open_db_at(&db_path))?;

            let plugins = collect_plugin_info();

            app.manage(AppState {
                accounts: bc_core::AccountService::new(pool.clone()),
                transactions: bc_core::TransactionService::new(pool.clone()),
                balance_engine: bc_core::BalanceEngine::new(pool),
                plugins,
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running borrow-checker");
}

/// Loads plugin metadata from configured search paths.
///
/// Builds a [`bc_plugins::PluginRegistry`] using paths from [`bc_config::Settings`],
/// then immediately converts the loaded plugins into plain [`bc_ipc::PluginInfo`]
/// values. This allows the metadata to be stored in [`AppState`] and cloned
/// cheaply, avoiding the need to store the non-`Clone` registry itself.
///
/// # Returns
///
/// A `Vec` of [`bc_ipc::PluginInfo`] for all successfully loaded plugins.
/// Returns an empty `Vec` if no plugins are found or the registry fails to
/// initialise.
fn collect_plugin_info() -> Vec<bc_ipc::PluginInfo> {
    let settings = bc_config::Settings::load().unwrap_or_default();
    let paths = settings.plugin_paths().to_owned();

    bc_plugins::PluginRegistry::load(&paths).map_or_else(
        |_| Vec::new(),
        |registry| {
            registry
                .plugins()
                .map(|p| {
                    bc_ipc::PluginInfo::new(
                        p.name().to_owned(),
                        p.sdk_abi(),
                        p.source_path().display().to_string(),
                    )
                })
                .collect()
        },
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_compiles() {}
}
