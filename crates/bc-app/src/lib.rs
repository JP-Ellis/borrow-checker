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
    /// Tag service — hierarchy, resolution, and membership.
    pub(crate) tags: bc_core::TagService,
    /// Commodity/currency registry service.
    pub(crate) commodities: bc_core::CommodityService,
    /// Backup service (snapshot + rotation).
    pub(crate) backup: bc_core::BackupService,
    /// Resolved database file path (used by restore).
    pub(crate) db_path: std::path::PathBuf,
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
            commands::accounts::edit_transaction,
            commands::accounts::set_reconciliation,
            commands::accounts::get_transaction_audit,
            commands::accounts::get_account_stats,
            commands::accounts::get_account_sparkline,
            commands::accounts::account_latest_activity,
            commands::tags::create_tag,
            commands::tags::rename_tag,
            commands::tags::delete_tag,
            commands::tags::list_tags,
            commands::commodities::list_currencies,
            commands::commodities::create_currency,
            commands::commodities::update_currency,
            commands::commodities::delete_currency,
            commands::plugins::list_plugins,
            commands::settings::get_settings,
            commands::backup::backup_database,
            commands::backup::list_backups,
            commands::backup::restore_database,
            commands::backup::get_backup_settings,
            commands::backup::update_backup_settings,
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

            // Apply a pending restore before opening any connection. Any failure
            // here must NOT abort startup: a missing/unreadable candidate would
            // otherwise brick the app on every launch. Log and drop the marker so
            // the next launch proceeds with the existing database (the pre-restore
            // safety snapshot remains for manual recovery).
            let marker = commands::backup::restore_marker_path(&db_path);
            if marker.exists() {
                match std::fs::read_to_string(&marker) {
                    Ok(candidate) => {
                        if let Err(e) = bc_core::BackupService::swap_in(
                            std::path::Path::new(candidate.trim()),
                            &db_path,
                        ) {
                            tracing::warn!(
                                error = %e,
                                "failed to swap in restore candidate; keeping existing database"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "failed to read restore marker; keeping existing database"
                        );
                    }
                }
                if let Err(e) = std::fs::remove_file(&marker) {
                    tracing::warn!(error = %e, "failed to remove restore marker");
                }
            }

            let settings = bc_config::Settings::load().unwrap_or_default();
            let b = settings.backup();
            let policy = bc_core::BackupPolicy::new(
                b.resolved_dir(),
                b.retain_count(),
                b.retain_days(),
                b.auto_pre_migration(),
            );

            let pool =
                tauri::async_runtime::block_on(bc_core::open_db_with_backup(&db_path, &policy))?;

            let plugins = commands::plugins::collect_plugin_info();
            let fx = bc_core::noop_fx();

            let commodities = bc_core::CommodityService::new(pool.clone());
            tauri::async_runtime::block_on(commodities.seed_defaults())?;

            app.manage(AppState {
                accounts: bc_core::AccountService::new(pool.clone()),
                transactions: bc_core::TransactionService::new(pool.clone()),
                balance_engine: bc_core::BalanceEngine::new(pool.clone()),
                budgets: bc_core::BudgetService::new(pool.clone()),
                tags: bc_core::TagService::new(pool.clone()),
                commodities,
                budget_tree: bc_core::BudgetTreeService::new(pool.clone(), fx),
                backup: bc_core::BackupService::new(pool, db_path.clone(), policy),
                db_path,
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
