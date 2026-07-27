//! Import execution: deduplicate raw rows, then create transactions and attach
//! source references for the rows that are new.

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::TransactionId;
use jiff::Timestamp;

use crate::BcResult;
use crate::RawTransaction;

/// Deduplicates `raws` against stored source references for `account_id`, then
/// creates a transaction and attaches a [`SourceRef`] for each new row.
///
/// This is the **interim single-posting** import path. Only a raw transaction
/// carrying exactly one posting with a concrete amount is persisted; it becomes
/// a single-posting (unbalanced, `Unreconciled`) transaction booked against
/// `account_id`. Rows whose `(account, fingerprint, occurrence)` already exist
/// are skipped, making repeated imports of the same document hierarchy
/// idempotent.
///
/// Rows that are not persistable on this path are skipped (logged at `warn`),
/// never imported:
///
/// - **Multi-posting** transactions (e.g. Beancount/Ledger files naming several
///   accounts). Persisting these requires account **path → id** resolution and
///   balanced multi-posting handling, both **deferred to the persistence
///   phase**. The parser and the WIT→core boundary carry every leg faithfully,
///   but this interim path does not yet write multi-posting transactions — it
///   skips them rather than silently collapsing them to their first leg.
/// - Rows whose single posting has no concrete amount (an elided residual).
/// - Rows with no postings at all (defensive; the WIT→core boundary already
///   rejects these).
///
/// # Arguments
///
/// * `transactions` - Transaction persistence service.
/// * `sources` - Source-reference persistence service.
/// * `account_id` - The account whose statement is being imported (source scope).
/// * `raws` - Parsed rows in file order.
///
/// # Returns
///
/// The number of newly-imported rows.
///
/// # Errors
///
/// Returns [`crate::BcError`] on query, transaction-create, or attach failure.
pub async fn execute_import(
    transactions: &crate::TransactionService,
    sources: &crate::SourceService,
    account_id: &AccountId,
    raws: &[RawTransaction],
) -> BcResult<usize> {
    let fingerprints: Vec<String> = raws
        .iter()
        .map(|raw| {
            let Some(amount) = interim_amount(raw) else {
                // Not persistable on the interim path; the per-row loop below
                // skips this index consistently, so the exact placeholder value
                // here is never used to persist a `SourceRef`.
                return String::new();
            };
            SourceRef::compute_fingerprint(
                raw.date,
                &raw.description,
                Some(&amount),
                raw.reference.as_deref(),
            )
        })
        .collect();
    let existing = sources.existing_occurrences(account_id).await?;
    let decisions = crate::plan_import(&existing, &fingerprints);

    let mut imported = 0_usize;
    for decision in &decisions {
        if decision.already_imported {
            continue;
        }
        let Some(raw) = raws.get(decision.index) else {
            tracing::warn!(
                index = decision.index,
                "import plan referenced an out-of-range row; skipping"
            );
            continue;
        };

        let Some(amount) = interim_amount(raw) else {
            match raw.postings.as_slice() {
                [] => tracing::warn!(index = decision.index, "raw row has no postings; skipping"),
                [_single] => tracing::warn!(
                    index = decision.index,
                    "raw row has no concrete posting amount; skipping"
                ),
                multi => tracing::warn!(
                    index = decision.index,
                    postings = multi.len(),
                    "multi-posting transactions are not yet persisted (deferred \
                     to the persistence phase); skipping"
                ),
            }
            continue;
        };

        let posting_id = bc_models::PostingId::new();
        let posting_account = bc_models::Posting::builder()
            .id(posting_id.clone())
            .account_id(account_id.clone())
            .amount(amount.clone())
            .build();

        let tx_id = TransactionId::new();
        // A freshly imported leg holds a single posting and is therefore
        // unbalanced (an accepted interim state). It stays `Unreconciled` until a
        // merge supplies the counter-leg, since an unbalanced transaction cannot
        // legitimately be reconciled.
        let tx = bc_models::Transaction::builder()
            .id(tx_id.clone())
            .date(raw.date)
            .maybe_payee(raw.payee.clone())
            .description(raw.description.clone())
            .postings(vec![posting_account])
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let source = SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(tx_id)
            .posting_id(posting_id)
            .account_id(account_id.clone())
            .date(raw.date)
            .narration(raw.description.clone())
            .amount(Some(amount))
            .occurrence(decision.occurrence)
            .created_at(Timestamp::now())
            .reference(raw.reference.clone())
            .build();

        // Create the transaction and attach its source reference atomically, so a
        // failure can never leave a transaction without provenance (which a later
        // re-import would then duplicate).
        let mut db_tx = transactions.pool().begin().await?;
        transactions.create_in_tx(&mut db_tx, tx).await?;
        sources.attach_in_tx(&mut db_tx, &source).await?;
        db_tx.commit().await?;

        imported = imported.saturating_add(1);
    }

    Ok(imported)
}

