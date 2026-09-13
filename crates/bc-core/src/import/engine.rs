//! Runs import profiles end to end: importer resolution, parsing, the plan or
//! the run itself, and — for a sweep — the one snapshot taken before the
//! first write.
//!
//! [`ImportEngine::run_profile`] is one profile, one batch, no snapshot, and
//! never returns an error: every failure travels in the result so a sweep
//! can continue past it. [`ImportEngine::sync`] is the sweep.

use std::path::PathBuf;
use std::sync::Arc;

use bc_models::ImportBatchId;
use bc_models::ProfileId;

use crate::BackupKind;
use crate::BackupService;
use crate::BcError;
use crate::BcResult;
use crate::ImportAbort;
use crate::ImportOutcome;
use crate::ImportPlan;
use crate::ImportProfile;
use crate::ImporterRegistry;
use crate::RawTransaction;
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
    /// [`crate::Importer::import`] failed: unreadable or unparsable files.
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
///
/// Equality compares `stage` and `message` only: [`BcError`] is not
/// comparable, and the message is its rendering.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct ProfileFailure {
    /// The step that failed.
    pub stage: FailureStage,
    /// The underlying error's message, without a stage prefix.
    pub message: String,
    /// The engine's own error, on the [`FailureStage::Engine`] stage only,
    /// so a caller can keep its exit-code mapping. `None` on every other
    /// stage. Shared, since [`BcError`] does not clone.
    pub source: Option<Arc<BcError>>,
    /// The batch a committed run opened before it stopped, still holding
    /// every row written up to the stop; `import discard` undoes it. `None`
    /// when nothing was written.
    pub batch_id: Option<ImportBatchId>,
}

impl ProfileFailure {
    /// Builds a failure with no underlying [`BcError`].
    ///
    /// # Arguments
    ///
    /// * `stage` - The step that failed.
    /// * `message` - The error's message, without a stage prefix.
    #[must_use]
    #[inline]
    pub fn new(stage: FailureStage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
            source: None,
            batch_id: None,
        }
    }
}

impl From<BcError> for ProfileFailure {
    /// Wraps an engine error, keeping it as the failure's `source`.
    #[inline]
    fn from(error: BcError) -> Self {
        Self {
            stage: FailureStage::Engine,
            message: error.to_string(),
            source: Some(Arc::new(error)),
            batch_id: None,
        }
    }
}

impl From<ImportAbort> for ProfileFailure {
    /// Wraps a stopped run, keeping its error and the batch it left open.
    #[inline]
    fn from(abort: ImportAbort) -> Self {
        Self {
            batch_id: abort.batch_id,
            ..Self::from(*abort.source)
        }
    }
}

impl PartialEq for ProfileFailure {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        self.stage == other.stage && self.message == other.message
    }
}

impl Eq for ProfileFailure {}

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

