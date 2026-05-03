//! Application context: shared database pool and service handles.

/// Shared application context threaded through every command handler.
#[non_exhaustive]
pub struct AppContext {
    /// Whether to emit JSON instead of human-readable output.
    pub json: bool,
    /// Raw plugin registry — retains manifest metadata for `plugin list`.
    pub plugin_registry: bc_plugins::PluginRegistry,
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
    /// Opens the SQLite database (creating it and its parent directories if
    /// they do not exist), loads plugins from the configured search paths, and
    /// initialises all core services.
    ///
    /// The database path is resolved from `settings.db_path()`, falling back
    /// to [`bc_config::default_db_path`] when no path is configured.
    ///
    /// # Arguments
    ///
    /// * `settings` - Application settings (database path, plugin search paths).
    /// * `json` - Whether commands should emit JSON output.
    ///
    /// # Errors
    ///
    /// Returns [`bc_core::BcError`] if the database directory cannot be
    /// created, the database cannot be opened, migrations fail, or the plugin
    /// registry cannot initialise.
    #[inline]
    pub async fn open(settings: &bc_config::Settings, json: bool) -> bc_core::BcResult<Self> {
        let db_path = settings
            .db_path()
            .map_or_else(bc_config::default_db_path, std::path::Path::to_path_buf);

        if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| bc_core::BcError::InvalidInput(e.to_string()))?;
        }

        let pool = bc_core::open_db_at(&db_path).await?;

        let plugin_registry = bc_plugins::PluginRegistry::load(settings.plugin_paths())
            .map_err(|e| bc_core::BcError::InvalidInput(e.to_string()))?;
        let importers = plugin_registry.build_importer_registry();

        Ok(Self {
            json,
            plugin_registry,
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
