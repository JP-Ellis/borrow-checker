//! Runs import profiles end to end: importer resolution, parsing, the plan or
//! the run itself, and — for a sweep — the one snapshot taken before the
//! first write.
//!
//! [`ImportEngine::run_profile`] is one profile, one batch, no snapshot, and
//! never returns an error: every failure travels in the result so a sweep
//! can continue past it. [`ImportEngine::sync`] is the sweep.

use std::path::PathBuf;
use std::sync::Arc;

use bc_models::ProfileId;

use crate::BackupService;
use crate::ImportOutcome;
use crate::ImportPlan;
use crate::ImportProfile;
use crate::ImporterRegistry;
use crate::execute_import;
use crate::plan_import;

/// Whether a run writes.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Resolve and report; open no batch, write nothing, take no snapshot.
    DryRun,
    /// Import for real: one batch per profile.
    Commit,
}

/// Which profiles a sweep covers.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// One profile, by its unique name.
    One(String),
    /// Every profile, in name order.
    All,
}

/// The identity of the profile a result belongs to.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileRef {
    /// The profile's id.
    pub id: ProfileId,
    /// The profile's unique name.
    pub name: String,
    /// The importer the profile names.
    pub importer: String,
}

impl From<&ImportProfile> for ProfileRef {
    #[inline]
    fn from(profile: &ImportProfile) -> Self {
        Self {
            id: profile.id.clone(),
            name: profile.name.clone(),
            importer: profile.importer.clone(),
        }
    }
}

/// What a profile's run produced, by [`Mode`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileRun {
    /// A dry run's plan.
    Planned(ImportPlan),
    /// A committed run's outcome, carrying its batch id.
    Imported(ImportOutcome),
}

/// Where a profile's run failed.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureStage {
    /// The profile names an importer the registry does not hold.
    UnknownImporter,
    /// [`crate::Importer::import`] failed: unreadable or unparseable files.
    Importer,
    /// [`plan_import`] or [`execute_import`] returned an error.
    Engine,
}

impl FailureStage {
    /// Returns the stable snake-case key a machine-readable report uses.
    #[must_use]
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::UnknownImporter => "unknown_importer",
            Self::Importer => "importer",
            Self::Engine => "engine",
        }
    }
}

/// Why a profile's run produced nothing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileFailure {
    /// The step that failed.
    pub stage: FailureStage,
    /// The underlying error's message, without a stage prefix.
    pub message: String,
}

impl std::fmt::Display for ProfileFailure {
    /// Renders the message the CLI has always printed for this failure:
    /// importer errors carry an `import error: ` prefix, the others none.
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.stage {
            FailureStage::Importer => write!(f, "import error: {}", self.message),
            FailureStage::UnknownImporter | FailureStage::Engine => f.write_str(&self.message),
        }
    }
}

/// One profile's share of a sweep.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileResult {
    /// The profile that ran.
    pub profile: ProfileRef,
    /// Its plan or outcome, or why it produced neither.
    pub result: Result<ProfileRun, ProfileFailure>,
}

/// What a sweep did.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncReport {
    /// The pre-import snapshot, when the sweep committed and the policy asked
    /// for one.
    pub snapshot: Option<PathBuf>,
    /// One entry per profile, in the order they ran.
    pub profiles: Vec<ProfileResult>,
}

/// Runs import profiles through the shared engine.
///
/// Holds every service a run reads or writes, the importer registry, and the
/// backup service the sweep snapshots through. Construct with
/// [`ImportEngine::builder`].
#[non_exhaustive]
pub struct ImportEngine {
    /// Transaction persistence.
    transactions: crate::TransactionService,
    /// Source-reference persistence.
    sources: crate::SourceService,
    /// Account lookup, snapshotted per run for path resolution.
    accounts: crate::AccountService,
    /// Commodity lookup, snapshotted per run for code resolution.
    commodities: crate::CommodityService,
    /// Tag materialisation.
    tags: crate::TagService,
    /// Batch provenance.
    batches: crate::ImportBatchService,
    /// Profile lookup, for [`Self::sync`]'s selection.
    profiles: crate::ImportProfileService,
    /// Importers by name.
    importers: Arc<ImporterRegistry>,
    /// Snapshot writer.
    backup: Arc<BackupService>,
    /// Take a `PreImport` snapshot before the first write of a commit sweep.
    snapshot_before_write: bool,
}

#[bon::bon]
impl ImportEngine {
    /// Builds an engine over the given services.
    ///
    /// # Arguments
    ///
    /// One per field; see the struct's field docs.
    #[builder]
    #[inline]
    pub fn new(
        transactions: crate::TransactionService,
        sources: crate::SourceService,
        accounts: crate::AccountService,
        commodities: crate::CommodityService,
        tags: crate::TagService,
        batches: crate::ImportBatchService,
        profiles: crate::ImportProfileService,
        importers: Arc<ImporterRegistry>,
        backup: Arc<BackupService>,
        snapshot_before_write: bool,
    ) -> Self {
        Self {
            transactions,
            sources,
            accounts,
            commodities,
            tags,
            batches,
            profiles,
            importers,
            backup,
            snapshot_before_write,
        }
    }

