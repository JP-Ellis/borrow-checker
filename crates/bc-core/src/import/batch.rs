//! Import batch provenance: one record per import run.

use bc_models::ImportBatchId;
use bc_models::ProfileId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::DiscardOutcome;
use crate::import::discard;

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
    /// When the run completed, or `None` if it never did.
    pub finished_at: Option<Timestamp>,
    /// When the run was discarded, or `None` if it still stands.
    pub discarded_at: Option<Timestamp>,
    /// What the run did, or `None` if it never completed.
    ///
    /// An aborted run leaves rows behind that its counts would not account
    /// for, so there is no honest total to report. `None` forces a reader to
    /// say what it does about that rather than spending zeros as a result.
    pub counts: Option<Counts>,
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

impl Counts {
    /// Returns the total postings skipped, whatever the cause.
    ///
    /// # Returns
    ///
    /// The sum of [`Self::unresolved_path_postings`] and
    /// [`Self::other_skipped_postings`], saturating rather than overflowing.
    #[must_use]
    #[inline]
    pub fn skipped(&self) -> usize {
        self.unresolved_path_postings
            .saturating_add(self.other_skipped_postings)
    }
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
            "INSERT INTO import_batches (id, profile_id, importer, started_at) \
             VALUES (?, ?, ?, ?)",
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

    /// Records the final counts for a completed import run and stamps its
    /// completion time.
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
        let to_i64 = |value: usize| {
            i64::try_from(value).map_err(|_err| BcError::BadData("import count exceeds i64".into()))
        };
        let finished_at = Timestamp::now();

        let result = sqlx::query(
            "UPDATE import_batches \
             SET finished_at = ?, new_transactions = ?, attached_postings = ?, \
                 unresolved_path_postings = ?, other_skipped_postings = ? \
             WHERE id = ?",
        )
        .bind(finished_at.to_string())
        .bind(to_i64(counts.new_transactions)?)
        .bind(to_i64(counts.attached_postings)?)
        .bind(to_i64(counts.unresolved_path_postings)?)
        .bind(to_i64(counts.other_skipped_postings)?)
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
            skipped_postings = counts.skipped(),
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
        let row: Row = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM import_batches WHERE id = ?"
        )))
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(format!("import batch {id}")))?;

        parse_row(row)
    }

    /// Lists every import batch, newest first.
    ///
    /// Discarded batches are included: the listing is the audit trail, and
    /// omitting them would make a repeated discard look like it did nothing.
    ///
    /// # Returns
    ///
    /// Every [`ImportBatch`] on record, ordered by start time descending.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    /// Returns [`BcError::Database`] on database query failure.
    #[inline]
    pub async fn list(&self) -> BcResult<Vec<ImportBatch>> {
        let rows: Vec<Row> = sqlx::query_as(sqlx::AssertSqlSafe(format!(
            "SELECT {COLUMNS} FROM import_batches ORDER BY started_at DESC"
        )))
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(parse_row).collect()
    }

    /// Checks that a batch exists and has not already been discarded, without
    /// doing any of the work discarding it would require.
    ///
    /// This is a cheap short-circuit for a caller that wants to reject a
    /// repeat (or unknown) discard before paying for something expensive on
    /// its way there — `bc-cli`'s `execute_discard` calls this before taking a
    /// pre-discard snapshot. It shares core's predicate and error message by
    /// construction, so the two cannot drift apart, but it is not itself the
    /// authoritative guard: [`Self::discard`] re-checks inside its own
    /// transaction, where the check and the work it gates cannot race.
    ///
    /// # Arguments
    ///
    /// * `id` - The batch to check.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no batch with that ID exists,
    /// [`BcError::InvalidInput`] if it has already been discarded, and
    /// [`BcError::Database`] on query failure.
    #[inline]
    pub async fn ensure_discardable(&self, id: &ImportBatchId) -> BcResult<()> {
        let mut conn = self.pool.acquire().await?;
        discard::ensure_discardable(&mut conn, id, &id.to_string()).await
    }

    /// Discards this batch: undoes the import run that produced it.
    ///
    /// Every reference the run wrote is deleted, freeing its dedup slot; every
    /// posting the run created goes with them; every transaction left with no
    /// postings is deleted. A posting the run merely adopted survives, losing
    /// only its provenance. See the `import::discard` module for the reasoning.
    ///
    /// # Arguments
    ///
    /// * `id` - The batch to discard.
    ///
    /// # Returns
    ///
    /// A [`DiscardOutcome`] describing what was removed.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no batch with that ID exists,
    /// [`BcError::InvalidInput`] if it has already been discarded, and
    /// [`BcError::Database`] on database failure.
    #[inline]
    pub async fn discard(&self, id: &ImportBatchId) -> BcResult<DiscardOutcome> {
        discard::discard(&self.pool, id).await
    }
}

