//! Database backup: `VACUUM INTO` snapshots with conservative rotation.

use std::path::Path;
use std::path::PathBuf;

use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// The origin of a backup, encoded in its filename suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BackupKind is the clearest public name for this type; re-exported at the crate root"
)]
pub enum BackupKind {
    /// Created explicitly by the user (`.manual`).
    Manual,
    /// Created automatically before running database migrations (`.pre-migration`).
    PreMigration,
    /// Created automatically as a safety snapshot just before a restore swap (`.pre-restore`).
    PreRestore,
    /// Created automatically before an import run (`.pre-import`).
    PreImport,
}

impl BackupKind {
    /// Returns the filename suffix for this kind (without dots).
    #[inline]
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::PreMigration => "pre-migration",
            Self::PreRestore => "pre-restore",
            Self::PreImport => "pre-import",
        }
    }

    /// Parses a kind from a filename suffix, if recognised.
    #[inline]
    #[must_use]
    pub fn from_suffix(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Self::Manual),
            "pre-migration" => Some(Self::PreMigration),
            "pre-restore" => Some(Self::PreRestore),
            "pre-import" => Some(Self::PreImport),
            _ => None,
        }
    }
}

/// Metadata about a single backup file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BackupRecord is the clearest public name for this type; re-exported at the crate root"
)]
pub struct BackupRecord {
    /// Absolute path to the backup file.
    pub path: PathBuf,
    /// Whether the backup was created manually or automatically.
    pub kind: BackupKind,
    /// The creation timestamp parsed from the filename (local civil time).
    pub created_at: jiff::civil::DateTime,
    /// File size in bytes.
    pub size_bytes: u64,
}

/// Runtime backup and rotation policy (translated from `bc_config::BackupSection`).
#[derive(Debug, Clone)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BackupPolicy is the clearest public name for this type; re-exported at the crate root"
)]
pub struct BackupPolicy {
    /// Directory backups are written to and rotated within.
    pub dir: PathBuf,
    /// "Keep N newest" retention limit.
    pub retain_count: Option<u32>,
    /// "Keep newer than N days" retention limit.
    pub retain_days: Option<u32>,
    /// Whether to snapshot automatically before migrations.
    pub auto_pre_migration: bool,
}

impl BackupPolicy {
    /// Creates a new backup policy.
    ///
    /// # Arguments
    ///
    /// * `dir` - Directory backups live in.
    /// * `retain_count` - "Keep N newest" limit, or `None`.
    /// * `retain_days` - "Keep newer than N days" limit, or `None`.
    /// * `auto_pre_migration` - Whether pre-migration snapshots are enabled.
    #[inline]
    #[must_use]
    pub fn new(
        dir: PathBuf,
        retain_count: Option<u32>,
        retain_days: Option<u32>,
        auto_pre_migration: bool,
    ) -> Self {
        Self {
            dir,
            retain_count,
            retain_days,
            auto_pre_migration,
        }
    }
}

/// Decides which backups to prune under the conservative union policy.
///
/// `ages_days` MUST be sorted newest-first (ascending age). A backup is kept if
/// it is among the `retain_count` newest **or** newer than `retain_days`; it is
/// pruned only if it satisfies neither. When both limits are `None`, nothing is
/// pruned.
///
/// # Arguments
///
/// * `ages_days` - Age of each backup in whole days, newest-first.
/// * `retain_count` - "Keep N newest" limit.
/// * `retain_days` - "Keep newer than N days" limit.
///
/// # Returns
///
/// The indices (into `ages_days`) of backups to delete.
#[must_use]
pub fn prune_indices(
    ages_days: &[i64],
    retain_count: Option<u32>,
    retain_days: Option<u32>,
) -> Vec<usize> {
    if retain_count.is_none() && retain_days.is_none() {
        return Vec::new();
    }
    ages_days
        .iter()
        .enumerate()
        .filter_map(|(i, &age)| {
            #[expect(
                clippy::as_conversions,
                reason = "index i is a small slice position; u64::try_from would be fallible for no practical benefit here"
            )]
            let within_count = retain_count.is_some_and(|n| (i as u64) < u64::from(n));
            let within_age = retain_days.is_some_and(|d| age < i64::from(d));
            (!(within_count || within_age)).then_some(i)
        })
        .collect()
}

/// Timestamp format used in backup filenames.
///
/// Millisecond precision (`%3f`, three digits, no separator) gives filenames
/// sub-second uniqueness so two snapshots taken within the same second do not
/// collide, while the same format string round-trips through `strptime` to
/// recover `created_at`.
const TS_FMT: &str = "%Y%m%d-%H%M%S%3f";