    /// Runs one profile: resolves its importer, parses its files, then plans
    /// or executes by `mode`. Opens one batch in [`Mode::Commit`]. Never
    /// snapshots and never returns an error: every failure is carried in the
    /// result so a sweep can continue past it.
    ///
    /// # Arguments
    ///
    /// * `profile` - The profile to run.
    /// * `mode` - Whether to write.
    ///
    /// # Returns
    ///
    /// The profile's plan, outcome, or failure.
    #[inline]
    pub async fn run_profile(&self, profile: &ImportProfile, mode: Mode) -> ProfileResult {
        ProfileResult {
            profile: ProfileRef::from(profile),
            result: self.run_inner(profile, mode).await,
        }
    }

    /// The body of [`Self::run_profile`], with `?` available.
    async fn run_inner(
        &self,
        profile: &ImportProfile,
        mode: Mode,
    ) -> Result<ProfileRun, ProfileFailure> {
        let importer = self
            .importers
            .create_for_name(&profile.importer)
            .ok_or_else(|| ProfileFailure {
                stage: FailureStage::UnknownImporter,
                message: format!(
                    "unknown importer '{}' for profile '{}'",
                    profile.importer, profile.name
                ),
            })?;

        // The importer is a synchronous Wasmtime call over the profile's
        // files; run inline it would stall the caller's runtime for the
        // whole parse.
        let config = profile.config.clone();
        let raws = tokio::task::spawn_blocking(move || importer.import(&config))
            .await
            .map_err(|join| ProfileFailure {
                stage: FailureStage::Importer,
                message: format!("importer task failed: {join}"),
            })?
            .map_err(|error| ProfileFailure {
                stage: FailureStage::Importer,
                message: error.to_string(),
            })?;

        let run = match mode {
            Mode::DryRun => plan_import(
                &self.transactions,
                &self.sources,
                &self.accounts,
                &self.commodities,
                &self.tags,
                &self.batches,
                Some(&profile.id),
                &profile.importer,
                &raws,
            )
            .await
            .map(ProfileRun::Planned),
            Mode::Commit => execute_import(
                &self.transactions,
                &self.sources,
                &self.accounts,
                &self.commodities,
                &self.tags,
                &self.batches,
                Some(&profile.id),
                &profile.importer,
                &raws,
            )
            .await
            .map(ProfileRun::Imported),
        };
        run.map_err(|error| ProfileFailure {
            stage: FailureStage::Engine,
            message: error.to_string(),
        })
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use bc_models::Amount;
    use bc_models::CommodityCode;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::*;
    use crate::BackupPolicy;
    use crate::ImportConfig;
    use crate::ImportError;
    use crate::Importer;
    use crate::ImporterFactory;
    use crate::ImporterRegistry;
    use crate::RawPosting;
    use crate::RawTransaction;

    /// Yields one single-leg row on `Assets:Bank`, which no test creates, so
    /// every run skips it as an unresolved account. That is enough to prove a
    /// batch opened or a plan was made; the resolution itself is covered in
    /// `import_exec`.
    struct StubImporter;

    impl Importer for StubImporter {
        fn name(&self) -> &'static str {
            "stub"
        }

        fn import(&self, _config: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
            Ok(vec![
                RawTransaction::builder()
                    .date(date(2026, 3, 14))
                    .description("STUB ROW")
                    .postings(vec![
                        RawPosting::builder()
                            .account("Assets:Bank")
                            .amount(Amount::new(
                                Decimal::from(-25_i64),
                                CommodityCode::new("AUD"),
                            ))
                            .build(),
                    ])
                    .build(),
            ])
        }

        fn validate(&self, _config: &ImportConfig) -> Result<(), ImportError> {
            Ok(())
        }
    }

    /// Fails every import, standing in for an unreadable export.
    struct FailingImporter;

    impl Importer for FailingImporter {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn import(&self, _config: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
            Err(ImportError::BadValue {
                field: "source_dir".to_owned(),
                detail: "no such directory".to_owned(),
            })
        }

        fn validate(&self, _config: &ImportConfig) -> Result<(), ImportError> {
            Ok(())
        }
    }

    fn make_stub() -> Box<dyn Importer> {
        Box::new(StubImporter)
    }

    fn make_failing() -> Box<dyn Importer> {
        Box::new(FailingImporter)
    }

    /// Everything a test reaches for: the engine, the services it shares,
    /// and the directory snapshots land in.
    struct Fixture {
        engine: ImportEngine,
        profiles: crate::ImportProfileService,
        batches: crate::ImportBatchService,
        backup_dir: PathBuf,
    }

