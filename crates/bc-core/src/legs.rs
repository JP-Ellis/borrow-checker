//! Resolved legs: one ordered pass over `postings`, grouped by transaction,
//! with each elided leg's residual resolved in-stream.
//!
//! A filter selects transactions. Every leg of a selected transaction is read,
//! because the residual rule needs the full leg set to tell one elided leg
//! from two (`docs/DESIGN.md` §4.4). The filter's per-leg form decides which
//! legs are yielded.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumers migrate onto the stream in later commits"
    )
)]

use std::collections::HashMap;
use std::sync::Arc;

use bc_models::AccountId;
use bc_models::TagId;
use futures_util::Stream;
use futures_util::TryStreamExt as _;
use futures_util::stream::BoxStream;
use jiff::civil::Date;
use rust_decimal::Decimal;
use sqlx::Row as _;
use sqlx::sqlite::SqliteRow;

use crate::BcError;
use crate::BcResult;
use crate::residual::KeyedResidual;
use crate::residual::keyed_residual;

// MARK: Filter

/// Which accounts' legs a stream yields.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LegScope<'a> {
    /// Every account.
    Ledger,
    /// The listed accounts.
    Accounts(&'a [AccountId]),
    /// The account and every descendant.
    Subtree(&'a AccountId),
}

/// Selects the transactions a stream reads and the legs it yields.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LegFilter<'a> {
    /// Accounts whose legs are yielded.
    scope: LegScope<'a>,
    /// Half-open `[from, to)` window on the leg's date.
    window: Option<(Date, Date)>,
    /// A yielded leg, or its transaction, carries a tag in this subtree.
    tag: Option<&'a TagId>,
}

impl<'a> LegFilter<'a> {
    /// A filter over `scope` with no window and no tag.
    pub(crate) const fn new(scope: LegScope<'a>) -> Self {
        Self {
            scope,
            window: None,
            tag: None,
        }
    }

    /// Restricts to legs dated in `[from, to)`.
    pub(crate) const fn window(mut self, from: Date, to: Date) -> Self {
        self.window = Some((from, to));
        self
    }

    /// Restricts to legs that carry, or whose transaction carries, a tag in
    /// `tag`'s subtree, when set.
    pub(crate) const fn tag(mut self, tag: Option<&'a TagId>) -> Self {
        self.tag = tag;
        self
    }
}

// MARK: Query

/// Columns read for every leg, in the index order the decoder relies on.
const LEG_COLUMNS: &str = "p.rowid, p.transaction_id, p.account_id, p.date, p.position, \
     p.amount, p.commodity, p.price_value, p.price_commodity, p.price_kind, \
     p.cost_value, p.cost_commodity, p.cost_kind";

/// A built leg query and its positional binds.
#[derive(Debug)]
struct LegQuery {
    /// The SQL text.
    sql: String,
    /// Values for `?1`, `?2`, … in order.
    binds: Vec<String>,
}