/// Maps an I/O error into a [`BcError`].
fn io_err(e: &std::io::Error) -> BcError {
    BcError::InvalidInput(e.to_string())
}

/// Backup service: snapshots the SQLite database and rotates old snapshots.
///
/// Unlike the projection services this owns the database **file path** (not just
/// the pool) because `VACUUM INTO` writes a sibling file and rotation manages the
/// backup directory.
#[non_exhaustive]
pub struct Service {
    /// Live pool used to run `VACUUM INTO`.
    pool: SqlitePool,
    /// Path of the live database file (used by callers doing restore swaps).
    db_path: PathBuf,
    /// Backup directory and retention policy.
    ///
    /// Behind a [`std::sync::Mutex`] so the policy can be hot-reloaded (e.g.
    /// after the user saves new backup settings) without rebuilding the
    /// service. The lock is only ever held long enough to clone the policy out
    /// or replace it — never across an `.await`.
    policy: std::sync::Mutex<BackupPolicy>,
}

impl Service {
    /// Creates a new backup service.
    ///
    /// # Arguments
    ///
    /// * `pool` - Live connection pool to the database being backed up.
    /// * `db_path` - Filesystem path of the live database file.
    /// * `policy` - Backup directory and retention policy.
    #[inline]
    #[must_use]
    pub fn new(pool: SqlitePool, db_path: PathBuf, policy: BackupPolicy) -> Self {
        Self {
            pool,
            db_path,
            policy: std::sync::Mutex::new(policy),
        }
    }

    /// Returns the live database file path.
    #[inline]
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    /// Returns a clone of the current backup policy.
    ///
    /// Recovers transparently from a poisoned lock (a panic while the policy
    /// was held would only ever have occurred mid-clone/replace, so the inner
    /// value is always consistent).
    ///
    /// # Returns
    ///
    /// The currently active [`BackupPolicy`].
    #[inline]
    #[must_use]
    pub fn current_policy(&self) -> BackupPolicy {
        self.policy
            .lock()
            .map_or_else(|e| e.into_inner().clone(), |g| g.clone())
    }

    /// Replaces the in-memory backup policy.
    ///
    /// Used to hot-reload retention/directory settings after the user saves new
    /// backup configuration, so subsequent [`backup`](Self::backup),
    /// [`list`](Self::list) and [`rotate`](Self::rotate) calls observe the new
    /// policy without restarting.
    ///
    /// # Arguments
    ///
    /// * `policy` - The new policy to apply from now on.
    #[inline]
    pub fn set_policy(&self, policy: BackupPolicy) {
        match self.policy.lock() {
            Ok(mut guard) => *guard = policy,
            Err(poisoned) => *poisoned.into_inner() = policy,
        }
    }

    /// Closes the underlying connection pool.
    ///
    /// `SqlitePool` clones share the same underlying state, so closing this
    /// service's clone closes the pool for the whole process. Callers use this
    /// before an in-place restore swap so that no WAL connection remains that
    /// could checkpoint stale frames onto the restored file.
    #[inline]
    pub async fn close_pool(&self) {
        self.pool.close().await;
    }