/// Returns the amount to fingerprint and persist for an interim single-posting
/// import, or `None` when the row is not persistable on this path.
///
/// Only a raw transaction with exactly one posting that carries a concrete
/// amount is persistable here; multi-posting transactions (deferred to the
/// persistence phase) and amount-less legs both yield `None`. Both the
/// fingerprint pass and the persist pass consult this, so the set of skipped
/// rows is identical between the two.
///
/// # Arguments
///
/// * `raw` - The parsed row to evaluate.
///
/// # Returns
///
/// `Some(amount)` if the row is a single posting with a concrete amount,
/// otherwise `None`.
fn interim_amount(raw: &RawTransaction) -> Option<Amount> {
    match raw.postings.as_slice() {
        [posting] => posting.amount.clone(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;

    use super::*;
    use crate::RawPosting;

    async fn account(
        pool: &SqlitePool,
        name: &str,
        ty: AccountType,
        kind: AccountKind,
    ) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create()
            .name(name)
            .account_type(ty)
            .kind(kind)
            .call()
            .await
            .expect("create account")
    }

    fn raw(desc: &str, amount: i64) -> RawTransaction {
        RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description(desc)
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(amount),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
            ])
            .build()
    }

    async fn tx_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
            .fetch_one(pool)
            .await
            .expect("count transactions")
    }

    async fn posting_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM postings")
            .fetch_one(pool)
            .await
            .expect("count postings")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_is_idempotent_and_incremental(pool: SqlitePool) {
        let bank = account(
            &pool,
            "Bank",
            AccountType::Asset,
            AccountKind::DepositAccount,
        )
        .await;
        let txs = crate::TransactionService::new(pool.clone());
        let srcs = crate::SourceService::new(pool.clone());

        let batch = vec![raw("COFFEE", -5), raw("LUNCH", -20)];

        let first = execute_import(&txs, &srcs, &bank, &batch)
            .await
            .expect("import 1");
        assert_eq!(first, 2);
        assert_eq!(tx_count(&pool).await, 2);
        assert_eq!(
            posting_count(&pool).await,
            2,
            "each imported transaction has exactly one posting"
        );

        // Re-import the identical batch: nothing new.
        let second = execute_import(&txs, &srcs, &bank, &batch)
            .await
            .expect("import 2");
        assert_eq!(second, 0);
        assert_eq!(tx_count(&pool).await, 2);

        // Append a genuinely new row: only it imports.
        let grown = vec![raw("COFFEE", -5), raw("LUNCH", -20), raw("DINNER", -40)];
        let third = execute_import(&txs, &srcs, &bank, &grown)
            .await
            .expect("import 3");
        assert_eq!(third, 1);
        assert_eq!(tx_count(&pool).await, 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn identical_rows_both_import_first_run(pool: SqlitePool) {
        let bank = account(
            &pool,
            "Bank",
            AccountType::Asset,
            AccountKind::DepositAccount,
        )
        .await;
        let txs = crate::TransactionService::new(pool.clone());
        let srcs = crate::SourceService::new(pool.clone());

        // Two legitimately identical rows (same day, narration, amount, no reference).
        let batch = vec![raw("COFFEE", -5), raw("COFFEE", -5)];
        let imported = execute_import(&txs, &srcs, &bank, &batch)
            .await
            .expect("import");
        assert_eq!(
            imported, 2,
            "both identical rows import at occurrences 0 and 1"
        );
        assert_eq!(tx_count(&pool).await, 2);

        let again = execute_import(&txs, &srcs, &bank, &batch)
            .await
            .expect("reimport");
        assert_eq!(again, 0, "re-import of both is a no-op");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rows_without_a_concrete_amount_are_skipped(pool: SqlitePool) {
        let bank = account(
            &pool,
            "Bank",
            AccountType::Asset,
            AccountKind::DepositAccount,
        )
        .await;
        let txs = crate::TransactionService::new(pool.clone());
        let srcs = crate::SourceService::new(pool.clone());

        let amountless = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("PENDING")
            .postings(vec![RawPosting::builder().account("Assets:Bank").build()])
            .build();
        let batch = vec![raw("COFFEE", -5), amountless];

        let imported = execute_import(&txs, &srcs, &bank, &batch)
            .await
            .expect("import");
        assert_eq!(
            imported, 1,
            "only the row with a concrete posting amount is imported"
        );
        assert_eq!(tx_count(&pool).await, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn multi_posting_rows_are_deferred_not_collapsed(pool: SqlitePool) {
        let bank = account(
            &pool,
            "Bank",
            AccountType::Asset,
            AccountKind::DepositAccount,
        )
        .await;
        let txs = crate::TransactionService::new(pool.clone());
        let srcs = crate::SourceService::new(pool.clone());

        // A two-leg transaction: the interim path must skip it wholesale rather
        // than silently persisting only its first posting. Full multi-posting
        // persistence is deferred to the persistence phase.
        let multi = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("SPLIT")
            .postings(vec![
                RawPosting::builder()
                    .account("Expenses:Food")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
                RawPosting::builder().account("Assets:Bank").build(),
            ])
            .build();
        let batch = vec![raw("COFFEE", -5), multi];

        let imported = execute_import(&txs, &srcs, &bank, &batch)
            .await
            .expect("import");
        assert_eq!(
            imported, 1,
            "only the single-posting row imports; the multi-posting row is deferred"
        );
        assert_eq!(tx_count(&pool).await, 1);
    }
}
