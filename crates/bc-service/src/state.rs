//! Shared services and the startup sequence that builds them.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use crate::commands::backup::restore_marker_path;
use crate::commands::plugins::load_plugin_registry;

/// How long [`AppState::open`] waits for another process to release the lock.
///
/// A desktop restart spawns the new process before the old one exits.
const LOCK_WAIT: Duration = Duration::from_secs(2);

/// Pause between lock attempts while waiting.
const LOCK_RETRY: Duration = Duration::from_millis(100);

/// Services and startup snapshot shared by every command.
///
/// Pre-built services share the underlying SQLite pool via internal cloning.
/// Stored here rather than a raw pool so hosts need not name `sqlx` types.
///
/// Both hosts share one state across threads, so every field is
/// `Send + Sync`.
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "fields are crate-internal; getters add no value for an internal state bag"
)]
#[expect(
    dead_code,
    reason = "the import commands that read the engine fields are added next"
)]
pub struct AppState {
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
    /// Metadata key registry — lookup, retype, and rename.
    pub(crate) metadata: bc_core::MetadataService,
    /// Commodity/currency registry service.
    pub(crate) commodities: bc_core::CommodityService,
    /// Transfer resolution service — merge/unmerge and suggestion matching.
    pub(crate) transfers: bc_core::TransferService,
    /// Backup service (snapshot + rotation), shared with [`Self::engine`] so a
    /// policy saved from Settings reaches pre-import snapshots too.
    pub(crate) backup: Arc<bc_core::BackupService>,
    /// Resolved database file path (used by restore).
    pub(crate) db_path: PathBuf,
    /// Import profile storage.
    pub(crate) profiles: bc_core::ImportProfileService,
    /// Import batch provenance: listing, discard preview and discard.
    pub(crate) batches: bc_core::ImportBatchService,
    /// Runs import profiles; preview and commit both go through it.
    pub(crate) engine: bc_core::ImportEngine,
    /// Importers by name: every loaded plugin, plus any the opener registered.
    pub(crate) importers: Arc<bc_core::ImporterRegistry>,
    /// The plugins loaded at startup, or `None` when the registry failed to
    /// initialise.
    pub(crate) plugin_registry: Option<bc_plugins::PluginRegistry>,
    /// Whether `import.documents-root` is set; without it no importer can
    /// read a file.
    pub(crate) documents_root_set: bool,
    /// Whether to snapshot before discarding an import batch
    /// (`backup.auto-pre-discard`).
    pub(crate) auto_pre_discard: bool,
    /// The open ledger's backup pool as of startup; a restore is confined to it.
    startup_backup_dir: PathBuf,
    /// Held for the life of the state: keeps a second host off the database.
    _lock: bc_core::DbLock,
}

impl AppState {
    /// Opens the database named by `settings` and builds every service.
    ///
    /// Takes the database lock first, waiting briefly for a departing process
    /// to release it, applies a pending restore, snapshots before pending
    /// migrations if the policy asks, then seeds default commodities.
    ///
    /// # Arguments
    ///
    /// * `settings` - Loaded configuration.
    ///
    /// # Returns
    ///
    /// The ready state.
    ///
    /// # Errors
    ///
    /// Returns [`bc_ipc::BcError::Internal`] if the lock is still held
    /// elsewhere after the wait, the database cannot be opened or migrated, or
    /// seeding fails.
    #[inline]
    pub async fn open(settings: &bc_config::Settings) -> Result<Self, bc_ipc::BcError> {
        Self::open_with_importers(settings, |_| {}).await
    }

