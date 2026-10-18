//! The sidecar file that maps export handles back to full transactions.

use std::ffi::OsString;
use std::path::Path;
use std::path::PathBuf;

use bc_models::Transaction;
use jiff::Timestamp;
use serde::Deserialize;
use serde::Serialize;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// The sidecar format version this build reads and writes.
pub(super) const VERSION: u32 = 1;

/// The transactions a working export was written from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Sidecar {
    /// The sidecar format version.
    pub version: u32,
    /// Identity of the database the export came from.
    pub database_id: String,
    /// When the export was written.
    pub exported_at: Timestamp,
    /// One entry per exported transaction.
    pub entries: Vec<SidecarEntry>,
}

/// One exported transaction and the handle that names it in the export.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[expect(
    clippy::module_name_repetitions,
    reason = "the type is re-exported at `roundtrip` where the prefix is needed"
)]
#[non_exhaustive]
pub struct SidecarEntry {
    /// The short handle written into the export.
    pub handle: String,
    /// The transaction as it stood at export time.
    pub transaction: Transaction,
}

impl SidecarEntry {
    /// Creates an entry.
    ///
    /// # Arguments
    ///
    /// * `handle` - The short handle written into the export.
    /// * `transaction` - The transaction as it stood at export time.
    #[must_use]
    pub fn new(handle: String, transaction: Transaction) -> Self {
        Self {
            handle,
            transaction,
        }
    }
}

impl Sidecar {
    /// Creates a current-version sidecar.
    ///
    /// # Arguments
    ///
    /// * `database_id` - Identity of the exporting database.
    /// * `exported_at` - When the export was written.
    /// * `entries` - The exported transactions.
    #[must_use]
    pub fn new(database_id: String, exported_at: Timestamp, entries: Vec<SidecarEntry>) -> Self {
        Self {
            version: VERSION,
            database_id,
            exported_at,
            entries,
        }
    }

    /// Returns the sidecar path for an export: `<export>.bc.json`.
    ///
    /// # Arguments
    ///
    /// * `export` - Path of the export file.
    #[must_use]
    pub fn path_for(export: &Path) -> PathBuf {
        let mut name: OsString = export.as_os_str().to_owned();
        name.push(".bc.json");
        PathBuf::from(name)
    }

    /// Reads a sidecar from `path`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the file cannot be read or holds a
    /// sidecar version other than [`VERSION`], and [`BcError::Serialisation`]
    /// if it is not a valid sidecar.
    pub fn read(path: &Path) -> BcResult<Self> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| BcError::BadData(format!("cannot read {}: {e}", path.display())))?;
        // The version is checked before the rest: another version's fields
        // need not decode as this one's.
        let value: serde_json::Value = serde_json::from_str(&text)?;
        let version = value.get("version").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(VERSION)) {
            let found = version.map_or_else(|| "none".to_owned(), |v| v.to_string());
            return Err(BcError::BadData(format!(
                "{} is sidecar version {found}, but this build reads version {VERSION}",
                path.display()
            )));
        }
        Ok(serde_json::from_value(value)?)
    }

    /// Writes the sidecar to `path` as JSON.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Serialisation`] if encoding fails and
    /// [`BcError::BadData`] if the file cannot be written.
    pub fn write(&self, path: &Path) -> BcResult<()> {
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(path, text)
            .map_err(|e| BcError::BadData(format!("cannot write {}: {e}", path.display())))
    }

    /// Finds the entry for `handle`.
    #[must_use]
    pub fn entry(&self, handle: &str) -> Option<&SidecarEntry> {
        self.entries.iter().find(|e| e.handle == handle)
    }
}

/// Returns the identity of the database behind `pool`.
///
/// # Errors
///
/// Returns [`BcError::Database`] if the identity row cannot be read.
pub async fn database_id(pool: &SqlitePool) -> BcResult<String> {
    let id: String = sqlx::query_scalar("SELECT id FROM database_identity")
        .fetch_one(pool)
        .await?;
    Ok(id)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::Reconciliation;
    use bc_models::TransactionId;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn sample() -> Sidecar {
        let transaction = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 2))
            .description("Woolworths")
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::UNIX_EPOCH)
            .build();
        Sidecar::new(
            "abc123".to_owned(),
            Timestamp::UNIX_EPOCH,
            vec![SidecarEntry::new("n02".to_owned(), transaction)],
        )
    }

    #[test]
    fn path_for_appends_the_suffix() {
        assert_eq!(
            Sidecar::path_for(Path::new("/tmp/out.beancount")),
            PathBuf::from("/tmp/out.beancount.bc.json")
        );
    }

    #[test]
    fn sidecar_round_trips_through_json() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x.bc.json");
        let sidecar = sample();
        sidecar.write(&path).expect("write");
        let back = Sidecar::read(&path).expect("read");
        assert_eq!(
            serde_json::to_value(&sidecar).expect("encode"),
            serde_json::to_value(&back).expect("encode")
        );
        assert_eq!(back.version, 1);
        assert!(back.entry("n02").is_some());
        assert!(back.entry("zzz").is_none());
    }

    #[rstest]
    #[case::newer(Some(2), "sidecar version 2, but this build reads version 1")]
    #[case::missing(None, "sidecar version none, but this build reads version 1")]
    fn another_sidecar_version_is_refused(#[case] version: Option<u32>, #[case] expected: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("x.bc.json");
        let mut value = serde_json::to_value(sample()).expect("encode");
        let object = value.as_object_mut().expect("object");
        match version {
            Some(v) => {
                object.insert("version".to_owned(), serde_json::Value::from(v));
            }
            None => {
                object.remove("version");
            }
        }
        std::fs::write(&path, value.to_string()).expect("write");
        let err = Sidecar::read(&path).expect_err("refused");
        assert!(
            matches!(&err, BcError::BadData(message) if message.contains(expected)),
            "got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn database_id_is_stable_across_calls(pool: SqlitePool) {
        let first = database_id(&pool).await.expect("first");
        let second = database_id(&pool).await.expect("second");
        assert_eq!(first, second);
        assert_eq!(first.len(), 32);
    }
}
