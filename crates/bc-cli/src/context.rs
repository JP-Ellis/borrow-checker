//! Application context: shared database pool and service handles.

use std::path::PathBuf;

/// Shared application context threaded through every command handler.
#[non_exhaustive]
pub struct AppContext {
    /// Whether to emit JSON instead of human-readable output.
    pub json: bool,
    /// Account service.
    pub accounts: bc_core::AccountService,
    /// Transaction service.
    pub transactions: bc_core::TransactionService,
    /// Balance computation engine.
    pub balances: bc_core::BalanceEngine,
    /// Import profile service.
    pub profiles: bc_core::ImportProfileService,
}

impl AppContext {
    /// Opens the SQLite database at `db_path` (creating it and its parent directories
    /// if they do not exist) and initialises all core services.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the SQLite database file.
    /// * `json` - Whether commands should emit JSON output.
    ///
    /// # Errors
    ///
    /// Returns [`bc_core::BcError`] if the database cannot be opened or migrations fail.
    #[inline]
    pub async fn open(db_path: &std::path::Path, json: bool) -> bc_core::BcResult<Self> {
        let url = format!("sqlite://{}?mode=rwc", db_path.display());
        let pool = bc_core::open_db(&url).await?;
        Ok(Self {
            json,
            accounts: bc_core::AccountService::new(pool.clone()),
            transactions: bc_core::TransactionService::new(pool.clone()),
            balances: bc_core::BalanceEngine::new(pool.clone()),
            profiles: bc_core::ImportProfileService::new(pool),
        })
    }
}

/// Returns the default database path based on the platform data directory.
///
/// Priority:
/// 1. Platform data dir: `$XDG_DATA_HOME/borrow-checker/db.sqlite` (Linux),
///    `~/Library/Application Support/borrow-checker/db.sqlite` (macOS).
/// 2. Fallback: `./borrow-checker.db` in the current directory.
#[must_use]
#[inline]
pub fn default_db_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "borrow-checker").map_or_else(
        || PathBuf::from("borrow-checker.db"),
        |dirs| dirs.data_dir().join("db.sqlite"),
    )
}
