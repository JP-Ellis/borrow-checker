//! Import batch provenance: one record per import run.

use bc_models::ImportBatchId;
use bc_models::ProfileId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// A record of one import run.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBatch {
    /// Unique identifier for this run.
    pub id: ImportBatchId,
    /// The profile that drove the run, if it was profile-driven.
    pub profile_id: Option<ProfileId>,
    /// Stable identifier of the importer used.
    pub importer: String,
    /// When the run started.
    pub started_at: Timestamp,
    /// Transactions created by this run.
    pub new_transactions: i64,
    /// Postings attached to transactions an earlier run had created.
    pub attached_postings: i64,
    /// Postings the run could not persist (unresolvable account, invalid path).
    pub skipped_postings: i64,
}

/// Service recording import batch provenance.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    ///
    /// # Arguments
    ///
    /// * `pool` - A SQLite connection pool connected to the BorrowChecker database.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Opens a new import batch and returns its ID.
    ///
    /// # Arguments
    ///
    /// * `profile_id` - The profile driving this run, if profile-driven.
    /// * `importer` - Stable identifier of the importer used for this run.
    ///
    /// # Returns
    ///
    /// The [`ImportBatchId`] of the newly opened batch.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on database insert failure.
    #[inline]
    pub async fn open(
        &self,
        profile_id: Option<&ProfileId>,
        importer: &str,
    ) -> BcResult<ImportBatchId> {
        let id = ImportBatchId::new();
        let started_at = Timestamp::now();

        sqlx::query(
            "INSERT INTO import_batches \
             (id, profile_id, importer, started_at, new_transactions, attached_postings, \
              skipped_postings) \
             VALUES (?, ?, ?, ?, 0, 0, 0)",
        )
        .bind(id.to_string())
        .bind(profile_id.map(ToString::to_string))
        .bind(importer)
        .bind(started_at.to_string())
        .execute(&self.pool)
        .await?;

        tracing::info!(batch_id = %id, %importer, "import batch opened");
        Ok(id)
    }

    /// Records the final counts for a completed import run.
    ///
    /// # Arguments
    ///
    /// * `id` - The batch to close.
    /// * `new_transactions` - Transactions created by this run.
    /// * `attached_postings` - Postings attached to transactions an earlier run created.
    /// * `skipped_postings` - Postings the run could not persist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a count exceeds `i64::MAX`.
    /// Returns [`BcError::NotFound`] if no batch with that ID exists.
    /// Returns [`BcError::Database`] on database update failure.
    #[inline]
    pub async fn close(
        &self,
        id: &ImportBatchId,
        new_transactions: usize,
        attached_postings: usize,
        skipped_postings: usize,
    ) -> BcResult<()> {
        let new_transactions_count = i64::try_from(new_transactions)
            .map_err(|_err| BcError::BadData("import count exceeds i64".into()))?;
        let attached_postings_count = i64::try_from(attached_postings)
            .map_err(|_err| BcError::BadData("import count exceeds i64".into()))?;
        let skipped_postings_count = i64::try_from(skipped_postings)
            .map_err(|_err| BcError::BadData("import count exceeds i64".into()))?;

        let result = sqlx::query(
            "UPDATE import_batches \
             SET new_transactions = ?, attached_postings = ?, skipped_postings = ? \
             WHERE id = ?",
        )
        .bind(new_transactions_count)
        .bind(attached_postings_count)
        .bind(skipped_postings_count)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(format!("import batch {id}")));
        }

        tracing::info!(
            batch_id = %id,
            new_transactions,
            attached_postings,
            skipped_postings,
            "import batch closed"
        );
        Ok(())
    }

    /// Finds an import batch by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The [`ImportBatchId`] to look up.
    ///
    /// # Returns
    ///
    /// The [`ImportBatch`] if found.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no batch with that ID exists.
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    /// Returns [`BcError::Database`] on database query failure.
    #[inline]
    pub async fn find_by_id(&self, id: &ImportBatchId) -> BcResult<ImportBatch> {
        let row: (String, Option<String>, String, String, i64, i64, i64) = sqlx::query_as(
            "SELECT id, profile_id, importer, started_at, new_transactions, attached_postings, \
                    skipped_postings \
             FROM import_batches WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(format!("import batch {id}")))?;

        parse_row(&row.0, row.1, row.2, &row.3, row.4, row.5, row.6)
    }
}

/// Parses a raw `import_batches` row into an [`ImportBatch`].
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any ID or timestamp string is malformed.
#[inline]
fn parse_row(
    raw_id: &str,
    raw_profile_id: Option<String>,
    importer: String,
    raw_started_at: &str,
    new_transactions: i64,
    attached_postings: i64,
    skipped_postings: i64,
) -> BcResult<ImportBatch> {
    let id = raw_id
        .parse::<ImportBatchId>()
        .map_err(|e: bc_models::IdParseError| BcError::BadData(e.to_string()))?;

    let profile_id = raw_profile_id
        .map(|raw| {
            raw.parse::<ProfileId>()
                .map_err(|e: bc_models::IdParseError| BcError::BadData(e.to_string()))
        })
        .transpose()?;

    let started_at = raw_started_at
        .parse::<Timestamp>()
        .map_err(|e| BcError::BadData(e.to_string()))?;

    Ok(ImportBatch {
        id,
        profile_id,
        importer,
        started_at,
        new_transactions,
        attached_postings,
        skipped_postings,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use sqlx::SqlitePool;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn open_then_close_records_the_counts(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc.open(None, "csv").await.expect("open batch");

        svc.close(&id, 12, 3, 5).await.expect("close batch");

        let batch = svc.find_by_id(&id).await.expect("find batch");
        assert_eq!(batch.importer, "csv");
        assert_eq!(batch.new_transactions, 12);
        assert_eq!(batch.attached_postings, 3);
        assert_eq!(batch.skipped_postings, 5);
        assert_eq!(batch.profile_id, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_freshly_opened_batch_has_zero_counts(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc.open(None, "ledger").await.expect("open batch");
        let batch = svc.find_by_id(&id).await.expect("find batch");
        assert_eq!(batch.new_transactions, 0);
        assert_eq!(batch.attached_postings, 0);
        assert_eq!(batch.skipped_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_on_an_unknown_batch_is_not_found(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let result = svc.close(&bc_models::ImportBatchId::new(), 0, 0, 0).await;
        assert!(matches!(result, Err(crate::BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_batch_records_its_profile(pool: SqlitePool) {
        let profiles = crate::ImportProfileService::new(pool.clone());
        let profile_id = profiles
            .create("Bank", "csv", crate::ImportConfig::default())
            .await
            .expect("create profile");

        let svc = Service::new(pool.clone());
        let id = svc
            .open(Some(&profile_id), "csv")
            .await
            .expect("open batch");
        let batch = svc.find_by_id(&id).await.expect("find batch");
        assert_eq!(batch.profile_id, Some(profile_id));
    }
}
