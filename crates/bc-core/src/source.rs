//! Source reference (import provenance) persistence service and import planner.

use std::collections::HashMap;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::TransactionId;
use jiff::Timestamp;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::BcResult;
use crate::events::insert_event;

/// Raw `transaction_sources` row tuple, mirroring the `SELECT` column list.
type SourceRow = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    String,
);

/// Persists and queries [`SourceRef`] import-provenance records.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Service {
    /// SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new source-reference service.
    ///
    /// # Arguments
    ///
    /// * `pool` - A SQLite connection pool.
    #[inline]
    #[must_use]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Attaches a source reference to its transaction.
    ///
    /// Appends a [`crate::Event::TransactionSourceAttached`] and inserts the
    /// projection row in one database transaction.
    ///
    /// # Arguments
    ///
    /// * `source` - The fully-built [`SourceRef`] to persist.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on event or row insert failure (including a
    /// `UNIQUE` violation if this `(account, fingerprint, occurrence)` already exists).
    #[inline]
    pub async fn attach(&self, source: &SourceRef) -> BcResult<()> {
        let fingerprint = source.fingerprint();
        let event = crate::Event::TransactionSourceAttached {
            id: source.id().clone(),
            transaction_id: source.transaction_id().clone(),
            account_id: source.account_id().clone(),
            date: source.date(),
            narration: source.narration().to_owned(),
            amount: source.amount().clone(),
            reference: source.reference().map(str::to_owned),
            occurrence: source.occurrence(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO transaction_sources \
             (id, transaction_id, account_id, date, narration, amount, commodity, \
              reference, occurrence, fingerprint, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(source.id().to_string())
        .bind(source.transaction_id().to_string())
        .bind(source.account_id().to_string())
        .bind(source.date().to_string())
        .bind(source.narration())
        .bind(source.amount().value().to_string())
        .bind(source.amount().commodity().as_str())
        .bind(source.reference())
        .bind(i64::from(source.occurrence()))
        .bind(&fingerprint)
        .bind(source.created_at().to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        Ok(())
    }

    /// Returns, per fingerprint, how many source references already exist for an account.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to count references for.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query failure.
    #[inline]
    pub async fn existing_counts(&self, account_id: &AccountId) -> BcResult<HashMap<String, u32>> {
        let rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT fingerprint, COUNT(*) FROM transaction_sources \
             WHERE account_id = ? GROUP BY fingerprint",
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(fp, raw_count)| {
                let count = u32::try_from(raw_count)
                    .map_err(|_err| crate::BcError::BadData("source count exceeds u32".into()))?;
                Ok((fp, count))
            })
            .collect()
    }

    /// Lists all source references attached to a transaction, oldest first.
    ///
    /// # Arguments
    ///
    /// * `transaction_id` - The transaction whose references to list.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query failure or malformed stored data.
    #[inline]
    pub async fn list_for_transaction(
        &self,
        transaction_id: &TransactionId,
    ) -> BcResult<Vec<SourceRef>> {
        let rows: Vec<SourceRow> = sqlx::query_as(
            "SELECT id, transaction_id, account_id, date, narration, amount, commodity, \
                        reference, occurrence, created_at \
                 FROM transaction_sources WHERE transaction_id = ? ORDER BY created_at ASC",
        )
        .bind(transaction_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter().map(parse_source_row).collect()
    }

    /// Detaches (deletes) a source reference by ID.
    ///
    /// Appends a [`crate::Event::TransactionSourceDetached`] and deletes the row.
    ///
    /// # Arguments
    ///
    /// * `id` - The source reference to remove.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] if no such reference exists, or a
    /// database error on failure.
    #[inline]
    pub async fn detach(&self, id: &SourceRefId) -> BcResult<()> {
        let raw_transaction_id: Option<String> =
            sqlx::query_scalar("SELECT transaction_id FROM transaction_sources WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        let Some(found_transaction_id) = raw_transaction_id else {
            return Err(crate::BcError::NotFound(format!("source reference {id}")));
        };
        let transaction_id = found_transaction_id
            .parse::<TransactionId>()
            .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;

        let event = crate::Event::TransactionSourceDetached {
            id: id.clone(),
            transaction_id,
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;
        sqlx::query("DELETE FROM transaction_sources WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *db_tx)
            .await?;
        db_tx.commit().await?;
        Ok(())
    }
}

/// Parses a `transaction_sources` row tuple into a [`SourceRef`].
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] if any stored ID, date, amount, or count is malformed.
fn parse_source_row(row: SourceRow) -> BcResult<SourceRef> {
    let (
        raw_id,
        raw_tx,
        raw_acct,
        raw_date,
        narration,
        raw_amount,
        commodity,
        reference,
        raw_occurrence,
        raw_created,
    ) = row;

    let id = raw_id
        .parse::<SourceRefId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let transaction_id = raw_tx
        .parse::<TransactionId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let account_id = raw_acct
        .parse::<AccountId>()
        .map_err(|e: bc_models::IdParseError| crate::BcError::BadData(e.to_string()))?;
    let date = raw_date
        .parse::<Date>()
        .map_err(|e| crate::BcError::BadData(e.to_string()))?;
    let value = raw_amount
        .parse::<rust_decimal::Decimal>()
        .map_err(|e| crate::BcError::BadData(e.to_string()))?;
    let amount = Amount::new(value, CommodityCode::new(&commodity));
    let occurrence = u32::try_from(raw_occurrence)
        .map_err(|_err| crate::BcError::BadData("occurrence exceeds u32".into()))?;
    let created_at = raw_created
        .parse::<Timestamp>()
        .map_err(|e| crate::BcError::BadData(e.to_string()))?;

    let with_reference = SourceRef::builder()
        .id(id)
        .transaction_id(transaction_id)
        .account_id(account_id)
        .date(date)
        .narration(narration)
        .amount(amount)
        .occurrence(occurrence)
        .created_at(created_at)
        .reference(reference);
    Ok(with_reference.build())
}

/// A per-row decision produced by [`plan_import`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportDecision {
    /// Index of this row in the input `fingerprints` slice (file order).
    pub index: usize,
    /// The row's dedup fingerprint.
    pub fingerprint: String,
    /// The row's occurrence ordinal among identical fingerprints in this batch.
    pub occurrence: u32,
    /// `true` if a stored source reference already covers this occurrence.
    pub already_imported: bool,
}

/// Plans an import: decides, per row, whether it is already imported.
///
/// Walks `fingerprints` in file order, counting occurrences of each fingerprint.
/// A row's `occurrence` is the number of identical fingerprints seen before it in
/// this batch; it is `already_imported` when that occurrence is already covered by
/// the stored count in `existing`. This makes whole-hierarchy re-imports a no-op
/// while still importing genuinely new (or genuinely duplicated) rows.
///
/// # Arguments
///
/// * `existing` - Stored source-reference counts per fingerprint for the target account.
/// * `fingerprints` - Row fingerprints in file order.
///
/// # Returns
///
/// One [`ImportDecision`] per input fingerprint, in the same order.
#[must_use]
#[expect(
    clippy::implicit_hasher,
    reason = "callers always use the default std HashMap hasher"
)]
pub fn plan_import(
    existing: &HashMap<String, u32>,
    fingerprints: &[String],
) -> Vec<ImportDecision> {
    let mut seen: HashMap<String, u32> = HashMap::new();
    fingerprints
        .iter()
        .enumerate()
        .map(|(index, fingerprint)| {
            let occurrence = seen.get(fingerprint).copied().unwrap_or(0);
            seen.insert(fingerprint.clone(), occurrence.saturating_add(1));
            let already_imported = occurrence < existing.get(fingerprint).copied().unwrap_or(0);
            ImportDecision {
                index,
                fingerprint: fingerprint.clone(),
                occurrence,
                already_imported,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::*;

    async fn make_account(pool: &SqlitePool) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    async fn make_tx(pool: &SqlitePool, account: &AccountId) -> TransactionId {
        let counter = crate::AccountService::new(pool.clone())
            .create()
            .name("Counter")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create counter");
        let tx = bc_models::Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 6, 27))
            .description("row")
            .postings(vec![
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(account.clone())
                    .amount(Amount::new(
                        Decimal::from(100_i32),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(counter)
                    .amount(Amount::new(
                        Decimal::from(-100_i32),
                        CommodityCode::new("AUD"),
                    ))
                    .build(),
            ])
            .reconciliation(bc_models::Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        let id = tx.id().clone();
        crate::TransactionService::new(pool.clone())
            .create(tx)
            .await
            .expect("create tx");
        id
    }

    fn source(tx: &TransactionId, account: &AccountId, occurrence: u32) -> SourceRef {
        SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx.clone())
            .account_id(account.clone())
            .date(date(2025, 6, 27))
            .narration("SMARTBEAR")
            .amount(Amount::new(
                Decimal::from(100_i32),
                CommodityCode::new("AUD"),
            ))
            .occurrence(occurrence)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attach_then_count_and_list(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let tx = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());

        svc.attach(&source(&tx, &account, 0)).await.expect("attach");

        let counts = svc.existing_counts(&account).await.expect("counts");
        let fp = SourceRef::compute_fingerprint(
            jiff::civil::date(2025, 6, 27),
            "SMARTBEAR",
            &Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
            None,
        );
        assert_eq!(counts.get(&fp).copied(), Some(1));

        let listed = svc.list_for_transaction(&tx).await.expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            listed.first().expect("one source ref").account_id(),
            &account
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn detach_removes_the_ref(pool: SqlitePool) {
        let account = make_account(&pool).await;
        let tx = make_tx(&pool, &account).await;
        let svc = Service::new(pool.clone());
        let sr = source(&tx, &account, 0);
        let id = sr.id().clone();
        svc.attach(&sr).await.expect("attach");

        svc.detach(&id).await.expect("detach");

        assert!(
            svc.list_for_transaction(&tx)
                .await
                .expect("list")
                .is_empty()
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test code with known length")]
    fn plan_import_marks_new_rows_and_skips_seen() {
        let mut existing = HashMap::new();
        existing.insert("A".to_owned(), 1_u32);

        let fps = vec!["A".to_owned(), "A".to_owned(), "B".to_owned()];
        let decisions = plan_import(&existing, &fps);

        assert_eq!(decisions[0].occurrence, 0);
        assert!(decisions[0].already_imported);
        assert_eq!(decisions[1].occurrence, 1);
        assert!(!decisions[1].already_imported);
        assert_eq!(decisions[2].occurrence, 0);
        assert!(!decisions[2].already_imported);
    }

    #[test]
    fn plan_import_is_idempotent_when_counts_match() {
        let mut existing = HashMap::new();
        existing.insert("A".to_owned(), 2_u32);
        let fps = vec!["A".to_owned(), "A".to_owned()];
        let decisions = plan_import(&existing, &fps);
        assert!(
            decisions.iter().all(|d| d.already_imported),
            "re-import is a no-op"
        );
    }
}