/// Builds the leg query for `filter`.
///
/// The per-leg predicate is written once per alias: on `e` it selects
/// transactions, on `p` it computes `emit`. Numbered parameters let both
/// copies share one set of binds. Every fragment is a compile-time constant;
/// values are bound.
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] if an account id list cannot be
/// serialised.
fn leg_query(filter: &LegFilter<'_>) -> BcResult<LegQuery> {
    let mut binds: Vec<String> = Vec::new();
    let mut ctes: Vec<String> = Vec::new();
    // Each clause is a template over `{a}`, the posting alias.
    let mut clauses: Vec<String> = Vec::new();

    match filter.scope {
        LegScope::Ledger => {}
        LegScope::Accounts(ids) => {
            binds.push(crate::balance::ids_json(ids)?);
            clauses.push(format!(
                "{{a}}.account_id IN (SELECT value FROM json_each(?{}))",
                binds.len()
            ));
        }
        LegScope::Subtree(root) => {
            binds.push(root.to_string());
            ctes.push(format!(
                "acct_tree(id) AS (SELECT ?{} UNION ALL \
                 SELECT a.id FROM accounts a INNER JOIN acct_tree ON a.parent_id = acct_tree.id)",
                binds.len()
            ));
            clauses.push("{a}.account_id IN (SELECT id FROM acct_tree)".to_owned());
        }
    }
    if let Some((from, to)) = filter.window {
        binds.push(from.to_string());
        let from_n = binds.len();
        binds.push(to.to_string());
        clauses.push(format!(
            "{{a}}.date >= ?{from_n} AND {{a}}.date < ?{}",
            binds.len()
        ));
    }
    if let Some(tag) = filter.tag {
        binds.push(tag.to_string());
        ctes.push(format!(
            "tag_subtree(id) AS (SELECT ?{} UNION ALL \
             SELECT tg.id FROM tags tg INNER JOIN tag_subtree ON tg.parent_id = tag_subtree.id)",
            binds.len()
        ));
        // A transaction tag flows down to every leg.
        clauses.push(
            "(EXISTS (SELECT 1 FROM posting_tags pt WHERE pt.posting_id = {a}.id \
               AND pt.tag_id IN (SELECT id FROM tag_subtree)) \
              OR EXISTS (SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id = {a}.transaction_id \
               AND tt.tag_id IN (SELECT id FROM tag_subtree)))"
                .to_owned(),
        );
    }

    if clauses.is_empty() {
        return Ok(LegQuery {
            sql: format!("SELECT {LEG_COLUMNS}, 1 FROM postings p ORDER BY p.transaction_id"),
            binds,
        });
    }

    let predicate = clauses.join(" AND ");
    let on_p = predicate.replace("{a}", "p");
    let on_e = predicate.replace("{a}", "e");
    let with = if ctes.is_empty() {
        String::new()
    } else {
        format!("WITH RECURSIVE {} ", ctes.join(", "))
    };
    Ok(LegQuery {
        sql: format!(
            "{with}SELECT {LEG_COLUMNS}, ({on_p}) FROM postings p \
             WHERE p.transaction_id IN (SELECT e.transaction_id FROM postings e WHERE {on_e}) \
             ORDER BY p.transaction_id"
        ),
        binds,
    })
}

// MARK: Items

/// An interned string: a per-stream dense index plus the shared text.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Interned {
    /// Dense index within the stream's interner for this kind of string.
    idx: u32,
    /// The text.
    name: Arc<str>,
}

impl Interned {
    /// The dense index, unique per string within one stream and one kind.
    pub(crate) const fn idx(&self) -> u32 {
        self.idx
    }

    /// The interned text.
    pub(crate) fn as_str(&self) -> &str {
        &self.name
    }
}

/// Where a yielded leg's value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LegSource {
    /// The stored amount of a concrete leg.
    Concrete,
    /// One commodity of an attributable elided leg's residual.
    Residual,
    /// An elided leg in a transaction with two or more elided legs. It carries
    /// no commodity and a zero value.
    Ambiguous,
}

/// One yielded leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLeg {
    /// The posting's SQLite rowid, valid within the stream's snapshot only.
    posting: i64,
    /// The account the leg posts to.
    account: Interned,
    /// The commodity; `None` only for [`LegSource::Ambiguous`].
    commodity: Option<Interned>,
    /// The amount in `commodity`.
    value: Decimal,
    /// Where the value came from.
    source: LegSource,
}

impl ResolvedLeg {
    /// The posting's SQLite rowid, valid within the stream's snapshot only.
    pub(crate) const fn posting(&self) -> i64 {
        self.posting
    }

    /// The account the leg posts to.
    pub(crate) const fn account(&self) -> &Interned {
        &self.account
    }

    /// The commodity; `None` only for [`LegSource::Ambiguous`].
    pub(crate) const fn commodity(&self) -> Option<&Interned> {
        self.commodity.as_ref()
    }

    /// The amount in the commodity.
    pub(crate) const fn value(&self) -> Decimal {
        self.value
    }

    /// Where the value came from.
    pub(crate) const fn source(&self) -> LegSource {
        self.source
    }
}

/// One transaction with at least one leg in the stream's scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedTransaction {
    /// The transaction id.
    id: Box<str>,
    /// The transaction date.
    date: Date,
    /// The in-scope legs in `(position, rowid)` order. Empty when the only
    /// in-scope leg is elided and its siblings sum to zero.
    legs: Vec<ResolvedLeg>,
}

impl ResolvedTransaction {
    /// The transaction id.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The transaction date.
    pub(crate) const fn date(&self) -> Date {
        self.date
    }

    /// The in-scope legs in `(position, rowid)` order.
    pub(crate) fn legs(&self) -> &[ResolvedLeg] {
        &self.legs
    }
}

// MARK: Stream