    /// Atomically swaps a validated backup file in as the live database,
    /// clearing stale WAL sidecars first.
    ///
    /// The database is opened in WAL mode everywhere, so a `{db_path}-wal` /
    /// `{db_path}-shm` pair left over from the database being replaced would be
    /// replayed by SQLite's recovery on the next open, silently corrupting the
    /// freshly restored file. This removes those sidecars before installing
    /// `candidate` (itself a standalone `VACUUM INTO` snapshot with no sidecars
    /// of its own).
    ///
    /// The candidate is copied to a sibling temp file on the same filesystem and
    /// then atomically renamed over `db_path`. If the copy fails part-way,
    /// `db_path` is left untouched (the original database), never half-written,
    /// so an interrupted restore cannot corrupt the live database.
    ///
    /// The caller MUST ensure no live connection holds the database (see
    /// [`close_pool`](Self::close_pool)); the GUI performs the swap before any
    /// pool is opened.
    ///
    /// # Arguments
    ///
    /// * `candidate` - Path to the validated standalone backup to swap in.
    /// * `db_path` - Path of the live database file to overwrite.
    ///
    /// # Returns
    ///
    /// `()` on a successful swap.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if a sidecar cannot be removed (other than being
    /// absent) or the copy or rename fails.
    #[inline]
    pub fn swap_in(candidate: &Path, db_path: &Path) -> BcResult<()> {
        for suffix in ["-wal", "-shm"] {
            let mut name = db_path.as_os_str().to_os_string();
            name.push(suffix);
            let sidecar = PathBuf::from(name);
            match std::fs::remove_file(&sidecar) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(io_err(&e)),
            }
        }
        let tmp = {
            let mut name = db_path.as_os_str().to_os_string();
            name.push(".restore-tmp");
            PathBuf::from(name)
        };
        drop(std::fs::remove_file(&tmp));
        std::fs::copy(candidate, &tmp).map_err(|e| io_err(&e))?;
        std::fs::rename(&tmp, db_path).map_err(|e| io_err(&e))?;
        Ok(())
    }

    /// Creates a consistent snapshot of the database.
    ///
    /// Writes via `VACUUM INTO` to a temporary file, then atomically renames it
    /// into place. When `dest` is `None` the snapshot lands in the managed backup
    /// directory with a timestamped name and rotation is applied afterwards;
    /// when `dest` is `Some`, it is written exactly there and rotation is
    /// skipped.
    ///
    /// # Arguments
    ///
    /// * `kind` - Manual, pre-migration, or pre-restore (controls the filename suffix).
    /// * `dest` - Explicit output path, or `None` for the managed directory.
    ///
    /// # Returns
    ///
    /// A [`BackupRecord`] describing the written file.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the directory cannot be created, `VACUUM INTO`
    /// fails, or the rename fails.
    #[inline]
    pub async fn backup(&self, kind: BackupKind, dest: Option<&Path>) -> BcResult<BackupRecord> {
        if let Some(target) = dest {
            let stamp = jiff::Zoned::now().strftime(TS_FMT).to_string();
            self.vacuum_into(target).await?;
            return record_from(target.to_path_buf(), kind, &stamp);
        }
        let record = self.write_managed_snapshot(kind).await?;
        self.rotate()?;
        Ok(record)
    }

    /// Writes a pre-restore safety snapshot into the managed backup directory
    /// WITHOUT applying rotation.
    ///
    /// Rotation is deliberately skipped: a restore's candidate is itself a file
    /// in the managed directory, and rotating while inserting this snapshot could
    /// prune the very backup being restored. If the directory sits at
    /// `retain_count` and the user restores the oldest managed backup, a rotating
    /// snapshot would push the count over the limit and delete that oldest file —
    /// the candidate — before the swap ever runs. Skipping rotation keeps the
    /// candidate intact; any resulting over-count is reconciled by the next
    /// ordinary [`backup`](Self::backup) call.
    ///
    /// # Returns
    ///
    /// A [`BackupRecord`] describing the written safety snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the directory cannot be created, `VACUUM INTO`
    /// fails, or the rename fails.
    #[inline]
    pub async fn pre_restore_snapshot(&self) -> BcResult<BackupRecord> {
        self.write_managed_snapshot(BackupKind::PreRestore).await
    }

    /// Writes a timestamped snapshot into the managed backup directory and
    /// returns its record, WITHOUT applying rotation.
    async fn write_managed_snapshot(&self, kind: BackupKind) -> BcResult<BackupRecord> {
        let stamp = jiff::Zoned::now().strftime(TS_FMT).to_string();
        let policy = self.current_policy();
        std::fs::create_dir_all(&policy.dir).map_err(|e| io_err(&e))?;
        let target = policy.dir.join(format!("{stamp}.{}.sqlite", kind.suffix()));
        self.vacuum_into(&target).await?;
        record_from(target, kind, &stamp)
    }

    /// Runs `VACUUM INTO` to `target` via a temp file + atomic rename.
    async fn vacuum_into(&self, target: &Path) -> BcResult<()> {
        let tmp = target.with_extension("tmp");
        // A stale temp from an interrupted run would make VACUUM INTO fail.
        drop(std::fs::remove_file(&tmp));
        // VACUUM INTO takes a string literal; single-quote-escape the path.
        let escaped = tmp.to_string_lossy().replace('\'', "''");
        // The escaped path is the only interpolated value; it is not user-controlled
        // SQL, so asserting safety here is sound.
        sqlx::query(sqlx::AssertSqlSafe(format!("VACUUM INTO '{escaped}'")))
            .execute(&self.pool)
            .await?;
        std::fs::rename(&tmp, target).map_err(|e| io_err(&e))?;
        Ok(())
    }

    /// Lists the managed backups, newest-first.
    ///
    /// Only files matching `{YYYYMMDD-HHMMSS}.{manual|pre-migration|pre-restore}.sqlite`
    /// in the policy directory are returned; anything else is ignored.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the directory cannot be read.
    #[inline]
    pub fn list(&self) -> BcResult<Vec<BackupRecord>> {
        let mut out = Vec::new();
        let dir = self.current_policy().dir;
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            // A not-yet-created backup dir simply has no backups.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(io_err(&e)),
        };
        for entry_result in entries {
            let dir_entry = entry_result.map_err(|e| io_err(&e))?;
            let path = dir_entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if let Some(rec) = parse_record(&path, name) {
                out.push(rec);
            }
        }
        out.sort_by_key(|rec| core::cmp::Reverse(rec.created_at));
        Ok(out)
    }

    /// Test-only helper: write a snapshot with an explicit timestamp string.
    #[cfg(test)]
    async fn write_snapshot_for_test(&self, kind: BackupKind, stamp: &str) -> BcResult<()> {
        let dir = self.current_policy().dir;
        std::fs::create_dir_all(&dir).map_err(|e| io_err(&e))?;
        let target = dir.join(format!("{stamp}.{}.sqlite", kind.suffix()));
        self.vacuum_into(&target).await
    }

    /// Validates that `candidate` is a real, migratable BorrowChecker database.
    ///
    /// Copies the file to a temporary location and opens it (which runs
    /// migrations) so the caller's file is never mutated by validation. A
    /// sentinel query confirms the schema is present.
    ///
    /// # Arguments
    ///
    /// * `candidate` - Path to the backup file to validate.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the file cannot be copied, opened, migrated, or is
    /// missing the expected schema.
    #[inline]
    pub async fn validate(candidate: &Path) -> BcResult<()> {
        let dir = tempfile::TempDir::new().map_err(|e| io_err(&e))?;
        let tmp = dir.path().join("candidate.sqlite");
        std::fs::copy(candidate, &tmp).map_err(|e| io_err(&e))?;
        let pool = crate::open_db_at(&tmp).await?;
        // Sentinel: the events table must exist in any real BorrowChecker DB.
        sqlx::query("SELECT count(*) FROM events")
            .fetch_one(&pool)
            .await
            .map_err(|e| BcError::BadData(format!("not a BorrowChecker database: {e}")))?;
        pool.close().await;
        Ok(())
    }

    /// Applies the retention policy, deleting backups that satisfy neither the
    /// count nor the age limit (see [`prune_indices`]).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the directory cannot be read or a file cannot be
    /// deleted.
    #[inline]
    pub fn rotate(&self) -> BcResult<()> {
        let records = self.list()?;
        let policy = self.current_policy();
        let now = jiff::Zoned::now();
        let ages: Vec<i64> = records
            .iter()
            .map(|r| age_days(&now, r.created_at))
            .collect();
        for i in prune_indices(&ages, policy.retain_count, policy.retain_days) {
            let Some(record) = records.get(i) else {
                continue;
            };
            std::fs::remove_file(&record.path).map_err(|e| io_err(&e))?;
        }
        Ok(())
    }
}