/// Raw `import_batches` row tuple, mirroring the `SELECT` column list.
type Row = (
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
    Option<i64>,
);

/// The `SELECT` column list shared by [`Service::find_by_id`] and
/// [`Service::list`], in [`Row`] order.
const COLUMNS: &str = "id, profile_id, importer, started_at, finished_at, discarded_at, \
                       new_transactions, attached_postings, unresolved_path_postings, \
                       other_skipped_postings";

/// Parses a raw `import_batches` row into an [`ImportBatch`].
///
/// # Arguments
///
/// * `row` - The raw row, in `SELECT` column order.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any ID or timestamp string is malformed, if
/// a stored count is negative, or if the outcome columns are partly populated
/// (which the table's `CHECK` should already prevent).
fn parse_row(row: Row) -> BcResult<ImportBatch> {
    let (
        raw_id,
        raw_profile_id,
        importer,
        raw_started_at,
        raw_finished_at,
        raw_discarded_at,
        new_transactions,
        attached_postings,
        unresolved_path_postings,
        other_skipped_postings,
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

    let parse_ts = |raw: String| {
        raw.parse::<Timestamp>()
            .map_err(|e| BcError::BadData(e.to_string()))
    };
    let started_at = parse_ts(raw_started_at)?;
    let finished_at = raw_finished_at.map(parse_ts).transpose()?;
    let discarded_at = raw_discarded_at.map(parse_ts).transpose()?;

    let to_usize = |value: i64| {
        usize::try_from(value).map_err(|_err| BcError::BadData("import count is negative".into()))
    };
    let counts = match (
        new_transactions,
        attached_postings,
        unresolved_path_postings,
        other_skipped_postings,
    ) {
        (Some(new), Some(attached), Some(unresolved), Some(other)) => Some(Counts {
            new_transactions: to_usize(new)?,
            attached_postings: to_usize(attached)?,
            unresolved_path_postings: to_usize(unresolved)?,
            other_skipped_postings: to_usize(other)?,
        }),
        (None, None, None, None) => None,
        _ => {
            return Err(BcError::BadData(format!(
                "import batch {id} has a partly recorded outcome"
            )));
        }
    };

    Ok(ImportBatch {
        id,
        profile_id,
        importer,
        started_at,
        finished_at,
        discarded_at,
        counts,
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use sqlx::SqlitePool;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn a_freshly_opened_batch_has_no_counts(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc.open(None, "ledger").await.expect("open batch");

        let batch = svc.find_by_id(&id).await.expect("find batch");
        assert_eq!(
            batch.counts, None,
            "a run that has not completed has no outcome to report"
        );
        assert_eq!(batch.finished_at, None);
        assert_eq!(batch.discarded_at, None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn closing_records_the_counts_and_the_finish(pool: SqlitePool) {
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
        let counts = batch.counts.expect("a closed batch reports its counts");
        assert_eq!(counts.new_transactions, 12);
        assert_eq!(counts.attached_postings, 3);
        assert_eq!(counts.unresolved_path_postings, 4);
        assert_eq!(counts.other_skipped_postings, 1);
        assert_eq!(counts.skipped(), 5, "the total is the sum of the causes");
        assert!(batch.finished_at.is_some());
    }

    #[sqlx::test(migrations = "./migrations")]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    async fn list_returns_batches_newest_first(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let first = svc.open(None, "csv").await.expect("open first");
        svc.close(&first, Counts::default()).await.expect("close");
        let second = svc.open(None, "ledger").await.expect("open second");

        let batches = svc.list().await.expect("list batches");
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].id, second, "the newest run is listed first");
        assert_eq!(batches[1].id, first);
        assert_eq!(
            batches[0].counts, None,
            "the still-open run reports no counts"
        );
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
