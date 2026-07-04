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
    /// Created automatically (pre-migration or pre-restore) (`.automatic`).
    Automatic,
}

impl BackupKind {
    /// Returns the filename suffix for this kind (without dots).
    #[inline]
    #[must_use]
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Automatic => "automatic",
        }
    }

    /// Parses a kind from a filename suffix, if recognised.
    #[inline]
    #[must_use]
    pub fn from_suffix(s: &str) -> Option<Self> {
        match s {
            "manual" => Some(Self::Manual),
            "automatic" => Some(Self::Automatic),
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
const TS_FMT: &str = "%Y%m%d-%H%M%S";

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
    policy: BackupPolicy,
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
            policy,
        }
    }

    /// Returns the live database file path.
    #[inline]
    #[must_use]
    pub fn db_path(&self) -> &Path {
        &self.db_path
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
    /// * `kind` - Manual or automatic (controls the filename suffix).
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
        let now = jiff::Zoned::now();
        let stamp = now.strftime(TS_FMT).to_string();
        let target = if let Some(p) = dest {
            p.to_path_buf()
        } else {
            std::fs::create_dir_all(&self.policy.dir).map_err(|e| io_err(&e))?;
            self.policy
                .dir
                .join(format!("{stamp}.{}.sqlite", kind.suffix()))
        };

        self.vacuum_into(&target).await?;
        if dest.is_none() {
            self.rotate()?;
        }

        let size_bytes = std::fs::metadata(&target).map_err(|e| io_err(&e))?.len();
        let created_at = jiff::civil::DateTime::strptime(TS_FMT, &stamp)
            .map_err(|e| BcError::BadData(format!("bad timestamp: {e}")))?;
        Ok(BackupRecord {
            path: target,
            kind,
            created_at,
            size_bytes,
        })
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
    /// Only files matching `{YYYYMMDD-HHMMSS}.{manual|automatic}.sqlite` in the
    /// policy directory are returned; anything else is ignored.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] if the directory cannot be read.
    #[inline]
    pub fn list(&self) -> BcResult<Vec<BackupRecord>> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.policy.dir) {
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
        std::fs::create_dir_all(&self.policy.dir).map_err(|e| io_err(&e))?;
        let target = self
            .policy
            .dir
            .join(format!("{stamp}.{}.sqlite", kind.suffix()));
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
        let tmp = tempfile::NamedTempFile::new().map_err(|e| io_err(&e))?;
        std::fs::copy(candidate, tmp.path()).map_err(|e| io_err(&e))?;
        let pool = crate::open_db_at(tmp.path()).await?;
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
        let now = jiff::Zoned::now();
        let ages: Vec<i64> = records
            .iter()
            .map(|r| age_days(&now, r.created_at))
            .collect();
        for i in prune_indices(&ages, self.policy.retain_count, self.policy.retain_days) {
            let Some(record) = records.get(i) else {
                continue;
            };
            std::fs::remove_file(&record.path).map_err(|e| io_err(&e))?;
        }
        Ok(())
    }
}

/// Whole-days age of `created_at` (local civil time) relative to `now`.
fn age_days(now: &jiff::Zoned, created_at: jiff::civil::DateTime) -> i64 {
    let Ok(created_zoned) = created_at.to_zoned(now.time_zone().clone()) else {
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
        svc.write_snapshot_for_test(BackupKind::Automatic, "20260101-000000")
            .await
            .expect("snap1");
        svc.write_snapshot_for_test(BackupKind::Manual, "20260601-000000")
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
            BackupKind::Automatic
        );
    }

    #[test]
    fn prune_disabled_when_both_limits_unset() {
        // 4 backups aged 0,10,100,400 days; no limits ⇒ keep all.
        assert_eq!(
            prune_indices(&[0, 10, 100, 400], None, None),
            Vec::<usize>::new()
        );
    }

    #[test]
    fn prune_count_only_keeps_newest_n() {
        // Keep 2 newest ⇒ delete indices 2 and 3 (ages ignored).
        assert_eq!(prune_indices(&[0, 1, 500, 9000], Some(2), None), vec![2, 3]);
    }

    #[test]
    fn prune_age_only_keeps_recent() {
        // Keep < 90 days ⇒ delete the 100- and 400-day-old ones.
        assert_eq!(
            prune_indices(&[0, 10, 100, 400], None, Some(90)),
            vec![2, 3]
        );
    }

    #[test]
    fn prune_union_deletes_only_when_beyond_both() {
        // count=2, days=90. Index 2 (100d) is beyond count AND age ⇒ delete.
        // Index 1 (10d) beyond count? no (i<2). Kept. Index 3 (400d) delete.
        assert_eq!(
            prune_indices(&[0, 10, 100, 400], Some(2), Some(90)),
            vec![2, 3]
        );
    }

    #[test]
    fn prune_union_age_rescues_old_beyond_count() {
        // count=1, days=90: index1 (10d) is beyond count but within age ⇒ kept.
        assert_eq!(prune_indices(&[0, 10, 100], Some(1), Some(90)), vec![2]);
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
        for stamp in ["20260101-000000", "20260201-000000", "20260301-000000"] {
            svc.write_snapshot_for_test(BackupKind::Automatic, stamp)
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

        svc.write_snapshot_for_test(BackupKind::Automatic, "20260101-000000")
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
}