/// Whole-days age of `created_at` (local civil time) relative to `now`.
///
/// If `created_at` cannot be resolved to a zoned instant (e.g. it falls in a DST
/// gap) this returns `0` — treating the backup as newest so the age rule never
/// prunes it. This is a deliberate keep-on-uncertainty policy: an undatable
/// backup is never a prune candidate. The failure is logged at `warn`.
fn age_days(now: &jiff::Zoned, created_at: jiff::civil::DateTime) -> i64 {
    let Ok(created_zoned) = created_at.to_zoned(now.time_zone().clone()) else {
        tracing::warn!(
            %created_at,
            "could not resolve backup timestamp to a zoned instant; treating as newest to avoid pruning an undatable backup"
        );
        return 0;
    };
    let secs = now
        .timestamp()
        .duration_since(created_zoned.timestamp())
        .as_secs();
    #[expect(
        clippy::integer_division,
        clippy::integer_division_remainder_used,
        reason = "converting whole seconds to whole days is an intentional floor division"
    )]
    let days = secs / 86_400;
    days
}

/// Builds a [`BackupRecord`] for a just-written file from its path, kind and the
/// timestamp string used in its name.
fn record_from(path: PathBuf, kind: BackupKind, stamp: &str) -> BcResult<BackupRecord> {
    let size_bytes = std::fs::metadata(&path).map_err(|e| io_err(&e))?.len();
    let created_at = jiff::civil::DateTime::strptime(TS_FMT, stamp)
        .map_err(|e| BcError::BadData(format!("bad timestamp: {e}")))?;
    Ok(BackupRecord {
        path,
        kind,
        created_at,
        size_bytes,
    })
}

