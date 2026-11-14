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
use crate::transaction::bind_posting_columns;
use crate::transaction::insert_posting_row;
use crate::transaction::parse_stored_id;

/// Reads the original that the stored transaction `id` reverses.
///
/// # Arguments
///
/// * `conn` - The connection, or open database transaction, to read through.
/// * `id` - The transaction to look up.
///
/// # Returns
///
/// The original's ID, or `None` when `id` is no reversal or does not exist.
///
/// # Errors
///
/// Returns [`BcError`] on database failure.
pub(crate) async fn original_of(
    conn: &mut sqlx::SqliteConnection,
    id: &TransactionId,
) -> BcResult<Option<String>> {
    let reverses: Option<Option<String>> =
        sqlx::query_scalar("SELECT reverses FROM transactions WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(&mut *conn)
            .await?;
    Ok(reverses.flatten())
}

/// Reads the original that the transaction owning the stored posting `id`
/// reverses.
///
/// # Arguments
///
/// * `conn` - The connection, or open database transaction, to read through.
/// * `id` - The posting to look up.
///
/// # Returns
///
/// The owning reversal's ID and its original's, or `None` when the owner is
/// no reversal or the posting does not exist.
///
/// # Errors
///
/// Returns [`BcError`] on database failure.
pub(crate) async fn original_of_posting(
    conn: &mut sqlx::SqliteConnection,
    id: &PostingId,
) -> BcResult<Option<(String, String)>> {
    let row: Option<(String, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.reverses FROM postings p JOIN transactions t ON t.id = p.transaction_id \
         WHERE p.id = ?",
    )
    .bind(id.to_string())
    .fetch_optional(&mut *conn)
    .await?;
    Ok(row.and_then(|(owner, reverses)| reverses.map(|original| (owner, original))))
}

/// Why a write that would change what a reversal derives from its original
/// is refused.
///
/// # Arguments
///
/// * `reversal` - The reversal the write aimed at.
/// * `original` - The transaction it reverses.
///
/// # Returns
///
/// The reason, naming both.
pub(crate) fn read_only_reason(reversal: &str, original: &str) -> String {
    format!("{reversal} is a reversal; edit the original {original}")
}

/// The refusal of a write that would change what a reversal derives from
/// its original.
///
/// # Arguments
///
/// * `reversal` - The reversal the write aimed at.
/// * `original` - The transaction it reverses.
///
/// # Returns
///
/// A [`BcError::InvalidInput`] carrying [`read_only_reason`].
pub(crate) fn read_only(reversal: &str, original: &str) -> BcError {
    BcError::InvalidInput(read_only_reason(reversal, original))
}

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
/// Returns [`BcError::BadData`] if a negated amount overflows, and
/// [`BcError`] on database failure. An `original` with no reversals, or one
/// that does not exist, is a no-op.
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
        let reversal_id = parse_stored_id(reversal)?;

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
/// The posting's tags and metadata are the caller's to replace.
async fn update_posting_row(
    db_tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: &PostingId,
    posting: &Posting,
    position: i64,
) -> BcResult<()> {
    let query = sqlx::query(
        "UPDATE postings SET account_id = ?, amount = ?, commodity = ?, position = ?, \
         price_value = ?, price_commodity = ?, price_kind = ?, \
         cost_value = ?, cost_commodity = ?, cost_kind = ?, cost_date = ?, cost_label = ?, \
         spread_from = ?, spread_until = ? \
         WHERE id = ?",
    );
    bind_posting_columns(query, posting, position)
        .bind(id.to_string())
        .execute(&mut **db_tx)
        .await?;
    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::BTreeMap;

    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Reconciliation;
    use bc_models::TagId;
    use bc_models::Transaction;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::transaction::DeleteMode;
    use crate::transaction::Service as TransactionService;
    use crate::transfer::Service as TransferService;

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

    /// The accounts and tags [`reversal_follows_every_edit_of_its_original`]
    /// writes with.
    struct Ledger {
        groceries: AccountId,
        dining: AccountId,
        checking: AccountId,
        fees: AccountId,
        shared: TagId,
        weekly: TagId,
    }

    async fn ledger(pool: &sqlx::SqlitePool) -> Ledger {
        let accounts = crate::account::Service::new(pool.clone());
        let mut ids = Vec::new();
        for name in ["Groceries", "Dining", "Checking", "Fees"] {
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
        let [groceries, dining, checking, fees] = <[AccountId; 4]>::try_from(ids)
            .unwrap_or_else(|_ids| unreachable!("four accounts were created"));
        let shared = TagId::new();
        let weekly = TagId::new();
        for (tag, name) in [(&shared, "Groceries:Shared"), (&weekly, "Weekly")] {
            sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
                .bind(tag.to_string())
                .bind(name)
                .bind(Timestamp::now().to_string())
                .execute(pool)
                .await
                .expect("tag");
        }
        Ledger {
            groceries,
            dining,
            checking,
            fees,
            shared,
            weekly,
        }
    }

    fn aud(value: Decimal) -> Amount {
        Amount::new(value, CommodityCode::new("AUD"))
    }

    fn leg(account: &AccountId, value: Decimal) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(account.clone())
            .amount(aud(value))
            .build()
    }

    /// Copies `posting` onto `account` at `value`, with `tags`.
    fn moved(posting: &Posting, account: &AccountId, value: Decimal, tags: &[TagId]) -> Posting {
        Posting::builder()
            .id(posting.id().clone())
            .account_id(account.clone())
            .amount(aud(value))
            .maybe_spread_from(posting.spread_from())
            .maybe_spread_until(posting.spread_until())
            .tag_ids(tags.to_vec())
            .build()
    }

    /// Rebuilds `tx` with a new date, tags and postings.
    fn rebuilt(tx: &Transaction, day: Date, tags: &[TagId], postings: Vec<Posting>) -> Transaction {
        Transaction::builder()
            .id(tx.id().clone())
            .date(day)
            .description(tx.description().to_owned())
            .tag_ids(tags.to_vec())
            .postings(postings)
            .reconciliation(tx.reconciliation())
            .created_at(*tx.created_at())
            .build()
    }

    fn single(account: &AccountId, value: Decimal, tags: &[TagId]) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Transfer")
            .tag_ids(tags.to_vec())
            .postings(vec![leg(account, value)])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    /// A write to an original that has a reversal.
    #[derive(Debug, Clone, Copy)]
    enum Write {
        EditAmount,
        Recategorise,
        RetagTransaction,
        RetagPosting,
        SetSpread,
        ClearSpread,
        AddPosting,
        RemovePosting,
        ChangeDate,
        Merge,
        Unmerge,
        AttachImportLeg,
    }

    /// Creates an original and its reversal, then applies `write` to the
    /// original. Returns the original's and the reversal's IDs.
    async fn reversed_then_written(
        pool: &sqlx::SqlitePool,
        l: &Ledger,
        write: Write,
    ) -> (TransactionId, TransactionId) {
        if matches!(write, Write::Merge | Write::Unmerge) {
            return transferred(pool, l, write).await;
        }
        let svc = TransactionService::new(pool.clone());
        let spread = Posting::builder()
            .id(PostingId::new())
            .account_id(l.dining.clone())
            .amount(aud(dec!(20)))
            .spread_from(date(2026, 1, 1))
            .spread_until(date(2026, 12, 31))
            .build();
        let groceries = leg(&l.groceries, dec!(30));
        let original = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Groceries")
            .tag_ids(vec![l.shared.clone()])
            .postings(vec![
                moved(
                    &groceries,
                    &l.groceries,
                    dec!(30),
                    std::slice::from_ref(&l.weekly),
                ),
                spread,
                leg(&l.checking, dec!(-50)),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(original.clone()).await.expect("create");
        let reversal = svc.reverse(original.id()).await.expect("reverse");
        let stored = svc.find_by_id(original.id()).await.expect("original");
        let [first, second, _third] = <[Posting; 3]>::try_from(stored.postings().to_vec())
            .unwrap_or_else(|_postings| unreachable!("three postings were created"));

        match write {
            Write::SetSpread => {
                svc.set_posting_spread(first.id(), date(2026, 1, 1), date(2026, 3, 31))
                    .await
                    .expect("set spread");
            }
            Write::ClearSpread => {
                svc.clear_posting_spread(second.id())
                    .await
                    .expect("clear spread");
            }
            Write::AttachImportLeg => {
                let mut db_tx = pool.begin().await.expect("begin");
                svc.add_postings_in_tx(
                    &mut db_tx,
                    stored.id(),
                    stored.date(),
                    &[leg(&l.fees, dec!(5))],
                )
                .await
                .expect("attach");
                db_tx.commit().await.expect("commit");
            }
            Write::EditAmount
            | Write::Recategorise
            | Write::RetagTransaction
            | Write::RetagPosting
            | Write::AddPosting
            | Write::RemovePosting
            | Write::ChangeDate
            | Write::Merge
            | Write::Unmerge => {
                svc.edit(edited(&stored, l, write)).await.expect("edit");
            }
        }
        (stored.id().clone(), reversal)
    }

    /// The whole-transaction edit `write` makes to `stored`.
    fn edited(stored: &Transaction, l: &Ledger, write: Write) -> Transaction {
        let [first, second, third] = <[Posting; 3]>::try_from(stored.postings().to_vec())
            .unwrap_or_else(|_postings| unreachable!("three postings were created"));
        let tags = stored.tag_ids();
        let day = stored.date();
        match write {
            Write::EditAmount => rebuilt(
                stored,
                day,
                tags,
                vec![
                    moved(&first, &l.groceries, dec!(35), first.tag_ids()),
                    second,
                    third,
                ],
            ),
            Write::Recategorise => rebuilt(
                stored,
                day,
                tags,
                vec![
                    moved(&first, &l.fees, dec!(30), first.tag_ids()),
                    second,
                    third,
                ],
            ),
            Write::RetagTransaction => rebuilt(
                stored,
                day,
                std::slice::from_ref(&l.weekly),
                vec![first, second, third],
            ),
            Write::RetagPosting => rebuilt(
                stored,
                day,
                tags,
                vec![
                    moved(
                        &first,
                        &l.groceries,
                        dec!(30),
                        std::slice::from_ref(&l.shared),
                    ),
                    second,
                    third,
                ],
            ),
            Write::AddPosting => rebuilt(
                stored,
                day,
                tags,
                vec![first, second, third, leg(&l.fees, dec!(5))],
            ),
            Write::RemovePosting => rebuilt(stored, day, tags, vec![first, third]),
            Write::ChangeDate => {
                rebuilt(stored, date(2026, 2, 20), tags, vec![first, second, third])
            }
            Write::SetSpread
            | Write::ClearSpread
            | Write::AttachImportLeg
            | Write::Merge
            | Write::Unmerge => {
                unreachable!("{write:?} is not a whole-transaction edit")
            }
        }
    }

    /// Merges a reversed original with another leg, or reverses a merged
    /// original and unmerges it. Returns the original's and the reversal's
    /// IDs.
    async fn transferred(
        pool: &sqlx::SqlitePool,
        l: &Ledger,
        write: Write,
    ) -> (TransactionId, TransactionId) {
        let svc = TransactionService::new(pool.clone());
        let transfers = TransferService::new(pool.clone());
        let original = single(&l.checking, dec!(-50), std::slice::from_ref(&l.shared));
        let other = single(&l.groceries, dec!(50), &[]);
        svc.create(original.clone()).await.expect("create");
        svc.create(other.clone()).await.expect("create");
        let reversal = if matches!(write, Write::Merge) {
            let reversal = svc.reverse(original.id()).await.expect("reverse");
            transfers
                .merge(original.id(), other.id())
                .await
                .expect("merge");
            reversal
        } else {
            transfers
                .merge(original.id(), other.id())
                .await
                .expect("merge");
            let reversal = svc.reverse(original.id()).await.expect("reverse");
            transfers.unmerge(original.id()).await.expect("unmerge");
            reversal
        };
        (original.id().clone(), reversal)
    }

    /// Asserts that `reversal` cancels `original` and carries its tags,
    /// spreads and date.
    fn assert_cancels(original: &Transaction, reversal: &Transaction) {
        let mut net: BTreeMap<(String, String), Decimal> = BTreeMap::new();
        for posting in original.postings().iter().chain(reversal.postings()) {
            let amount = posting.amount().expect("concrete amounts in test");
            let entry = net
                .entry((
                    posting.account_id().to_string(),
                    amount.commodity().as_str().to_owned(),
                ))
                .or_default();
            *entry = entry.checked_add(amount.value()).expect("no overflow");
        }
        assert!(
            net.values().all(Decimal::is_zero),
            "original and reversal net to zero per account and commodity: {net:?}"
        );
        assert_eq!(reversal.date(), original.date());
        let mut original_tags: Vec<String> =
            original.tag_ids().iter().map(ToString::to_string).collect();
        let mut reversal_tags: Vec<String> =
            reversal.tag_ids().iter().map(ToString::to_string).collect();
        original_tags.sort();
        reversal_tags.sort();
        assert_eq!(reversal_tags, original_tags);
        assert_eq!(reversal.postings().len(), original.postings().len());
        for (orig, rev) in original.postings().iter().zip(reversal.postings()) {
            assert_eq!(rev.account_id(), orig.account_id());
            assert_eq!(rev.tag_ids(), orig.tag_ids());
            assert_eq!(
                (rev.spread_from(), rev.spread_until()),
                (orig.spread_from(), orig.spread_until())
            );
        }
    }

    #[rstest]
    #[case::edit_amount(Write::EditAmount)]
    #[case::recategorise(Write::Recategorise)]
    #[case::retag_transaction(Write::RetagTransaction)]
    #[case::retag_posting(Write::RetagPosting)]
    #[case::set_spread(Write::SetSpread)]
    #[case::clear_spread(Write::ClearSpread)]
    #[case::add_posting(Write::AddPosting)]
    #[case::remove_posting(Write::RemovePosting)]
    #[case::change_date(Write::ChangeDate)]
    #[case::merge(Write::Merge)]
    #[case::unmerge(Write::Unmerge)]
    #[case::attach_import_leg(Write::AttachImportLeg)]
    #[tokio::test]
    async fn reversal_follows_every_edit_of_its_original(#[case] write: Write) {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = crate::open_db_at(&dir.path().join("db.sqlite"))
            .await
            .expect("open db");
        let l = ledger(&pool).await;
        let (original_id, reversal_id) = reversed_then_written(&pool, &l, write).await;

        let svc = TransactionService::new(pool.clone());
        let original = svc.find_by_id(&original_id).await.expect("original");
        let reversal = svc.find_by_id(&reversal_id).await.expect("reversal");
        assert_cancels(&original, &reversal);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn deleting_an_original_deletes_its_reversals(pool: sqlx::SqlitePool) {
        let l = ledger(&pool).await;
        let svc = TransactionService::new(pool.clone());
        let original = single(&l.checking, dec!(-50), std::slice::from_ref(&l.shared));
        svc.create(original.clone()).await.expect("create");
        let reversal = svc.reverse(original.id()).await.expect("reverse");

        let outcome = svc
            .delete(original.id(), DeleteMode::KeepProvenance)
            .await
            .expect("delete");

        assert_eq!(outcome.reversals_deleted, 1);
        assert!(matches!(
            svc.find_by_id(&reversal).await,
            Err(BcError::NotFound(_))
        ));
    }

    /// Stores a two-leg original with a spread leg, reverses it and returns
    /// both as stored.
    async fn reversed_pair(pool: &sqlx::SqlitePool, l: &Ledger) -> (Transaction, Transaction) {
        let svc = TransactionService::new(pool.clone());
        let spread = Posting::builder()
            .id(PostingId::new())
            .account_id(l.dining.clone())
            .amount(aud(dec!(1200)))
            .spread_from(date(2026, 1, 1))
            .spread_until(date(2026, 12, 31))
            .build();
        let original = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Annual bill")
            .tag_ids(vec![l.shared.clone()])
            .postings(vec![spread, leg(&l.checking, dec!(-1200))])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(original.clone()).await.expect("create");
        let reversal = svc.reverse(original.id()).await.expect("reverse");
        (
            svc.find_by_id(original.id()).await.expect("original"),
            svc.find_by_id(&reversal).await.expect("reversal"),
        )
    }

    /// Copies `tx` with a new description and metadata.
    fn restated(tx: &Transaction, description: &str, metadata: bc_models::Metadata) -> Transaction {
        Transaction::builder()
            .id(tx.id().clone())
            .date(tx.date())
            .description(description.to_owned())
            .tag_ids(tx.tag_ids().to_vec())
            .metadata(metadata)
            .postings(tx.postings().to_vec())
            .reconciliation(tx.reconciliation())
            .created_at(*tx.created_at())
            .build()
    }

    /// A write aimed at a reversal.
    #[derive(Debug, Clone, Copy)]
    enum ReversalWrite {
        EditAmount,
        EditTags,
        EditMetadata,
        EditDate,
        EditIfUnchangedAmount,
        SetSpread,
        ClearSpread,
        AttachLeg,
    }

    #[rstest]
    #[case::edit_amount(ReversalWrite::EditAmount)]
    #[case::edit_tags(ReversalWrite::EditTags)]
    #[case::edit_metadata(ReversalWrite::EditMetadata)]
    #[case::edit_date(ReversalWrite::EditDate)]
    #[case::edit_if_unchanged_amount(ReversalWrite::EditIfUnchangedAmount)]
    #[case::set_spread(ReversalWrite::SetSpread)]
    #[case::clear_spread(ReversalWrite::ClearSpread)]
    #[case::attach_leg(ReversalWrite::AttachLeg)]
    #[tokio::test]
    async fn editing_a_reversal_is_refused_naming_the_original(#[case] write: ReversalWrite) {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = crate::open_db_at(&dir.path().join("db.sqlite"))
            .await
            .expect("open db");
        let l = ledger(&pool).await;
        let (original, reversal) = reversed_pair(&pool, &l).await;
        let svc = TransactionService::new(pool.clone());
        let [first, second] = <[Posting; 2]>::try_from(reversal.postings().to_vec())
            .unwrap_or_else(|_postings| unreachable!("two postings were reversed"));

        let result = match write {
            ReversalWrite::EditAmount => svc
                .edit(rebuilt(
                    &reversal,
                    reversal.date(),
                    reversal.tag_ids(),
                    vec![moved(&first, &l.dining, dec!(-1000), &[]), second],
                ))
                .await
                .map(drop),
            ReversalWrite::EditTags => svc
                .edit(rebuilt(
                    &reversal,
                    reversal.date(),
                    std::slice::from_ref(&l.weekly),
                    vec![first, second],
                ))
                .await
                .map(drop),
            ReversalWrite::EditMetadata => {
                let note = bc_models::MetaEntry::new(
                    bc_models::MetaKey::new("note").expect("key"),
                    bc_models::MetaValue::Text("invented".to_owned()),
                );
                svc.edit(restated(
                    &reversal,
                    reversal.description(),
                    bc_models::Metadata::new(vec![note]),
                ))
                .await
                .map(drop)
            }
            ReversalWrite::EditDate => svc
                .edit(rebuilt(
                    &reversal,
                    date(2026, 2, 1),
                    reversal.tag_ids(),
                    vec![first, second],
                ))
                .await
                .map(drop),
            ReversalWrite::EditIfUnchangedAmount => svc
                .edit_if_unchanged(
                    &reversal,
                    rebuilt(
                        &reversal,
                        reversal.date(),
                        reversal.tag_ids(),
                        vec![moved(&first, &l.dining, dec!(-1000), &[]), second],
                    ),
                )
                .await
                .map(drop),
            ReversalWrite::SetSpread => {
                svc.set_posting_spread(second.id(), date(2026, 1, 1), date(2026, 6, 30))
                    .await
            }
            ReversalWrite::ClearSpread => svc.clear_posting_spread(first.id()).await,
            ReversalWrite::AttachLeg => {
                let mut db_tx = pool.begin().await.expect("begin");
                svc.add_postings_in_tx(
                    &mut db_tx,
                    reversal.id(),
                    reversal.date(),
                    &[leg(&l.fees, dec!(5))],
                )
                .await
                .map(drop)
            }
        };

        let expected = format!(
            "{} is a reversal; edit the original {}",
            reversal.id(),
            original.id()
        );
        assert!(
            matches!(result, Err(BcError::InvalidInput(ref message)) if *message == expected),
            "{write:?} is refused: {result:?}"
        );
        let stored = svc.find_by_id(reversal.id()).await.expect("reversal");
        assert_cancels(&original, &stored);
    }

    /// Rebuilds `tx` field by field, as `bc-service`'s `model_from_edit`
    /// builds the UI's edit: every posting from its stored fields, with the
    /// elided amount left out, and a fresh `created_at`.
    fn as_the_ui_sends(tx: &Transaction, description: &str) -> Transaction {
        let postings = tx
            .postings()
            .iter()
            .map(|p| {
                Posting::builder()
                    .id(p.id().clone())
                    .account_id(p.account_id().clone())
                    .maybe_amount(p.amount().cloned())
                    .maybe_price(p.price().cloned())
                    .maybe_cost(p.cost().cloned())
                    .metadata(p.metadata().clone())
                    .tag_ids(p.tag_ids().to_vec())
                    .maybe_spread_from(p.spread_from())
                    .maybe_spread_until(p.spread_until())
                    .build()
            })
            .collect();
        Transaction::builder()
            .id(tx.id().clone())
            .date(tx.date())
            .description(description.to_owned())
            .metadata(tx.metadata().clone())
            .postings(postings)
            .reconciliation(tx.reconciliation())
            .tag_ids(tx.tag_ids().to_vec())
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_ui_rename_of_a_reversal_with_an_elided_leg_saves(pool: sqlx::SqlitePool) {
        let l = ledger(&pool).await;
        let svc = TransactionService::new(pool.clone());
        let elided = Posting::builder()
            .id(PostingId::new())
            .account_id(l.checking.clone())
            .build();
        let original = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Annual bill")
            .tag_ids(vec![l.shared.clone()])
            .postings(vec![leg(&l.dining, dec!(1200)), elided])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(original.clone()).await.expect("create");
        let reversal_id = svc.reverse(original.id()).await.expect("reverse");
        let stored = svc.find_by_id(&reversal_id).await.expect("reversal");
        assert!(
            stored.postings().iter().any(|p| p.amount().is_none()),
            "the reversal keeps the elided leg"
        );

        svc.edit_if_unchanged(
            &as_the_ui_sends(&stored, stored.description()),
            as_the_ui_sends(&stored, "Refunded annual bill"),
        )
        .await
        .expect("a UI rename saves");

        let renamed = svc.find_by_id(&reversal_id).await.expect("reversal");
        assert_eq!(renamed.description(), "Refunded annual bill");
        assert_eq!(renamed.postings(), stored.postings());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_reversal_description_and_reconciliation_stay_editable(pool: sqlx::SqlitePool) {
        let l = ledger(&pool).await;
        let (original, reversal) = reversed_pair(&pool, &l).await;
        let svc = TransactionService::new(pool.clone());

        let renamed = restated(
            &reversal,
            "Refunded annual bill",
            reversal.metadata().clone(),
        );
        svc.edit(renamed.clone())
            .await
            .expect("edit the description");
        let base = svc.find_by_id(reversal.id()).await.expect("reversal");
        svc.edit_if_unchanged(&base, restated(&base, "Refund", base.metadata().clone()))
            .await
            .expect("edit the description against a base");
        svc.reconcile(reversal.id(), Reconciliation::Flagged)
            .await
            .expect("reconcile");

        let stored = svc.find_by_id(reversal.id()).await.expect("reversal");
        assert_eq!(stored.description(), "Refund");
        assert_eq!(stored.reconciliation(), Reconciliation::Flagged);
        assert_cancels(&original, &stored);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reversing_a_reversal_is_refused(pool: sqlx::SqlitePool) {
        let l = ledger(&pool).await;
        let (_original, reversal) = reversed_pair(&pool, &l).await;
        let svc = TransactionService::new(pool.clone());

        let result = svc.reverse(reversal.id()).await;

        let expected = format!("{} is a reversal and cannot be reversed", reversal.id());
        assert!(
            matches!(result, Err(BcError::InvalidInput(ref message)) if *message == expected),
            "{result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reversing_an_original_twice_rebuilds_both(pool: sqlx::SqlitePool) {
        let l = ledger(&pool).await;
        let (original, first) = reversed_pair(&pool, &l).await;
        let svc = TransactionService::new(pool.clone());
        let second = svc.reverse(original.id()).await.expect("reverse again");

        let [spread, checking] = <[Posting; 2]>::try_from(original.postings().to_vec())
            .unwrap_or_else(|_postings| unreachable!("two postings were created"));
        svc.edit(rebuilt(
            &original,
            original.date(),
            original.tag_ids(),
            vec![
                moved(&spread, &l.groceries, dec!(1000), &[]),
                moved(&checking, &l.checking, dec!(-1000), &[]),
            ],
        ))
        .await
        .expect("edit the original");

        let edited = svc.find_by_id(original.id()).await.expect("original");
        for reversal in [first.id(), &second] {
            let stored = svc.find_by_id(reversal).await.expect("reversal");
            assert_eq!(stored.reverses(), Some(original.id()));
            assert_cancels(&edited, &stored);
        }
    }

    /// Which side of a merge carries the reversal link.
    #[derive(Debug, Clone, Copy)]
    enum MergeSide {
        SurvivorIsReversal,
        AbsorbedIsReversal,
        AbsorbedReversed,
    }

    #[rstest]
    #[case::survivor_is_a_reversal(MergeSide::SurvivorIsReversal)]
    #[case::absorbed_is_a_reversal(MergeSide::AbsorbedIsReversal)]
    #[case::absorbed_has_a_reversal(MergeSide::AbsorbedReversed)]
    #[tokio::test]
    async fn merging_a_reversal_or_a_reversed_transaction_is_refused(#[case] side: MergeSide) {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = crate::open_db_at(&dir.path().join("db.sqlite"))
            .await
            .expect("open db");
        let l = ledger(&pool).await;
        let svc = TransactionService::new(pool.clone());
        let reversed = single(&l.checking, dec!(-50), &[]);
        svc.create(reversed.clone()).await.expect("create");
        let reversal = svc.reverse(reversed.id()).await.expect("reverse");
        let other = single(&l.groceries, dec!(50), &[]);
        svc.create(other.clone()).await.expect("create");

        let (survivor, absorbed, named, reason) = match side {
            MergeSide::SurvivorIsReversal => (&reversal, other.id(), &reversal, "is a reversal"),
            MergeSide::AbsorbedIsReversal => (other.id(), &reversal, &reversal, "is a reversal"),
            MergeSide::AbsorbedReversed => {
                (other.id(), reversed.id(), reversed.id(), "has a reversal")
            }
        };
        let result = TransferService::new(pool.clone())
            .merge(survivor, absorbed)
            .await;

        let expected = format!("{named} {reason} and cannot be merged");
        assert!(
            matches!(result, Err(BcError::InvalidInput(ref message)) if *message == expected),
            "{side:?}: {result:?}"
        );
        assert_eq!(svc.list().await.expect("list").len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_refuses_a_transaction_marked_as_a_reversal(pool: sqlx::SqlitePool) {
        let l = ledger(&pool).await;
        let svc = TransactionService::new(pool.clone());
        let original = single(&l.checking, dec!(-50), &[]);
        svc.create(original.clone()).await.expect("create");
        let forged = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 15))
            .description("Forged reversal")
            .postings(vec![leg(&l.checking, dec!(999))])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .reverses(original.id().clone())
            .build();

        let result = svc.create(forged).await;

        assert!(
            matches!(result, Err(BcError::InvalidInput(_))),
            "{result:?}"
        );
        assert_eq!(svc.list().await.expect("list").len(), 1);
    }
}
