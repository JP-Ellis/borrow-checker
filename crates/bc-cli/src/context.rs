//! Application context: shared database pool and service handles.

/// Shared application context threaded through every command handler.
#[non_exhaustive]
pub struct AppContext {
    /// Whether to emit JSON instead of human-readable output.
    pub json: bool,
    /// Fortnightly anchor date from config.
    pub fortnightly_anchor: Option<jiff::civil::Date>,
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
    /// Budget service.
    pub budgets: bc_core::BudgetService,
    /// Budget status engine.
    pub budget_status: bc_core::BudgetStatusEngine,
    /// Tag service.
    pub tags: bc_core::TagService,
    /// Backup service (snapshot + restore + rotation).
    pub backup: bc_core::BackupService,
    /// Resolved database file path (used by restore to swap the file).
    pub db_path: std::path::PathBuf,
    /// Source-reference service (import provenance / dedup).
    pub sources: bc_core::SourceService,
    /// Transfer resolution service (merge / unmerge / suggest).
    pub transfers: bc_core::TransferService,
}

impl AppContext {
    /// Opens the SQLite database (creating it and its parent directories if
    /// they do not exist), loads plugins from the configured search paths, and
    /// initialises all core services.
    ///
    /// The database path is resolved via [`bc_config::Settings::db_path`].
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
        let db_path = settings.db_path();

        if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| bc_core::BcError::InvalidInput(e.to_string()))?;
        }

        let backup_section = settings.backup();
        let policy = bc_core::BackupPolicy::new(
            backup_section.resolved_dir(),
            backup_section.retain_count(),
            backup_section.retain_days(),
            backup_section.auto_pre_migration(),
        );
        let pool = bc_core::open_db_with_backup(&db_path, &policy).await?;

        let plugin_registry =
            bc_plugins::PluginRegistry::load(settings.plugin_paths(), settings.documents_root())
                .map_err(|e| bc_core::BcError::InvalidInput(e.to_string()))?;
        let importers = plugin_registry.build_importer_registry();

        Ok(Self {
            json,
            fortnightly_anchor: settings.fortnightly_anchor(),
            plugin_registry,
            importers,
            accounts: bc_core::AccountService::new(pool.clone()),
            transactions: bc_core::TransactionService::new(pool.clone()),
            balances: bc_core::BalanceEngine::new(pool.clone()),
            profiles: bc_core::ImportProfileService::new(pool.clone()),
            assets: bc_core::AssetService::new(pool.clone()),
            loans: bc_core::LoanService::new(pool.clone()),
            budgets: bc_core::BudgetService::new(pool.clone()),
            tags: bc_core::TagService::new(pool.clone()),
            backup: bc_core::BackupService::new(pool.clone(), db_path.clone(), policy),
            db_path,
            sources: bc_core::SourceService::new(pool.clone()),
            transfers: bc_core::TransferService::new(pool.clone()),
            budget_status: bc_core::BudgetStatusEngine::new(pool, bc_core::noop_fx()),
        })
    }
}