    /// Opens the state as [`Self::open`] does, letting `register` add
    /// importers beside the loaded plugins.
    ///
    /// Plugins are WASM components that need a `wasm32-wasip2` build, so a
    /// test registers a native importer here instead.
    ///
    /// # Arguments
    ///
    /// * `settings` - Loaded configuration.
    /// * `register` - Adds importers to the registry the engine runs.
    ///
    /// # Returns
    ///
    /// The ready state.
    ///
    /// # Errors
    ///
    /// As [`Self::open`].
    pub(crate) async fn open_with_importers(
        settings: &bc_config::Settings,
        register: impl FnOnce(&mut bc_core::ImporterRegistry),
    ) -> Result<Self, bc_ipc::BcError> {
        let internal = |e: &dyn core::fmt::Display| bc_ipc::BcError::Internal(e.to_string());
        let db_path = prepare_db_path(settings).map_err(|e| internal(&e))?;
        let lock = acquire_lock(&db_path).await.map_err(|e| internal(&e))?;
        apply_pending_restore(&db_path).await;

        let b = settings.backup();
        let policy = bc_core::BackupPolicy::new(
            b.resolved_dir(),
            b.retain_count(),
            b.retain_days(),
            b.auto_pre_migration(),
        );
        let pool = bc_core::open_db_with_backup(&db_path, &policy)
            .await
            .map_err(|e| internal(&e))?;
        let ledger_id = bc_core::ensure_ledger_id(&pool)
            .await
            .map_err(|e| internal(&e))?;
        let backup = Arc::new(bc_core::BackupService::new(
            pool.clone(),
            db_path.clone(),
            ledger_id,
            policy,
        ));
        let startup_backup_dir = backup.pool_dir();
        // A missing pool only disables restore, which refuses it at call time.
        if let Err(e) = std::fs::create_dir_all(&startup_backup_dir) {
            tracing::warn!(
                path = %startup_backup_dir.display(),
                error = %e,
                "could not create the backup pool directory"
            );
        }
        let plugin_registry = load_plugin_registry(settings);
        let mut registry = plugin_registry.as_ref().map_or_else(
            bc_core::ImporterRegistry::new,
            bc_plugins::PluginRegistry::build_importer_registry,
        );
        register(&mut registry);
        let importers = Arc::new(registry);

        let fx = bc_core::noop_fx();
        let commodities = bc_core::CommodityService::new(pool.clone());
        commodities
            .seed_defaults()
            .await
            .map_err(|e| internal(&e))?;

        let accounts = bc_core::AccountService::new(pool.clone());
        let transactions = bc_core::TransactionService::new(pool.clone());
        let tags = bc_core::TagService::new(pool.clone());
        let profiles = bc_core::ImportProfileService::new(pool.clone());
        let batches = bc_core::ImportBatchService::new(pool.clone());
        let engine = bc_core::ImportEngine::builder()
            .transactions(transactions.clone())
            .sources(bc_core::SourceService::new(pool.clone()))
            .accounts(accounts.clone())
            .commodities(commodities.clone())
            .tags(tags.clone())
            .batches(batches.clone())
            .profiles(profiles.clone())
            .importers(Arc::clone(&importers))
            .backup(Arc::clone(&backup))
            .snapshot_before_write(b.auto_pre_import())
            .build();

        Ok(Self {
            accounts,
            transactions,
            balance_engine: bc_core::BalanceEngine::new(pool.clone()),
            budgets: bc_core::BudgetService::new(pool.clone()),
            tags,
            metadata: bc_core::MetadataService::new(pool.clone()),
            commodities,
            budget_tree: bc_core::BudgetTreeService::new(pool.clone(), fx),
            transfers: bc_core::TransferService::new(pool),
            backup,
            db_path,
            profiles,
            batches,
            engine,
            importers,
            plugin_registry,
            documents_root_set: settings.documents_root().is_some(),
            auto_pre_discard: b.auto_pre_discard(),
            startup_backup_dir,
            _lock: lock,
        })
    }

    /// Closes the shared pool, checkpointing the WAL into the database file.
    #[inline]
    pub async fn close(&self) {
        self.backup.close_pool().await;
    }

    /// Returns the open ledger's backup pool as of startup; a restore is
    /// confined to it.
    ///
    /// # Returns
    ///
    /// The startup pool, unchanged by later settings updates.
    #[inline]
    #[must_use]
    pub fn backup_dir(&self) -> PathBuf {
        self.startup_backup_dir.clone()
    }
}

/// Takes the database lock, retrying while another process holds it.
///
/// # Arguments
///
/// * `db_path` - Path of the database file to lock.
///
/// # Returns
///
/// The held lock.
///
/// # Errors
///
/// Returns [`bc_core::BcError::DatabaseInUse`] if the lock is still held after
/// [`LOCK_WAIT`], or any other error from [`bc_core::DbLock::acquire`] at once.
async fn acquire_lock(db_path: &Path) -> bc_core::BcResult<bc_core::DbLock> {
    let started = Instant::now();
    loop {
        let result = bc_core::DbLock::acquire(db_path);
        if matches!(result, Err(bc_core::BcError::DatabaseInUse(_)))
            && started.elapsed() < LOCK_WAIT
        {
            tokio::time::sleep(LOCK_RETRY).await;
            continue;
        }
        return result;
    }
}

