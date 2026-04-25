//! Application context: shared database pool and service handles.

/// Shared application context threaded through every command handler.
#[non_exhaustive]
pub struct AppContext {
    /// Whether to emit JSON instead of human-readable output.
    pub json: bool,
    /// Loaded importer plugins (WASM + any native adapters).
    pub importers: bc_core::ImporterRegistry,
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
    /// Opens the SQLite database at `db_path` (creating it and its parent
    /// directories if they do not exist), loads plugins from the configured
    /// search paths, and initialises all core services.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path to the SQLite database file.
    /// * `json` - Whether commands should emit JSON output.
    /// * `settings` - Application settings (used to resolve plugin search paths).
    ///
    /// # Errors
    ///
    /// Returns [`bc_core::BcError`] if the database cannot be opened,
    /// migrations fail, or the plugin registry cannot initialise.
    #[inline]
    pub async fn open(
        db_path: &std::path::Path,
        json: bool,
        settings: &bc_config::Settings,
    ) -> bc_core::BcResult<Self> {
        let pool = bc_core::open_db_at(db_path).await?;

        // Build plugin search paths: config dirs + binary sidecar dir.
        let mut plugin_paths = settings.plugin_paths().to_vec();
        if let Ok(exe) = std::env::current_exe() {
            if let Some(sidecar) = exe
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join("share").join("borrow-checker").join("plugins"))
            {
                if !plugin_paths.contains(&sidecar) {
                    plugin_paths.push(sidecar);
                }
            }
        }

        let importers = bc_plugins::PluginRegistry::load(&plugin_paths)
            .map_err(|e| bc_core::BcError::InvalidInput(e.to_string()))?
            .into_importer_registry();

        Ok(Self {
            json,
            importers,
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
