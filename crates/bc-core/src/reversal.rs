//! Rebuilds a reversal from its original.
//!
//! A reversal is a function of the transaction it reverses: its date, tags,
//! metadata and postings (negated amounts, everything else copied) all follow
//! the original. [`refresh_reversals`] is the single writer of that content.

use bc_models::Amount;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::TransactionId;
use rust_decimal::Decimal;
use sqlx::Connection as _;

use crate::BcError;
use crate::BcResult;
use crate::metadata::Owner;
use crate::transaction::Service;
use crate::transaction::insert_posting_row;

/// Rewrites every reversal of `original` to match it.
///
/// Each transaction whose `reverses` names `original` takes the original's
/// date, transaction tags and metadata. Its postings are rewritten by
/// position: a position that still exists keeps its posting id and has its
/// columns, tags and metadata replaced, a new position gets a fresh posting,
/// and surplus positions are deleted. A posting carries the negated amount (an
/// elided amount stays elided) and the original's account, price, cost, spread,
/// metadata and tags.
///
/// Runs inside the caller's open database transaction when there is one, as a
/// savepoint, and as its own transaction otherwise. Emits no events.
///
/// # Arguments
///
/// * `conn` - The connection, or open database transaction, to write through.
/// * `original` - The transaction whose reversals to rebuild.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] if `original` does not exist, and
/// [`BcError::BadData`] if a negated amount overflows. Returns [`BcError`] on
/// database failure.
pub(crate) async fn refresh_reversals(
    conn: &mut sqlx::SqliteConnection,
    original: &TransactionId,
) -> BcResult<()> {
    let reversals: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM transactions WHERE reverses = ?")
            .bind(original.to_string())
            .fetch_all(&mut *conn)
            .await?;
    if reversals.is_empty() {
        return Ok(());
    }
    let source = Service::find_in(conn, original).await?;

    let mut db_tx = conn.begin().await?;
    for (reversal,) in &reversals {
        sqlx::query("UPDATE transactions SET date = ? WHERE id = ?")
            .bind(source.date().to_string())
            .bind(reversal)
            .execute(&mut *db_tx)
            .await?;

        let existing: Vec<(String,)> = sqlx::query_as(
            "SELECT id FROM postings WHERE transaction_id = ? ORDER BY position ASC",
        )
        .bind(reversal)
        .fetch_all(&mut *db_tx)
        .await?;
        let reversal_id = reversal
            .parse::<TransactionId>()
            .map_err(|e| BcError::BadData(format!("invalid transaction id '{reversal}': {e}")))?;

        for (index, posting) in source.postings().iter().enumerate() {
            let position = i64::try_from(index)
                .map_err(|_err| BcError::BadData("posting position exceeds i64::MAX".into()))?;
            let kept = existing
                .get(index)
                .map(|(id,)| {
                    id.parse::<PostingId>()
                        .map_err(|e| BcError::BadData(format!("invalid posting id '{id}': {e}")))
                })
                .transpose()?;
            let reversed = negate(posting, kept.clone().unwrap_or_else(PostingId::new))?;
            match kept {
                Some(id) => {
                    update_posting_row(&mut db_tx, &id, &reversed, position).await?;
                    sqlx::query("DELETE FROM posting_tags WHERE posting_id = ?")
                        .bind(id.to_string())
                        .execute(&mut *db_tx)
                        .await?;
                    crate::metadata::replace(
                        &mut db_tx,
                        Owner::Posting,
                        &id.to_string(),
                        reversed.metadata(),
                    )
                    .await?;
                }
                None => insert_posting_row(&mut db_tx, &reversal_id, &reversed, position).await?,
            }
            crate::tag::insert_posting_tags(&mut db_tx, reversed.id(), posting.tag_ids()).await?;
        }

        for (surplus,) in existing.iter().skip(source.postings().len()) {
            sqlx::query("DELETE FROM posting_tags WHERE posting_id = ?")
                .bind(surplus)
                .execute(&mut *db_tx)
                .await?;
            sqlx::query("DELETE FROM posting_metadata WHERE posting_id = ?")
                .bind(surplus)
                .execute(&mut *db_tx)
                .await?;
            sqlx::query("DELETE FROM postings WHERE id = ?")
                .bind(surplus)
                .execute(&mut *db_tx)
                .await?;
        }

        sqlx::query("DELETE FROM transaction_tags WHERE transaction_id = ?")
            .bind(reversal)
            .execute(&mut *db_tx)
            .await?;
        crate::tag::insert_transaction_tags(&mut db_tx, &reversal_id, source.tag_ids()).await?;
        crate::metadata::replace(&mut db_tx, Owner::Transaction, reversal, source.metadata())
            .await?;
    }
    db_tx.commit().await?;
    Ok(())
}

