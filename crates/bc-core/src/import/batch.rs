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
    /// Postings the run could not persist, whatever the cause.
    pub skipped_postings: i64,
    /// The subset of [`Self::skipped_postings`] whose account path named no
    /// existing account.
    pub unresolved_path_postings: i64,
}

/// The final tallies of one import run, as [`Service::close`] records them.
///
/// Skips are carried by cause rather than as one total, so a report can
/// attribute each number: an unresolved account path is actionable (create the
/// account and re-run), everything else was warned about individually.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Transactions created by the run.
    pub new_transactions: usize,
    /// Postings attached to transactions an earlier run created.
    pub attached_postings: usize,
    /// Postings skipped because their account path named no existing account.
    pub unresolved_path_postings: usize,
    /// Postings skipped for any other reason.
    pub other_skipped_postings: usize,
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
              skipped_postings, unresolved_path_postings) \
             VALUES (?, ?, ?, ?, 0, 0, 0, 0)",
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
    /// * `counts` - The run's final tallies.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a count exceeds `i64::MAX`.
    /// Returns [`BcError::NotFound`] if no batch with that ID exists.
    /// Returns [`BcError::Database`] on database update failure.
    #[inline]
    pub async fn close(&self, id: &ImportBatchId, counts: Counts) -> BcResult<()> {
        let skipped_postings = counts
            .unresolved_path_postings
            .saturating_add(counts.other_skipped_postings);
        let to_i64 = |value: usize| {
            i64::try_from(value).map_err(|_err| BcError::BadData("import count exceeds i64".into()))
        };

        let result = sqlx::query(
            "UPDATE import_batches \
             SET new_transactions = ?, attached_postings = ?, skipped_postings = ?, \
                 unresolved_path_postings = ? \
             WHERE id = ?",
        )
        .bind(to_i64(counts.new_transactions)?)
        .bind(to_i64(counts.attached_postings)?)
        .bind(to_i64(skipped_postings)?)
        .bind(to_i64(counts.unresolved_path_postings)?)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(format!("import batch {id}")));
        }

        tracing::info!(
            batch_id = %id,
            new_transactions = counts.new_transactions,
            attached_postings = counts.attached_postings,
            skipped_postings,
            unresolved_path_postings = counts.unresolved_path_postings,
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
        let row: Row = sqlx::query_as(
            "SELECT id, profile_id, importer, started_at, new_transactions, attached_postings, \
                    skipped_postings, unresolved_path_postings \
             FROM import_batches WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(format!("import batch {id}")))?;

        parse_row(row)
    }
}

/// Raw `import_batches` row tuple, mirroring the `SELECT` column list.
type Row = (String, Option<String>, String, String, i64, i64, i64, i64);

/// Parses a raw `import_batches` row into an [`ImportBatch`].
///
/// # Arguments
///
/// * `row` - The raw row, in `SELECT` column order.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any ID or timestamp string is malformed.
#[inline]
fn parse_row(row: Row) -> BcResult<ImportBatch> {
    let (
        raw_id,
        raw_profile_id,
        importer,
        raw_started_at,
        new_transactions,
        attached_postings,
        skipped_postings,
        unresolved_path_postings,
    ) = row;

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
        unresolved_path_postings,
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

        svc.close(
            &id,
            Counts {
                new_transactions: 12,
                attached_postings: 3,
                unresolved_path_postings: 4,
                other_skipped_postings: 1,
            },
        )
        .await
        .expect("close batch");

        let batch = svc.find_by_id(&id).await.expect("find batch");
        assert_eq!(batch.importer, "csv");
        assert_eq!(batch.new_transactions, 12);
        assert_eq!(batch.attached_postings, 3);
        assert_eq!(
            batch.skipped_postings, 5,
            "the stored total is the sum of the causes"
        );
        assert_eq!(batch.unresolved_path_postings, 4);
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
        assert_eq!(batch.unresolved_path_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_on_an_unknown_batch_is_not_found(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let result = svc
            .close(&bc_models::ImportBatchId::new(), Counts::default())
            .await;
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