/// Maps strings to [`Interned`] handles, allocating once per distinct string.
#[derive(Debug, Default)]
struct Interner {
    /// Text to index.
    map: HashMap<Arc<str>, u32>,
}

impl Interner {
    /// Returns the handle for `text`, interning it on first sight.
    fn intern(&mut self, text: &str) -> BcResult<Interned> {
        if let Some((name, &idx)) = self.map.get_key_value(text) {
            return Ok(Interned {
                idx,
                name: Arc::clone(name),
            });
        }
        let idx = u32::try_from(self.map.len())
            .map_err(|e| BcError::BadData(format!("interner overflow: {e}")))?;
        let name: Arc<str> = Arc::from(text);
        self.map.insert(Arc::clone(&name), idx);
        Ok(Interned { idx, name })
    }
}

/// One leg as read, before resolution.
#[derive(Debug)]
struct RawLeg {
    /// SQLite rowid.
    posting: i64,
    /// Leg order within the transaction.
    position: i64,
    /// Account handle.
    account: Interned,
    /// Stored amount; `None` when elided.
    amount: Option<(Interned, Decimal)>,
    /// Weight (at cost, else at price, else the amount); `None` when elided.
    weight: Option<(Interned, Decimal)>,
    /// Whether the filter yields this leg.
    emit: bool,
}

/// Parses a stored decimal, naming the posting on failure.
fn parse_decimal(text: &str, posting: i64) -> BcResult<Decimal> {
    text.parse::<Decimal>().map_err(|e| {
        BcError::BadData(format!(
            "invalid amount '{text}' on posting rowid {posting}: {e}"
        ))
    })
}

/// Decodes one row into a [`RawLeg`], borrowing every string from the row.
fn decode_leg(
    row: &SqliteRow,
    accounts: &mut Interner,
    commodities: &mut Interner,
) -> BcResult<RawLeg> {
    let posting: i64 = row.try_get(0)?;
    let account = accounts.intern(row.try_get::<&str, _>(2)?)?;
    let position: i64 = row.try_get(4)?;
    let amount_text: Option<&str> = row.try_get(5)?;
    let commodity_text: Option<&str> = row.try_get(6)?;
    let emit = row.try_get::<i64, _>(13)? != 0;

    let (amount, weight) = match (amount_text, commodity_text) {
        (None, None) => (None, None),
        (Some(text), Some(code)) => {
            let value = parse_decimal(text, posting)?;
            let commodity = commodities.intern(code)?;
            let price_value: Option<&str> = row.try_get(7)?;
            let cost_value: Option<&str> = row.try_get(10)?;
            let weight = if price_value.is_none() && cost_value.is_none() {
                (commodity.clone(), value)
            } else {
                // Rare: quoted legs build owned values for the shared parsers.
                let owned = |i: usize| row.try_get::<Option<String>, _>(i);
                let price = crate::quote::parse_quote("price", owned(7)?, owned(8)?, owned(9)?)?;
                let cost =
                    crate::quote::parse_cost(owned(10)?, owned(11)?, owned(12)?, None, None)?;
                let base = bc_models::Amount::new(value, code);
                let weighed = bc_models::weight_of(&base, cost.as_ref(), price.as_ref())
                    .map_err(|e| BcError::BadData(format!("posting weight overflow: {e}")))?;
                (
                    commodities.intern(weighed.commodity().as_str())?,
                    weighed.value(),
                )
            };
            (Some((commodity, value)), Some(weight))
        }
        _ => {
            return Err(BcError::BadData(format!(
                "posting rowid {posting} stores an amount without a commodity, or vice versa"
            )));
        }
    };
    Ok(RawLeg {
        posting,
        position,
        account,
        amount,
        weight,
        emit,
    })
}