/// Parses a [`BackupRecord`] from a filename of the managed form.
fn parse_record(path: &Path, name: &str) -> Option<BackupRecord> {
    let rest = name.strip_suffix(".sqlite")?;
    let (stamp, suffix) = rest.rsplit_once('.')?;
    let kind = BackupKind::from_suffix(suffix)?;
    let created_at = jiff::civil::DateTime::strptime(TS_FMT, stamp).ok()?;
    let size_bytes = std::fs::metadata(path).ok()?.len();
    Some(BackupRecord {
        path: path.to_path_buf(),
        kind,
        created_at,
        size_bytes,
    })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rstest::rstest;

    use super::prune_indices;
    use crate::BackupKind;
    use crate::BackupPolicy;

    /// Builds a Service over a fresh on-disk DB in `dir`, returning (service, `db_path`).
    async fn service_in(dir: &std::path::Path) -> (super::Service, PathBuf) {
        let db_path = dir.join("db.sqlite");
        let pool = crate::open_db_at(&db_path).await.expect("open db");
        let policy = BackupPolicy::new(dir.join("backups"), Some(5), None, true);
        (super::Service::new(pool, db_path.clone(), policy), db_path)
    }

    #[test]
    fn pre_import_kind_round_trips_through_its_suffix() {
        assert_eq!(BackupKind::PreImport.suffix(), "pre-import");
        assert_eq!(
            BackupKind::from_suffix("pre-import"),
            Some(BackupKind::PreImport)
        );
    }

    #[tokio::test]
    async fn backup_produces_openable_copy() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (svc, _db) = service_in(dir.path()).await;

        let rec = svc.backup(BackupKind::Manual, None).await.expect("backup");
        assert!(rec.path.exists(), "backup file should exist");
        assert_eq!(rec.kind, BackupKind::Manual);
        assert!(rec.size_bytes > 0);

        // The copy opens and has the schema (events table exists).
        let pool2 = crate::open_db_at(&rec.path).await.expect("reopen backup");
        let n: (i64,) = sqlx::query_as("SELECT count(*) FROM events")
            .fetch_one(&pool2)
            .await
            .expect("events table present in backup");
        assert_eq!(n.0, 0);
    }

    #[tokio::test]
    async fn list_returns_backups_newest_first() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (svc, _db) = service_in(dir.path()).await;
        // Two backups with distinct filenames (inject timestamps directly).
        svc.write_snapshot_for_test(BackupKind::PreMigration, "20260101-000000000")
            .await
            .expect("snap1");
        svc.write_snapshot_for_test(BackupKind::Manual, "20260601-000000000")
            .await
            .expect("snap2");

        let list = svc.list().expect("list");
        assert_eq!(list.len(), 2);
        assert_eq!(
            list.first().expect("first backup").kind,
            BackupKind::Manual,
            "June backup is newest"
        );
        assert_eq!(
            list.get(1).expect("second backup").kind,
            BackupKind::PreMigration
        );
    }

    /// Retention policy cases for [`prune_indices`], covering both limits unset,
    /// count-only, age-only, and the two union scenarios (delete only when a
    /// backup is beyond both limits, and the age rule rescuing an old backup that
    /// is beyond the count limit).
    #[rstest]
    // both limits unset ⇒ keep all.
    #[case(&[0, 10, 100, 400], None, None, vec![])]
    // keep 2 newest ⇒ delete indices 2 and 3 (ages ignored).
    #[case(&[0, 1, 500, 9000], Some(2), None, vec![2, 3])]
    // keep < 90 days ⇒ delete the 100- and 400-day-old ones.
    #[case(&[0, 10, 100, 400], None, Some(90), vec![2, 3])]
    // count=2, days=90: only backups beyond BOTH limits are deleted.
    #[case(&[0, 10, 100, 400], Some(2), Some(90), vec![2, 3])]
    // count=1, days=90: index 1 (10d) is beyond count but within age ⇒ kept.
    #[case(&[0, 10, 100], Some(1), Some(90), vec![2])]
    fn prune_indices_cases(
        #[case] ages_days: &[i64],
        #[case] retain_count: Option<u32>,
        #[case] retain_days: Option<u32>,
        #[case] expected: Vec<usize>,
    ) {
        assert_eq!(
            prune_indices(ages_days, retain_count, retain_days),
            expected
        );
    }

    #[tokio::test]
    async fn validate_accepts_real_backup_rejects_garbage() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (svc, _db) = service_in(dir.path()).await;
        let rec = svc.backup(BackupKind::Manual, None).await.expect("backup");
        super::Service::validate(&rec.path)
            .await
            .expect("valid backup ok");

        let junk = dir.path().join("junk.sqlite");
        std::fs::write(&junk, b"not a database").expect("write junk");
        assert!(
            super::Service::validate(&junk).await.is_err(),
            "garbage file must be rejected"
        );
    }

    #[tokio::test]
    async fn rotate_keeps_newest_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("db.sqlite");
        let pool = crate::open_db_at(&db_path).await.expect("open");
        // retain_count = 2, no age limit.
        let policy = BackupPolicy::new(dir.path().join("backups"), Some(2), None, true);
        let svc = super::Service::new(pool, db_path, policy);
        for stamp in [
            "20260101-000000000",
            "20260201-000000000",
            "20260301-000000000",
        ] {
            svc.write_snapshot_for_test(BackupKind::PreMigration, stamp)
                .await
                .expect("snap");
        }
        svc.rotate().expect("rotate");
        let list = svc.list().expect("list");
        assert_eq!(list.len(), 2, "only the 2 newest survive");
        assert_eq!(
            list.first().expect("first backup").created_at.to_string(),
            "2026-03-01T00:00:00"
        );
        assert_eq!(
            list.get(1).expect("second backup").created_at.to_string(),
            "2026-02-01T00:00:00"
        );
    }

    #[tokio::test]
    async fn backup_rotates_when_over_managed_count_limit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("db.sqlite");
        let pool = crate::open_db_at(&db_path).await.expect("open db");
        let policy = BackupPolicy::new(dir.path().join("backups"), Some(1), None, true);
        let svc = super::Service::new(pool, db_path, policy);

        svc.write_snapshot_for_test(BackupKind::PreMigration, "20260101-000000000")
            .await
            .expect("snap1");
        svc.backup(BackupKind::Manual, None).await.expect("backup2");

        let list = svc.list().expect("list");
        assert_eq!(
            list.len(),
            1,
            "managed backup() call should trigger rotation"
        );
        assert_eq!(
            list.first().expect("first backup").kind,
            BackupKind::Manual,
            "newest backup survives"
        );
    }

    #[tokio::test]
    async fn pre_restore_snapshot_does_not_rotate_out_candidate() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("db.sqlite");
        let pool = crate::open_db_at(&db_path).await.expect("open db");
        // retain_count = 1: an ordinary managed backup() would rotate away the
        // pre-existing file, but the safety snapshot must not.
        let policy = BackupPolicy::new(dir.path().join("backups"), Some(1), None, true);
        let svc = super::Service::new(pool, db_path, policy);

        let b1 = svc
            .write_managed_snapshot(BackupKind::Manual)
            .await
            .expect("b1");
        svc.pre_restore_snapshot().await.expect("snapshot");

        assert!(
            b1.path.exists(),
            "candidate B1 must survive: pre_restore_snapshot must not rotate"
        );
        let list = svc.list().expect("list");
        assert_eq!(list.len(), 2, "both managed files present, no rotation");
    }

    #[test]
    fn swap_in_leaves_db_intact_when_candidate_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("db.sqlite");
        std::fs::write(&db_path, b"original database bytes").expect("seed db");
        let missing = dir.path().join("does-not-exist.sqlite");

        assert!(
            super::Service::swap_in(&missing, &db_path).is_err(),
            "swap_in must fail when the candidate does not exist"
        );
        let after = std::fs::read(&db_path).expect("db still readable");
        assert_eq!(
            after, b"original database bytes",
            "failed swap must leave the original db untouched, never truncated"
        );
    }

    #[tokio::test]
    async fn back_to_back_managed_backups_get_distinct_filenames() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (svc, _db) = service_in(dir.path()).await;

        // No injected timestamps: rely on sub-second filename uniqueness.
        svc.backup(BackupKind::Manual, None).await.expect("b1");
        svc.backup(BackupKind::Manual, None).await.expect("b2");

        let list = svc.list().expect("list");
        assert_eq!(list.len(), 2, "two distinct managed backups must be listed");
        assert_ne!(
            list.first().expect("first").path,
            list.get(1).expect("second").path,
            "back-to-back backups must have distinct filenames"
        );
    }
}