    /// Builds an engine over a fresh on-disk database under `dir`, with the
    /// `stub` and `failing` importers registered.
    async fn fixture_in(dir: &std::path::Path, snapshot_before_write: bool) -> Fixture {
        let db_path = dir.join("db.sqlite");
        let backup_dir = dir.join("backups");
        let pool = crate::open_db_at(&db_path).await.expect("open db");
        let policy = BackupPolicy::new(backup_dir.clone(), Some(5), None, false);

        let commodities = crate::CommodityService::new(pool.clone());
        commodities.seed_defaults().await.expect("seed commodities");

        let mut importers = ImporterRegistry::new();
        importers.register(ImporterFactory::new("stub", make_stub));
        importers.register(ImporterFactory::new("failing", make_failing));

        let profiles = crate::ImportProfileService::new(pool.clone());
        let batches = crate::ImportBatchService::new(pool.clone());
        let engine = ImportEngine::builder()
            .transactions(crate::TransactionService::new(pool.clone()))
            .sources(crate::SourceService::new(pool.clone()))
            .accounts(crate::AccountService::new(pool.clone()))
            .commodities(commodities)
            .tags(crate::TagService::new(pool.clone()))
            .batches(batches.clone())
            .profiles(profiles.clone())
            .importers(Arc::new(importers))
            .backup(Arc::new(crate::BackupService::new(pool, db_path, policy)))
            .snapshot_before_write(snapshot_before_write)
            .build();
        Fixture {
            engine,
            profiles,
            batches,
            backup_dir,
        }
    }

    /// Creates a profile named `name` driving importer `importer`, returning it.
    async fn profile(fixture: &Fixture, name: &str, importer: &str) -> crate::ImportProfile {
        let id = fixture
            .profiles
            .create(
                name,
                importer,
                ImportConfig::from_value(serde_json::json!({})),
            )
            .await
            .expect("create profile");
        fixture
            .profiles
            .find_by_id(&id)
            .await
            .expect("find profile")
    }

    /// Counts `pre-import` snapshots in `dir`.
    fn pre_import_snapshots(dir: &std::path::Path) -> usize {
        std::fs::read_dir(dir).map_or(0, |entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .ends_with(".pre-import.sqlite")
                })
                .count()
        })
    }

    #[tokio::test]
    async fn run_profile_dry_run_plans_and_opens_no_batch() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        let nightly = profile(&fixture, "nightly", "stub").await;

        let result = fixture.engine.run_profile(&nightly, Mode::DryRun).await;

        assert_eq!(result.profile.name, "nightly");
        assert_eq!(result.profile.importer, "stub");
        assert!(
            matches!(result.result, Ok(ProfileRun::Planned(_))),
            "got {result:?}"
        );
        assert!(fixture.batches.list().await.expect("list").is_empty());
        assert_eq!(pre_import_snapshots(&fixture.backup_dir), 0);
    }

    #[tokio::test]
    async fn run_profile_commit_opens_one_batch_and_takes_no_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        let nightly = profile(&fixture, "nightly", "stub").await;

        let result = fixture.engine.run_profile(&nightly, Mode::Commit).await;

        let run = result.result.expect("an outcome");
        let batches = fixture.batches.list().await.expect("list");
        assert_eq!(batches.len(), 1);
        let opened = &batches.first().expect("one batch").id;
        assert!(
            matches!(&run, ProfileRun::Imported(outcome) if &outcome.batch_id == opened),
            "the outcome must carry the batch the run opened: {run:?}"
        );
        assert_eq!(
            pre_import_snapshots(&fixture.backup_dir),
            0,
            "run_profile never snapshots; that is sync's job"
        );
    }

    #[tokio::test]
    async fn an_unknown_importer_is_a_failure_not_an_error() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        let orphan = profile(&fixture, "orphan", "missing").await;

        let result = fixture.engine.run_profile(&orphan, Mode::Commit).await;

        let failure = result.result.expect_err("a failure");
        assert_eq!(failure.stage, FailureStage::UnknownImporter);
        assert_eq!(
            failure.to_string(),
            "unknown importer 'missing' for profile 'orphan'"
        );
        assert!(fixture.batches.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn an_importer_error_is_a_failure_at_the_importer_stage() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        let broken = profile(&fixture, "broken", "failing").await;

        let result = fixture.engine.run_profile(&broken, Mode::Commit).await;

        let failure = result.result.expect_err("a failure");
        assert_eq!(failure.stage, FailureStage::Importer);
        assert_eq!(
            failure.to_string(),
            "import error: bad value for field 'source_dir': no such directory"
        );
        assert!(fixture.batches.list().await.expect("list").is_empty());
    }

    #[test]
    fn failure_stage_labels_are_stable() {
        assert_eq!(FailureStage::UnknownImporter.label(), "unknown_importer");
        assert_eq!(FailureStage::Importer.label(), "importer");
        assert_eq!(FailureStage::Engine.label(), "engine");
    }
}