/// Resolves one buffered transaction, or `None` when no leg is in scope.
fn resolve(
    id: Box<str>,
    date: Date,
    buffer: &mut [RawLeg],
) -> BcResult<Option<ResolvedTransaction>> {
    if !buffer.iter().any(|leg| leg.emit) {
        return Ok(None);
    }
    buffer.sort_unstable_by_key(|leg| (leg.position, leg.posting));
    let residual = keyed_residual(buffer.iter().map(|leg| leg.weight.clone()))
        .map_err(|e| BcError::BadData(format!("residual overflow in '{id}': {e}")))?;

    let mut legs = Vec::with_capacity(buffer.len());
    for leg in buffer.iter().filter(|leg| leg.emit) {
        match (&leg.amount, &residual) {
            (Some((commodity, value)), _) => legs.push(ResolvedLeg {
                posting: leg.posting,
                account: leg.account.clone(),
                commodity: Some(commodity.clone()),
                value: *value,
                source: LegSource::Concrete,
            }),
            (None, KeyedResidual::Attributable(entries)) => {
                legs.extend(entries.iter().map(|(commodity, value)| ResolvedLeg {
                    posting: leg.posting,
                    account: leg.account.clone(),
                    commodity: Some(commodity.clone()),
                    value: *value,
                    source: LegSource::Residual,
                }));
            }
            (None, KeyedResidual::Ambiguous) => legs.push(ResolvedLeg {
                posting: leg.posting,
                account: leg.account.clone(),
                commodity: None,
                value: Decimal::ZERO,
                source: LegSource::Ambiguous,
            }),
            // An elided leg makes the residual non-`NotElided`.
            (None, KeyedResidual::NotElided) => {}
        }
    }
    Ok(Some(ResolvedTransaction { id, date, legs }))
}

/// Grouping state threaded through the stream.
struct Grouper<'e> {
    /// Rows in `transaction_id` order.
    rows: BoxStream<'e, Result<SqliteRow, sqlx::Error>>,
    /// The first row of the next transaction, read while closing the last.
    pending: Option<SqliteRow>,
    /// Account handles.
    accounts: Interner,
    /// Commodity handles.
    commodities: Interner,
    /// Reused per-transaction leg buffer.
    buffer: Vec<RawLeg>,
}

impl Grouper<'_> {
    /// Reads rows until one in-scope transaction is complete.
    async fn next_transaction(&mut self) -> BcResult<Option<ResolvedTransaction>> {
        loop {
            let first = match self.pending.take() {
                Some(row) => row,
                None => match self.rows.try_next().await? {
                    Some(row) => row,
                    None => return Ok(None),
                },
            };
            let id: Box<str> = first.try_get::<&str, _>(1)?.into();
            let date_text: Option<&str> = first.try_get(3)?;
            let date = date_text
                .ok_or_else(|| BcError::BadData(format!("transaction '{id}' has an undated leg")))?
                .parse::<Date>()
                .map_err(|e| {
                    BcError::BadData(format!("invalid date on transaction '{id}': {e}"))
                })?;
            self.buffer.clear();
            self.buffer.push(decode_leg(
                &first,
                &mut self.accounts,
                &mut self.commodities,
            )?);
            while let Some(row) = self.rows.try_next().await? {
                if row.try_get::<&str, _>(1)? != &*id {
                    self.pending = Some(row);
                    break;
                }
                self.buffer
                    .push(decode_leg(&row, &mut self.accounts, &mut self.commodities)?);
            }
            if let Some(resolved) = resolve(id, date, &mut self.buffer)? {
                return Ok(Some(resolved));
            }
        }
    }
}