/// Applies a pending restore marker before any database connection is opened.
///
/// Any failure here must NOT abort startup: startup continues with the intact
/// original database either way. The swap is atomic, so the marker is kept on
/// swap failure and the next launch retries; it is removed only after a
/// successful swap, or when it is unreadable (nothing usable can be done with
/// it). The pre-restore safety snapshot remains for manual recovery.
///
/// # Arguments
///
/// * `db_path` - Path of the live database file to swap the candidate in over.
async fn apply_pending_restore(db_path: &Path) {
    let marker = restore_marker_path(db_path);
    if !marker.exists() {
        return;
    }
    let candidate = match std::fs::read_to_string(&marker) {
        Ok(candidate) => candidate,
        Err(read_err) => {
            tracing::warn!(
                error = %read_err,
                "failed to read restore marker; removing it and keeping existing database"
            );
            if let Err(rm_err) = std::fs::remove_file(&marker) {
                tracing::warn!(error = %rm_err, "failed to remove unreadable restore marker");
            }
            return;
        }
    };
    match bc_core::BackupService::swap_in(Path::new(candidate.trim()), db_path).await {
        Ok(()) => {
            if let Err(rm_err) = std::fs::remove_file(&marker) {
                tracing::warn!(
                    error = %rm_err,
                    "failed to remove restore marker after successful swap"
                );
            }
        }
        Err(swap_err) => {
            tracing::warn!(
                error = %swap_err,
                "failed to swap in restore candidate; keeping existing \
                 database and retaining marker to retry next launch"
            );
        }
    }
}

/// Resolves the database path from `settings` and creates its directory.
///
/// # Arguments
///
/// * `settings` - The settings the host loaded at startup.
///
/// # Returns
///
/// The path of the database file to open.
///
/// # Errors
///
/// Returns an I/O error if the database's parent directory cannot be created.
fn prepare_db_path(settings: &bc_config::Settings) -> std::io::Result<PathBuf> {
    let db_path = settings.db_path();
    tracing::info!(db_path = %db_path.display(), "opening database");
    if let Some(parent) = db_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    Ok(db_path)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_core::Directive;
    use bc_core::ImportConfig;
    use bc_core::ImportError;
    use bc_core::Importer;
    use bc_core::ImporterFactory;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::commands::plugins::list_plugins;

    /// Parses nothing; enough to prove the opener's registration reached the engine.
    struct EmptyImporter;

    impl Importer for EmptyImporter {
        fn name(&self) -> &'static str {
            "empty"
        }

        fn import(&self, _config: &ImportConfig) -> Result<Vec<Directive>, ImportError> {
            Ok(Vec::new())
        }

        fn validate(&self, _config: &ImportConfig) -> Result<(), ImportError> {
            Ok(())
        }
    }

    /// Builds an [`EmptyImporter`].
    fn make_empty() -> Box<dyn Importer> {
        Box::new(EmptyImporter)
    }

    /// Settings that keep the database and backups inside `dir`.
    fn settings_in(dir: &tempfile::TempDir) -> bc_config::Settings {
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(dir.path().join("ledger.db"));
        settings.set_backup_dir(dir.path().join("backups"));
        settings
    }

    /// Compiles only for a value both hosts can share across threads.
    fn assert_send_sync<T>(_: &T)
    where
        T: Send + Sync,
    {
    }

    #[test]
    fn prepare_db_path_uses_the_configured_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let configured = dir.path().join("ledger").join("db.sqlite");
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(configured.clone());

        let db_path = prepare_db_path(&settings).expect("prepare");

        assert_eq!(db_path, configured);
        assert!(
            dir.path().join("ledger").is_dir(),
            "parent directory created"
        );
    }

    #[tokio::test]
    async fn open_with_importers_adds_to_the_plugin_importers() {
        let dir = tempfile::tempdir().expect("tempdir");

        let state = AppState::open_with_importers(&settings_in(&dir), |registry| {
            registry.register(ImporterFactory::new("empty", make_empty));
        })
        .await
        .expect("open");

        let names: Vec<&str> = state.importers.names().collect();
        assert_eq!(names, vec!["empty"]);
        assert!(
            state
                .plugin_registry
                .as_ref()
                .is_some_and(bc_plugins::PluginRegistry::is_empty),
            "no plugin directory is configured, so the registry loads empty"
        );
    }

    #[tokio::test]
    async fn open_reads_the_import_settings() {
        let dir = tempfile::tempdir().expect("tempdir");

        let state = AppState::open(&settings_in(&dir)).await.expect("open");

        assert_send_sync(&state.plugin_registry);
        assert_send_sync(&state.engine);
        assert_send_sync(&state);
        assert!(!state.documents_root_set, "the default leaves it unset");
        assert!(
            state.auto_pre_discard,
            "the default snapshots before a discard"
        );
        assert_eq!(list_plugins(&state).expect("list"), Vec::new());
    }
}