/// A profile whose importer has parsed its files, ready to plan or import.
///
/// Splitting the run here lets [`ImportEngine::sync`] take its snapshot
/// after the parse and before the write, so a profile that fails to parse
/// costs no database copy.
struct Prepared<'a> {
    /// The profile being run.
    profile: &'a ImportProfile,
    /// What its importer yielded.
    raws: Vec<RawTransaction>,
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
        let result = match self.prepare(profile).await {
            Ok(prepared) => self.finish(prepared, mode).await,
            Err(failure) => Err(failure),
        };
        ProfileResult {
            profile: ProfileRef::from(profile),
            result,
        }
    }

    /// The read-only half of a run: resolves the importer and parses the
    /// profile's files. Nothing is written, so a failure here costs nothing
    /// to recover from.
    async fn prepare<'a>(
        &self,
        profile: &'a ImportProfile,
    ) -> Result<Prepared<'a>, ProfileFailure> {
        let importer = self
            .importers
            .create_for_name(&profile.importer)
            .ok_or_else(|| {
                ProfileFailure::new(
                    FailureStage::UnknownImporter,
                    format!(
                        "unknown importer '{}' for profile '{}'",
                        profile.importer, profile.name
                    ),
                )
            })?;

        // The importer is a synchronous Wasmtime call over the profile's
        // files; run inline it would stall the caller's runtime for the
        // whole parse.
        let config = profile.config.clone();
        let raws = tokio::task::spawn_blocking(move || importer.import(&config))
            .await
            .map_err(|join| {
                ProfileFailure::new(
                    FailureStage::Importer,
                    format!("importer task failed: {join}"),
                )
            })?
            .map_err(|error| ProfileFailure::new(FailureStage::Importer, error.to_string()))?;

        Ok(Prepared { profile, raws })
    }

    /// The writing half of a run: plans the parsed rows, or imports them
    /// under one batch, by `mode`.
    async fn finish(
        &self,
        prepared: Prepared<'_>,
        mode: Mode,
    ) -> Result<ProfileRun, ProfileFailure> {
        let Prepared { profile, raws } = prepared;
        match mode {
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
            .map(ProfileRun::Planned)
            .map_err(ProfileFailure::from),
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
            .map(ProfileRun::Imported)
            .map_err(ProfileFailure::from),
        }
    }

    /// Runs a selection of profiles in name order, taking one `PreImport`
    /// snapshot before the first write when the policy asks for one, and
    /// continuing past any profile that fails.
    ///
    /// A committed profile stays committed when a later one fails: each has
    /// its own batch, so `import discard` undoes one without the others, and
    /// the report carries the snapshot path for a wholesale restore.
    ///
    /// # Arguments
    ///
    /// * `selection` - One profile by name, or every profile.
    /// * `mode` - Whether to write. [`Mode::DryRun`] never snapshots.
    ///
    /// The snapshot is taken after the first profile parses and before it
    /// writes, so a sweep whose every profile fails to parse leaves nothing
    /// behind.
    ///
    /// # Returns
    ///
    /// One [`ProfileResult`] per profile, in the order they ran.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] when the selection cannot be resolved (a
    /// name that matches no profile, or a profile listing failure) or the
    /// snapshot cannot be written. A profile's own failure is not an error;
    /// it is carried in its [`ProfileResult`].
    #[inline]
    pub async fn sync(&self, selection: Selection, mode: Mode) -> BcResult<SyncReport> {
        let profiles = match selection {
            Selection::One(name) => vec![self.profiles.find_by_name(&name).await?],
            Selection::All => {
                let mut all = self.profiles.list_all().await?;
                all.sort_by(|left, right| left.name.cmp(&right.name));
                all
            }
        };

        // One snapshot for the whole sweep, taken lazily before the first
        // write: a dry run has nothing to protect, and a sweep whose every
        // profile fails to parse would otherwise leave an orphan copy in the
        // retention pool for a run that changed nothing.
        let mut snapshot = None;
        let mut results = Vec::with_capacity(profiles.len());
        for profile in &profiles {
            let result = match self.prepare(profile).await {
                Ok(prepared) => {
                    if mode == Mode::Commit && self.snapshot_before_write && snapshot.is_none() {
                        let record = self.backup.backup(BackupKind::PreImport, None).await?;
                        tracing::info!(path = %record.path.display(), "pre-import snapshot taken");
                        snapshot = Some(record.path);
                    }
                    self.finish(prepared, mode).await
                }
                Err(failure) => Err(failure),
            };
            results.push(ProfileResult {
                profile: ProfileRef::from(profile),
                result,
            });
        }

        Ok(SyncReport {
            snapshot,
            profiles: results,
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
    use pretty_assertions::assert_ne;
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

    /// Panics inside `import`, standing in for a plugin trap that unwinds
    /// instead of returning an error.
    struct PanickingImporter;

    impl Importer for PanickingImporter {
        fn name(&self) -> &'static str {
            "panicking"
        }

        fn import(&self, _config: &ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
            panic!("trap while parsing")
        }

        fn validate(&self, _config: &ImportConfig) -> Result<(), ImportError> {
            Ok(())
        }
    }

    fn make_panicking() -> Box<dyn Importer> {
        Box::new(PanickingImporter)
    }

    /// Everything a test reaches for: the engine, the services it shares,
    /// the pool beneath them, and the directory snapshots land in.
    struct Fixture {
        engine: ImportEngine,
        profiles: crate::ImportProfileService,
        batches: crate::ImportBatchService,
        pool: sqlx::SqlitePool,
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
        importers.register(ImporterFactory::new("panicking", make_panicking));

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
            .backup(Arc::new(crate::BackupService::new(
                pool.clone(),
                db_path,
                policy,
            )))
            .snapshot_before_write(snapshot_before_write)
            .build();
        Fixture {
            engine,
            profiles,
            batches,
            pool,
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
    fn an_engine_failure_displays_its_bare_message() {
        let failure = ProfileFailure::from(BcError::InvalidInput("no such row".to_owned()));
        assert_eq!(failure.stage, FailureStage::Engine);
        assert_eq!(failure.to_string(), "invalid input: no such row");
        assert!(
            failure.source.is_some(),
            "the engine stage keeps its error for the caller's exit-code mapping"
        );
    }

    #[test]
    fn only_the_engine_stage_carries_a_source() {
        let failure = ProfileFailure::new(FailureStage::Importer, "boom");
        assert_eq!(failure.source.map(|error| error.to_string()), None);
    }

    #[test]
    fn failures_compare_by_stage_and_message() {
        let from_error = ProfileFailure::from(BcError::InvalidInput("x".to_owned()));
        let bare = ProfileFailure::new(FailureStage::Engine, "invalid input: x");
        assert_eq!(from_error, bare);
        assert_ne!(
            bare,
            ProfileFailure::new(FailureStage::Importer, "invalid input: x")
        );
    }

    #[test]
    fn failure_stage_labels_are_stable() {
        assert_eq!(FailureStage::UnknownImporter.label(), "unknown_importer");
        assert_eq!(FailureStage::Importer.label(), "importer");
        assert_eq!(FailureStage::Engine.label(), "engine");
    }

    /// Names in the order the report lists them.
    fn names(report: &SyncReport) -> Vec<&str> {
        report
            .profiles
            .iter()
            .map(|result| result.profile.name.as_str())
            .collect()
    }

    #[tokio::test]
    async fn sync_all_orders_profiles_by_name() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), false).await;
        profile(&fixture, "zeta", "stub").await;
        profile(&fixture, "alpha", "stub").await;
        profile(&fixture, "mid", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::DryRun)
            .await
            .expect("sync");

        assert_eq!(names(&report), vec!["alpha", "mid", "zeta"]);
    }

    #[tokio::test]
    async fn sync_all_continues_past_a_failed_profile() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), false).await;
        profile(&fixture, "alpha", "failing").await;
        profile(&fixture, "beta", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        assert_eq!(names(&report), vec!["alpha", "beta"]);
        let alpha = report.profiles.first().expect("alpha ran");
        let beta = report.profiles.get(1).expect("beta ran");
        assert!(matches!(
            alpha.result,
            Err(ProfileFailure {
                stage: FailureStage::Importer,
                ..
            })
        ));
        assert!(matches!(beta.result, Ok(ProfileRun::Imported(_))));
        assert_eq!(fixture.batches.list().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn a_panicking_importer_is_a_failure_at_the_importer_stage() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), false).await;
        let broken = profile(&fixture, "broken", "panicking").await;

        let result = fixture.engine.run_profile(&broken, Mode::Commit).await;

        let failure = result.result.expect_err("the panic is a failure");
        assert_eq!(failure.stage, FailureStage::Importer);
        assert!(
            failure.message.starts_with("importer task failed: "),
            "the join error is the message: {failure:?}"
        );
        assert!(fixture.batches.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn a_run_that_stops_after_opening_names_its_batch() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), false).await;
        profile(&fixture, "alpha", "stub").await;
        profile(&fixture, "beta", "stub").await;
        // Every committed run closes its batch last, so failing the close
        // stops the run after the batch opened and its rows were written.
        sqlx::query(
            "CREATE TRIGGER stop_close BEFORE UPDATE OF finished_at ON import_batches \
             BEGIN SELECT RAISE(ABORT, 'disk full'); END",
        )
        .execute(&fixture.pool)
        .await
        .expect("create trigger");

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        let mut open: Vec<String> = fixture
            .batches
            .list()
            .await
            .expect("list")
            .into_iter()
            .map(|batch| batch.id.to_string())
            .collect();
        open.sort();
        assert_eq!(open.len(), 2, "each profile opened its own batch");
        let mut named = Vec::new();
        for result in &report.profiles {
            let failure = result.result.as_ref().expect_err("the close failed");
            assert_eq!(failure.stage, FailureStage::Engine);
            assert!(
                failure.source.is_some(),
                "the error survives for the exit code: {failure:?}"
            );
            named.push(
                failure
                    .batch_id
                    .as_ref()
                    .expect("the failure names its batch")
                    .to_string(),
            );
        }
        named.sort();
        assert_eq!(named, open, "each failure names the batch it left open");
    }

    #[tokio::test]
    async fn sync_commit_takes_one_snapshot_for_many_profiles() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        profile(&fixture, "a", "stub").await;
        profile(&fixture, "b", "stub").await;
        profile(&fixture, "c", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        assert_eq!(pre_import_snapshots(&fixture.backup_dir), 1);
        assert!(report.snapshot.is_some());
        assert_eq!(fixture.batches.list().await.expect("list").len(), 3);
    }

    #[tokio::test]
    async fn a_failed_parse_takes_no_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        profile(&fixture, "broken", "failing").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        assert!(matches!(
            report.profiles.first().map(|result| &result.result),
            Some(Err(ProfileFailure {
                stage: FailureStage::Importer,
                ..
            }))
        ));
        assert_eq!(report.snapshot, None);
        assert_eq!(
            pre_import_snapshots(&fixture.backup_dir),
            0,
            "a sweep that wrote nothing must not leave an orphan copy in the retention pool"
        );
        assert!(fixture.batches.list().await.expect("list").is_empty());
    }

    #[tokio::test]
    async fn a_sweep_snapshots_once_before_the_first_write() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        profile(&fixture, "a-broken", "failing").await;
        profile(&fixture, "b-good", "stub").await;
        profile(&fixture, "c-good", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        assert_eq!(names(&report), vec!["a-broken", "b-good", "c-good"]);
        assert!(report.snapshot.is_some());
        assert_eq!(
            pre_import_snapshots(&fixture.backup_dir),
            1,
            "the failed first profile defers the snapshot; the two writes share one"
        );
        assert_eq!(fixture.batches.list().await.expect("list").len(), 2);
    }

    #[tokio::test]
    async fn sync_dry_run_takes_no_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        profile(&fixture, "a", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::DryRun)
            .await
            .expect("sync");

        assert_eq!(pre_import_snapshots(&fixture.backup_dir), 0);
        assert_eq!(report.snapshot, None);
    }

    #[tokio::test]
    async fn sync_commit_without_the_setting_takes_no_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), false).await;
        profile(&fixture, "a", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        assert_eq!(pre_import_snapshots(&fixture.backup_dir), 0);
        assert_eq!(report.snapshot, None);
    }

    #[tokio::test]
    async fn sync_commit_over_no_profiles_takes_no_snapshot() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;

        let report = fixture
            .engine
            .sync(Selection::All, Mode::Commit)
            .await
            .expect("sync");

        assert!(report.profiles.is_empty());
        assert_eq!(report.snapshot, None);
        assert_eq!(pre_import_snapshots(&fixture.backup_dir), 0);
    }

    #[tokio::test]
    async fn sync_one_runs_only_that_profile() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;
        profile(&fixture, "alpha", "stub").await;
        profile(&fixture, "beta", "stub").await;

        let report = fixture
            .engine
            .sync(Selection::One("beta".to_owned()), Mode::Commit)
            .await
            .expect("sync");

        assert_eq!(names(&report), vec!["beta"]);
        assert_eq!(pre_import_snapshots(&fixture.backup_dir), 1);
        assert_eq!(fixture.batches.list().await.expect("list").len(), 1);
    }

    #[tokio::test]
    async fn sync_one_with_an_unknown_name_is_an_error() {
        let home = tempfile::tempdir().expect("tempdir");
        let fixture = fixture_in(home.path(), true).await;

        let error = fixture
            .engine
            .sync(Selection::One("nobody".to_owned()), Mode::Commit)
            .await
            .expect_err("no such profile");

        assert!(
            matches!(error, crate::BcError::NotFound(_)),
            "a mistyped name is the caller's error, not a per-profile failure: {error:?}"
        );
        assert_eq!(
            pre_import_snapshots(&fixture.backup_dir),
            0,
            "the lookup fails before the snapshot, so a typo costs nothing"
        );
    }
}