/// Streams every transaction with a leg in `filter`'s scope, residuals resolved.
///
/// One `SELECT`, so the whole stream reads one snapshot. Pass a transaction as
/// `executor` when a second query must agree with it.
///
/// # Arguments
///
/// * `executor` - Pool, connection or transaction to read on.
/// * `filter` - Selects transactions and the legs to yield.
///
/// # Returns
///
/// A stream of [`ResolvedTransaction`] in `transaction_id` order.
///
/// # Errors
///
/// Building fails with [`BcError::BadData`] if an account id list cannot be
/// serialised. Each item fails with [`BcError::Database`] on a query error, or
/// [`BcError::BadData`] on an unparsable stored value, a half-null amount, or
/// an overflowing residual.
pub(crate) fn resolved_transactions<'e, E>(
    executor: E,
    filter: &LegFilter<'_>,
) -> BcResult<impl Stream<Item = BcResult<ResolvedTransaction>> + 'e>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite> + 'e,
{
    let LegQuery { sql, binds } = leg_query(filter)?;
    let mut query = sqlx::query(sqlx::AssertSqlSafe(sql));
    for bind in binds {
        query = query.bind(bind);
    }
    let grouper = Grouper {
        rows: query.fetch(executor),
        pending: None,
        accounts: Interner::default(),
        commodities: Interner::default(),
        buffer: Vec::new(),
    };
    Ok(futures_util::stream::try_unfold(
        grouper,
        |mut state| async move { Ok(state.next_transaction().await?.map(|tx| (tx, state))) },
    ))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[expect(
    clippy::indexing_slicing,
    reason = "tests index fixtures of known length"
)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    /// Returns the `detail` column of every `EXPLAIN QUERY PLAN` row.
    async fn query_plan(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
        sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(pool)
            .await
            .expect("explain query plan")
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn ledger_query_walks_the_transaction_index_without_sorting(pool: sqlx::SqlitePool) {
        let query = leg_query(&LegFilter::new(LegScope::Ledger)).expect("query");
        let plan = query_plan(&pool, &query.sql).await;
        assert!(
            plan.iter().any(|d| d.contains("idx_postings_transaction")),
            "expected the transaction index: {plan:?}"
        );
        assert!(
            !plan.iter().any(|d| d.contains("TEMP B-TREE")),
            "ledger order must come from the index: {plan:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn scoped_selection_uses_an_account_index(pool: sqlx::SqlitePool) {
        let id = AccountId::new();
        let ids = [id.clone()];
        let tag = TagId::new();
        let from = jiff::civil::date(2026, 1, 1);
        let to = jiff::civil::date(2026, 2, 1);
        for filter in [
            LegFilter::new(LegScope::Accounts(&ids)),
            LegFilter::new(LegScope::Accounts(&ids)).window(from, to),
            LegFilter::new(LegScope::Subtree(&id)).window(from, to),
            LegFilter::new(LegScope::Subtree(&id))
                .window(from, to)
                .tag(Some(&tag)),
        ] {
            let query = leg_query(&filter).expect("query");
            let plan = query_plan(&pool, &query.sql).await;
            assert!(
                !plan
                    .iter()
                    .any(|d| d.starts_with("SCAN e") || d.starts_with("SCAN p")),
                "scoped selection must not scan postings: {plan:?}"
            );
            assert!(
                plan.iter().any(|d| d.contains("idx_postings_account_date")
                    || d.contains("idx_postings_account_commodity_date")),
                "scoped selection must use an account index: {plan:?}"
            );
        }
    }

    #[test]
    fn ledger_query_binds_nothing() {
        let query = leg_query(&LegFilter::new(LegScope::Ledger)).expect("query");
        assert_eq!(query.binds, Vec::<String>::new());
        assert!(!query.sql.contains("WHERE"));
    }

    #[test]
    fn windowed_subtree_with_tag_binds_root_window_then_tag() {
        let root = AccountId::new();
        let tag = TagId::new();
        let from = jiff::civil::date(2026, 1, 1);
        let to = jiff::civil::date(2026, 2, 1);
        let filter = LegFilter::new(LegScope::Subtree(&root))
            .window(from, to)
            .tag(Some(&tag));
        let query = leg_query(&filter).expect("query");
        assert_eq!(
            query.binds,
            vec![
                root.to_string(),
                "2026-01-01".to_owned(),
                "2026-02-01".to_owned(),
                tag.to_string()
            ]
        );
        assert!(query.sql.contains("?4"));
    }

    // MARK: Stream

    async fn insert_tx(pool: &sqlx::SqlitePool, id: &str, date: &str) {
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, ?, 'Test', 'unreconciled', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(date)
        .execute(pool)
        .await
        .expect("insert transaction");
    }

    /// Inserts a leg; `amount` is `None` for an elided leg.
    async fn insert_leg(
        pool: &sqlx::SqlitePool,
        id: &str,
        tx_id: &str,
        account: &AccountId,
        amount: Option<(&str, &str)>,
        position: i64,
    ) {
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(tx_id)
        .bind(account.to_string())
        .bind(amount.map(|(v, _)| v))
        .bind(amount.map(|(_, c)| c))
        .bind(position)
        .execute(pool)
        .await
        .expect("insert posting");
    }

    /// Inserts a concrete leg carrying a price annotation.
    async fn insert_priced_leg(
        pool: &sqlx::SqlitePool,
        id: &str,
        tx_id: &str,
        account: &AccountId,
        amount: (&str, &str),
        price: (&str, &str, &str),
        position: i64,
    ) {
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, \
             price_value, price_commodity, price_kind, position) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(tx_id)
        .bind(account.to_string())
        .bind(amount.0)
        .bind(amount.1)
        .bind(price.0)
        .bind(price.1)
        .bind(price.2)
        .bind(position)
        .execute(pool)
        .await
        .expect("insert priced posting");
    }

    async fn make_account(
        pool: &sqlx::SqlitePool,
        name: &str,
        account_type: AccountType,
        parent: Option<&AccountId>,
    ) -> AccountId {
        let svc = crate::account::Service::new(pool.clone());
        let builder = svc
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount);
        match parent {
            Some(p) => builder.parent_id(p).call().await,
            None => builder.call().await,
        }
        .expect("create account")
    }

    /// Drains a stream into a vector.
    async fn collect(pool: &sqlx::SqlitePool, filter: &LegFilter<'_>) -> Vec<ResolvedTransaction> {
        resolved_transactions(pool, filter)
            .expect("build stream")
            .try_collect()
            .await
            .expect("drain stream")
    }

    /// `(account id, commodity, value, source)` for compact asserts.
    fn summary(tx: &ResolvedTransaction) -> Vec<(String, Option<String>, Decimal, LegSource)> {
        tx.legs()
            .iter()
            .map(|l| {
                (
                    l.account().as_str().to_owned(),
                    l.commodity().map(|c| c.as_str().to_owned()),
                    l.value(),
                    l.source(),
                )
            })
            .collect()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_absorbs_the_negated_sibling(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_leg(&pool, "p_food", "tx_1", &food, Some(("50.00", "AUD")), 0).await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, None, 1).await;

        let txs = collect(&pool, &LegFilter::new(LegScope::Ledger)).await;

        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id(), "tx_1");
        assert_eq!(txs[0].date(), jiff::civil::date(2026, 1, 1));
        let postings: Vec<i64> = txs[0].legs().iter().map(ResolvedLeg::posting).collect();
        assert_eq!(postings.len(), 2);
        assert!(postings[0] != postings[1], "{postings:?}");
        assert_eq!(
            summary(&txs[0]),
            vec![
                (
                    food.to_string(),
                    Some("AUD".to_owned()),
                    dec!(50.00),
                    LegSource::Concrete
                ),
                (
                    bank.to_string(),
                    Some("AUD".to_owned()),
                    dec!(-50.00),
                    LegSource::Residual
                ),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn ambiguous_transaction_yields_a_marker_per_elided_leg(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        let fees = make_account(&pool, "Fees", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, Some(("-50.00", "AUD")), 0).await;
        insert_leg(&pool, "p_food", "tx_1", &food, None, 1).await;
        insert_leg(&pool, "p_fees", "tx_1", &fees, None, 2).await;

        let ids = [food.clone()];
        let txs = collect(&pool, &LegFilter::new(LegScope::Accounts(&ids))).await;

        assert_eq!(
            summary(&txs[0]),
            vec![(food.to_string(), None, Decimal::ZERO, LegSource::Ambiguous)]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn multi_commodity_residual_yields_one_leg_per_commodity(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_leg(&pool, "p_a", "tx_1", &food, Some(("3.00", "AUD")), 0).await;
        insert_leg(&pool, "p_u", "tx_1", &food, Some(("7.00", "USD")), 1).await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, None, 2).await;

        let ids = [bank.clone()];
        let txs = collect(&pool, &LegFilter::new(LegScope::Accounts(&ids))).await;

        assert_eq!(
            summary(&txs[0]),
            vec![
                (
                    bank.to_string(),
                    Some("AUD".to_owned()),
                    dec!(-3.00),
                    LegSource::Residual
                ),
                (
                    bank.to_string(),
                    Some("USD".to_owned()),
                    dec!(-7.00),
                    LegSource::Residual
                ),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn priced_sibling_funds_the_residual_at_its_weight(pool: sqlx::SqlitePool) {
        // -2 ETH @ 300 AUD weighs -600 AUD; +590 AUD cash; the gains leg takes +10 AUD.
        let wallet = make_account(&pool, "Wallet", AccountType::Asset, None).await;
        let cash = make_account(&pool, "Cash", AccountType::Asset, None).await;
        let gains = make_account(&pool, "Gains", AccountType::Income, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_priced_leg(
            &pool,
            "p_eth",
            "tx_1",
            &wallet,
            ("-2", "ETH"),
            ("300", "AUD", "unit"),
            0,
        )
        .await;
        insert_leg(&pool, "p_cash", "tx_1", &cash, Some(("590", "AUD")), 1).await;
        insert_leg(&pool, "p_gain", "tx_1", &gains, None, 2).await;

        let txs = collect(&pool, &LegFilter::new(LegScope::Ledger)).await;

        // The concrete ETH leg reports its stored amount, not its weight.
        assert_eq!(
            summary(&txs[0]),
            vec![
                (
                    wallet.to_string(),
                    Some("ETH".to_owned()),
                    dec!(-2),
                    LegSource::Concrete
                ),
                (
                    cash.to_string(),
                    Some("AUD".to_owned()),
                    dec!(590),
                    LegSource::Concrete
                ),
                (
                    gains.to_string(),
                    Some("AUD".to_owned()),
                    dec!(10),
                    LegSource::Residual
                ),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn zero_residual_still_yields_the_transaction(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_leg(&pool, "p_in", "tx_1", &food, Some(("5.00", "AUD")), 0).await;
        insert_leg(&pool, "p_out", "tx_1", &food, Some(("-5.00", "AUD")), 1).await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, None, 2).await;

        let ids = [bank.clone()];
        let txs = collect(&pool, &LegFilter::new(LegScope::Accounts(&ids))).await;

        assert_eq!(txs.len(), 1);
        let empty: &[ResolvedLeg] = &[];
        assert_eq!(txs[0].legs(), empty);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn scope_yields_only_in_scope_legs_and_skips_untouched_transactions(
        pool: sqlx::SqlitePool,
    ) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        let rent = make_account(&pool, "Rent", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_leg(&pool, "p_food", "tx_1", &food, Some(("50.00", "AUD")), 0).await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, None, 1).await;
        insert_tx(&pool, "tx_2", "2026-01-02").await;
        insert_leg(&pool, "p_rent", "tx_2", &rent, Some(("10.00", "AUD")), 0).await;
        insert_leg(&pool, "p_bank2", "tx_2", &bank, Some(("-10.00", "AUD")), 1).await;

        let ids = [food.clone()];
        let txs = collect(&pool, &LegFilter::new(LegScope::Accounts(&ids))).await;

        assert_eq!(txs.len(), 1);
        assert_eq!(
            summary(&txs[0]),
            vec![(
                food.to_string(),
                Some("AUD".to_owned()),
                dec!(50.00),
                LegSource::Concrete
            )]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn window_is_half_open(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        for (tx, date) in [
            ("tx_before", "2025-12-31"),
            ("tx_from", "2026-01-01"),
            ("tx_until", "2026-02-01"),
        ] {
            insert_tx(&pool, tx, date).await;
            insert_leg(
                &pool,
                &format!("{tx}_f"),
                tx,
                &food,
                Some(("1.00", "AUD")),
                0,
            )
            .await;
            insert_leg(&pool, &format!("{tx}_b"), tx, &bank, None, 1).await;
        }

        let ids = [bank.clone()];
        let filter = LegFilter::new(LegScope::Accounts(&ids))
            .window(jiff::civil::date(2026, 1, 1), jiff::civil::date(2026, 2, 1));
        let txs = collect(&pool, &filter).await;

        let found: Vec<&str> = txs.iter().map(ResolvedTransaction::id).collect();
        assert_eq!(found, vec!["tx_from"]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn subtree_reaches_a_grandchild_and_skips_a_sibling_subtree(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        let cafes = make_account(&pool, "Cafes", AccountType::Expense, Some(&food)).await;
        let espresso = make_account(&pool, "Espresso", AccountType::Expense, Some(&cafes)).await;
        let rent = make_account(&pool, "Rent", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-05").await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, Some(("-4.00", "AUD")), 0).await;
        insert_leg(&pool, "p_esp", "tx_1", &espresso, None, 1).await;
        insert_tx(&pool, "tx_2", "2026-01-06").await;
        insert_leg(&pool, "p_bank2", "tx_2", &bank, Some(("-9.00", "AUD")), 0).await;
        insert_leg(&pool, "p_rent", "tx_2", &rent, None, 1).await;

        let txs = collect(&pool, &LegFilter::new(LegScope::Subtree(&food))).await;

        assert_eq!(txs.len(), 1);
        assert_eq!(
            summary(&txs[0]),
            vec![(
                espresso.to_string(),
                Some("AUD".to_owned()),
                dec!(4.00),
                LegSource::Residual
            )]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tagged_elided_leg_is_funded_by_an_untagged_sibling(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        let organic = TagId::new();
        sqlx::query(
            "INSERT INTO tags (id, name, created_at) VALUES (?, 'organic', '2026-01-01T00:00:00Z')",
        )
        .bind(organic.to_string())
        .execute(&pool)
        .await
        .expect("insert tag");
        insert_tx(&pool, "tx_1", "2026-01-05").await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, Some(("-25.00", "AUD")), 0).await;
        insert_leg(&pool, "p_food", "tx_1", &food, None, 1).await;
        sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES ('p_food', ?)")
            .bind(organic.to_string())
            .execute(&pool)
            .await
            .expect("tag elided leg");

        let filter = LegFilter::new(LegScope::Subtree(&food)).tag(Some(&organic));
        let txs = collect(&pool, &filter).await;

        assert_eq!(
            summary(&txs[0]),
            vec![(
                food.to_string(),
                Some("AUD".to_owned()),
                dec!(25.00),
                LegSource::Residual
            )]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reinserted_legs_still_group_with_their_transaction(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_a", "2026-01-01").await;
        insert_leg(&pool, "p_a1", "tx_a", &food, Some(("5.00", "AUD")), 0).await;
        insert_tx(&pool, "tx_b", "2026-01-02").await;
        insert_leg(&pool, "p_b1", "tx_b", &food, Some(("7.00", "AUD")), 0).await;
        insert_leg(&pool, "p_b2", "tx_b", &bank, None, 1).await;
        // tx_a's second leg arrives after all of tx_b, as an edit would leave it.
        insert_leg(&pool, "p_a2", "tx_a", &bank, None, 1).await;

        let txs = collect(&pool, &LegFilter::new(LegScope::Ledger)).await;

        let shape: Vec<(&str, usize)> = txs.iter().map(|t| (t.id(), t.legs().len())).collect();
        assert_eq!(shape, vec![("tx_a", 2), ("tx_b", 2)]);
        assert_eq!(txs[0].legs()[1].value(), dec!(-5.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn interned_indices_are_dense_and_shared_per_kind(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        for (tx, date) in [("tx_1", "2026-01-01"), ("tx_2", "2026-01-02")] {
            insert_tx(&pool, tx, date).await;
            insert_leg(
                &pool,
                &format!("{tx}_f"),
                tx,
                &food,
                Some(("1.00", "AUD")),
                0,
            )
            .await;
            insert_leg(&pool, &format!("{tx}_b"), tx, &bank, None, 1).await;
        }

        let txs = collect(&pool, &LegFilter::new(LegScope::Ledger)).await;

        let legs: Vec<&ResolvedLeg> = txs.iter().flat_map(ResolvedTransaction::legs).collect();
        let accounts: Vec<u32> = legs.iter().map(|l| l.account().idx()).collect();
        let commodities: Vec<u32> = legs
            .iter()
            .filter_map(|l| l.commodity().map(Interned::idx))
            .collect();
        assert_eq!(accounts, vec![0, 1, 0, 1]);
        assert_eq!(commodities, vec![0, 0, 0, 0]);
    }

    /// Drains a stream that must fail, returning its error.
    async fn drain_err(pool: &sqlx::SqlitePool, filter: &LegFilter<'_>) -> BcError {
        resolved_transactions(pool, filter)
            .expect("build stream")
            .try_collect::<Vec<_>>()
            .await
            .expect_err("stream must fail")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unparsable_amount_fails_with_bad_data(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_leg(&pool, "p_food", "tx_1", &food, Some(("12.3.4", "AUD")), 0).await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, None, 1).await;

        let err = drain_err(&pool, &LegFilter::new(LegScope::Ledger)).await;

        assert!(
            matches!(&err, BcError::BadData(m) if m.contains("invalid amount")),
            "got {err:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unparsable_transaction_date_fails_with_bad_data(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset, None).await;
        let food = make_account(&pool, "Food", AccountType::Expense, None).await;
        insert_tx(&pool, "tx_1", "2026-13-45").await;
        insert_leg(&pool, "p_food", "tx_1", &food, Some(("5.00", "AUD")), 0).await;
        insert_leg(&pool, "p_bank", "tx_1", &bank, None, 1).await;

        let err = drain_err(&pool, &LegFilter::new(LegScope::Ledger)).await;

        assert!(
            matches!(&err, BcError::BadData(m) if m.contains("invalid date")),
            "got {err:?}"
        );
    }
}
