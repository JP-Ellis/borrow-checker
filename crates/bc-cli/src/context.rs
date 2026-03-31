//! Application context: shared database pool and service handles.

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
    /// Asset valuation and depreciation service.
    pub assets: bc_core::AssetService,
    /// Loan terms and amortization service.
    pub loans: bc_core::LoanService,
    /// Envelope service.
    pub envelopes: bc_core::EnvelopeService,
    /// Budget calculation engine.
    pub budget: bc_core::BudgetEngine,
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
        let pool = bc_core::open_db_at(db_path).await?;
        Ok(Self {
            json,
            accounts: bc_core::AccountService::new(pool.clone()),
            transactions: bc_core::TransactionService::new(pool.clone()),
            balances: bc_core::BalanceEngine::new(pool.clone()),
            profiles: bc_core::ImportProfileService::new(pool.clone()),
            assets: bc_core::AssetService::new(pool.clone()),
            loans: bc_core::LoanService::new(pool.clone()),
            envelopes: bc_core::EnvelopeService::new(pool.clone()),
            budget: bc_core::BudgetEngine::new(pool),
        })
    }
}
