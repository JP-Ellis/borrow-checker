//! Import execution: deduplicate raw rows, then create transactions and attach
//! source references for the rows that are new.

use bc_models::AccountId;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::TransactionId;
use jiff::Timestamp;

use crate::BcResult;
use crate::RawTransaction;

/// Deduplicates `raws` against stored source references for `account_id`, then
/// creates a transaction and attaches a [`SourceRef`] for each new row.
///
/// Single-account raw rows each become a single-posting (interim, unbalanced)
/// transaction. Rows whose `(account, fingerprint, occurrence)` already exist
/// are skipped, making repeated imports of the same document hierarchy
/// idempotent.
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
            let Some(amount) = raw.postings.first().and_then(|p| p.amount.clone()) else {
                // No concrete amount to fingerprint; the per-row loop below skips
                // this index consistently, so the exact placeholder value here is
                // never used to persist a `SourceRef`.
                return String::new();
            };
            SourceRef::compute_fingerprint(
                raw.date,
                &raw.description,
                &amount,
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

        let Some(amount) = raw.postings.first().and_then(|p| p.amount.clone()) else {
            tracing::warn!(
                index = decision.index,
                "raw row has no concrete posting amount; skipping"
            );
            continue;
        };

        let posting_account = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
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
            .account_id(account_id.clone())
            .date(raw.date)
            .narration(raw.description.clone())
            .amount(amount)
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
}
