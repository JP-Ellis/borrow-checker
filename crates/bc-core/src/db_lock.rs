//! Exclusive advisory lock that keeps two long-lived processes off one database.

use std::fs::File;
use std::fs::OpenOptions;
use std::fs::TryLockError;
use std::path::Path;
use std::path::PathBuf;

use crate::BcError;
use crate::BcResult;

/// Holds an exclusive lock on `<db>.lock` until dropped.
///
/// The desktop app and the web server each hold one for their lifetime, and
/// `borrow-checker restore` holds one across its swap: replacing the database
/// file under a live process would send that process's writes to the
/// unlinked file. The kernel releases the lock when the holder exits, crashes
/// included.
#[derive(Debug)]
pub struct DbLock {
    /// The open lock file; the lock lives as long as this handle.
    _file: File,
    /// Path of the lock file.
    path: PathBuf,
}

impl DbLock {
    /// Takes the lock for the database at `db_path`, without waiting.
    ///
    /// # Arguments
    ///
    /// * `db_path` - Path of the SQLite database file.
    ///
    /// # Returns
    ///
    /// The held lock.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::DatabaseInUse`] if another handle holds the lock, or
    /// [`BcError::BadData`] if the lock file cannot be opened or locked.
    #[inline]
    pub fn acquire(db_path: &Path) -> BcResult<Self> {
        let mut name = db_path.file_name().unwrap_or_default().to_os_string();
        name.push(".lock");
        let path = db_path.with_file_name(name);
        let io = |e: std::io::Error| BcError::BadData(format!("lock file {}: {e}", path.display()));
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(io)?;
        match file.try_lock() {
            Ok(()) => Ok(Self { _file: file, path }),
            Err(TryLockError::WouldBlock) => Err(BcError::DatabaseInUse(path)),
            Err(TryLockError::Error(e)) => Err(io(e)),
        }
    }

    /// Returns the path of the lock file.
    #[inline]
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::DbLock;
    use crate::BcError;

    #[test]
    fn second_acquire_fails_while_first_is_held() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("ledger.db");
        let first = DbLock::acquire(&db).expect("first lock");

        let second = DbLock::acquire(&db);

        match second {
            Err(BcError::DatabaseInUse(path)) => assert_eq!(path, first.path()),
            other => panic!("expected DatabaseInUse, got {other:?}"),
        }
    }

    #[test]
    fn acquire_succeeds_after_the_holder_drops() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("ledger.db");
        drop(DbLock::acquire(&db).expect("first lock"));

        DbLock::acquire(&db).expect("second lock, after the first released");
    }

    #[test]
    fn lock_file_sits_beside_the_database() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("ledger.db");

        let lock = DbLock::acquire(&db).expect("lock");

        assert_eq!(lock.path(), dir.path().join("ledger.db.lock"));
    }

    #[test]
    fn in_use_message_names_the_lock_file() {
        let e = BcError::DatabaseInUse("/data/ledger.db.lock".into());
        assert_eq!(
            e.to_string(),
            "database is in use by another BorrowChecker process (lock: /data/ledger.db.lock)"
        );
    }
}