/// Builds the posting that reverses `posting`, under the id `id`.
fn negate(posting: &Posting, id: PostingId) -> BcResult<Posting> {
    let amount = posting
        .amount()
        .map(|amount| {
            amount
                .value()
                .checked_mul(Decimal::NEGATIVE_ONE)
                .map(|value| Amount::new(value, amount.commodity().clone()))
                .ok_or_else(|| BcError::BadData("posting amount negation overflow".into()))
        })
        .transpose()?;
    Ok(Posting::builder()
        .id(id)
        .account_id(posting.account_id().clone())
        .maybe_amount(amount)
        .maybe_price(posting.price().cloned())
        .maybe_cost(posting.cost().cloned())
        .metadata(posting.metadata().clone())
        .maybe_spread_from(posting.spread_from())
        .maybe_spread_until(posting.spread_until())
        .tag_ids(posting.tag_ids().to_vec())
        .build())
}

/// Overwrites the stored columns of an existing posting row.
///
/// Mirrors the column list of `insert_posting_row`. The posting's tags and
/// metadata are the caller's to replace.
async fn update_posting_row(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &PostingId,
    posting: &Posting,
    position: i64,
) -> BcResult<()> {
    let (price_value, price_commodity, price_kind) = crate::quote::quote_columns(posting.price());
    let (cost_value, cost_commodity, cost_kind) =
        crate::quote::quote_columns(posting.cost().map(bc_models::Cost::basis));
    let cost_date = posting
        .cost()
        .and_then(bc_models::Cost::date)
        .map(|d| d.to_string());
    let cost_label = posting.cost().and_then(|c| c.label()).map(str::to_owned);

    sqlx::query(
        "UPDATE postings SET account_id = ?, amount = ?, commodity = ?, position = ?, \
         price_value = ?, price_commodity = ?, price_kind = ?, \
         cost_value = ?, cost_commodity = ?, cost_kind = ?, cost_date = ?, cost_label = ?, \
         spread_from = ?, spread_until = ? \
         WHERE id = ?",
    )
    .bind(posting.account_id().to_string())
    .bind(posting.amount().map(|a| a.value().to_string()))
    .bind(posting.amount().map(|a| a.commodity().as_str().to_owned()))
    .bind(position)
    .bind(price_value)
    .bind(price_commodity)
    .bind(price_kind)
    .bind(cost_value)
    .bind(cost_commodity)
    .bind(cost_kind)
    .bind(cost_date)
    .bind(cost_label)
    .bind(posting.spread_from().map(|d| d.to_string()))
    .bind(posting.spread_until().map(|d| d.to_string()))
    .bind(id.to_string())
    .execute(&mut **db_tx)
    .await?;
    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Reconciliation;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::transaction::Service as TransactionService;

    #[sqlx::test(migrations = "./migrations")]
    async fn refresh_follows_the_original_by_position(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let mut ids = Vec::new();
        for name in ["Income", "Checking"] {
            ids.push(
                accounts
                    .create()
                    .name(name)
                    .account_type(AccountType::Asset)
                    .kind(AccountKind::DepositAccount)
                    .call()
                    .await
                    .expect("account"),
            );
        }
        let [income, checking] = ids.as_slice() else {
            unreachable!("two accounts were created")
        };
        let leg = |account: &bc_models::AccountId, value| {
            Posting::builder()
                .id(PostingId::new())
                .account_id(account.clone())
                .amount(Amount::new(value, CommodityCode::new("AUD")))
                .build()
        };
        let original = bc_models::Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Bill")
            .postings(vec![leg(income, dec!(100)), leg(checking, dec!(-100))])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let svc = TransactionService::new(pool.clone());
        svc.create(original.clone()).await.expect("create");
        let reversal_id = svc.reverse(original.id()).await.expect("reverse");
        let before = svc.find_by_id(&reversal_id).await.expect("reversal");

        let first = original.postings().first().expect("first leg");
        let second = original.postings().get(1).expect("second leg");
        sqlx::query("UPDATE transactions SET date = '2026-02-20' WHERE id = ?")
            .bind(original.id().to_string())
            .execute(&pool)
            .await
            .expect("move original");
        sqlx::query("UPDATE postings SET amount = '250' WHERE id = ?")
            .bind(first.id().to_string())
            .execute(&pool)
            .await
            .expect("edit amount");
        sqlx::query("DELETE FROM postings WHERE id = ?")
            .bind(second.id().to_string())
            .execute(&pool)
            .await
            .expect("drop leg");

        let mut db_tx = pool.begin().await.expect("begin");
        refresh_reversals(&mut db_tx, original.id())
            .await
            .expect("refresh");
        db_tx.commit().await.expect("commit");

        let after = svc.find_by_id(&reversal_id).await.expect("reversal");
        assert_eq!(after.date(), date(2026, 2, 20));
        assert_eq!(after.postings().len(), 1);
        let kept = after.postings().first().expect("kept leg");
        assert_eq!(
            kept.id(),
            before.postings().first().expect("old first").id()
        );
        assert_eq!(
            kept.amount(),
            Some(&Amount::new(dec!(-250), CommodityCode::new("AUD")))
        );
    }
}
