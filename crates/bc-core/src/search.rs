//! Structured transaction search: query types, per-leg match attribution, and
//! the `Service::search` query surface.

use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::TagId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::civil::Date;
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;

use crate::BcResult;
use crate::db::to_db_str;
use crate::transaction::Service;
use crate::transaction::TxRow;
use crate::transaction::sql_placeholders;

/// Magnitude predicate for the amount dimension (parsed from `bc_ipc::AmountFilter`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct AmountQuery {
    /// Inclusive lower bound on the magnitude.
    pub min: Option<Decimal>,
    /// Inclusive upper bound on the magnitude.
    pub max: Option<Decimal>,
    /// Restrict to a single commodity when set.
    pub commodity: Option<CommodityCode>,
}

/// A parsed transaction query: `bc_ipc::Filter` with ids resolved to domain types.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct TransactionQuery {
    /// Inclusive lower date bound.
    pub date_from: Option<Date>,
    /// Exclusive upper date bound.
    pub date_until: Option<Date>,
    /// Account ids; each matches its subtree; multiple union.
    pub accounts: Vec<AccountId>,
    /// Tag ids; multiple union.
    pub tags: Vec<TagId>,
    /// Case-insensitive substring over payee OR narration.
    pub text: Option<String>,
    /// Magnitude predicate.
    pub amount: Option<AmountQuery>,
    /// Exact reconciliation status.
    pub reconciliation: Option<Reconciliation>,
}

impl TransactionQuery {
    /// Constructs a query scoped to a date window, accounts, and tags.
    ///
    /// `#[non_exhaustive]` blocks struct-literal construction outside this
    /// crate, so callers assembling a query from CLI flags go through this
    /// constructor instead. Every other filter is left unset.
    ///
    /// # Arguments
    ///
    /// * `date_from` - Inclusive lower date bound.
    /// * `date_until` - Exclusive upper date bound.
    /// * `accounts` - Account ids; each matches its subtree; multiple union.
    /// * `tags` - Tag ids; multiple union.
    ///
    /// # Returns
    ///
    /// The constructed [`TransactionQuery`].
    #[inline]
    #[must_use]
    pub fn windowed(
        date_from: Option<Date>,
        date_until: Option<Date>,
        accounts: Vec<AccountId>,
        tags: Vec<TagId>,
    ) -> Self {
        Self {
            date_from,
            date_until,
            accounts,
            tags,
            ..Self::default()
        }
    }
}

impl AmountQuery {
    /// Returns whether `amount`'s magnitude falls in `[min, max]` and, if a
    /// commodity is set, matches it. An elided (`None`) amount never matches.
    #[must_use]
    #[expect(
        clippy::shadow_reuse,
        reason = "narrowing Option<&Amount> to &Amount under the same name reads clearer than a fresh name"
    )]
    pub fn matches(&self, amount: Option<&Amount>) -> bool {
        let Some(amount) = amount else { return false };
        if let Some(c) = &self.commodity
            && amount.commodity() != c
        {
            return false;
        }
        let magnitude = amount.value().abs();
        if let Some(min) = self.min
            && magnitude < min
        {
            return false;
        }
        if let Some(max) = self.max
            && magnitude > max
        {
            return false;
        }
        true
    }
}

/// Returns the set of posting ids that match the active posting-scoped
/// predicates. An empty set means the transaction is excluded.
///
/// A leg matches iff, for every active posting-scoped dimension, it satisfies
/// that dimension — where the tag dimension is satisfied when a filter tag hits
/// at the transaction level (all legs) or on that leg. With no posting-scoped
/// dimension active, all legs match.
///
/// # Arguments
///
/// * `tx` - The hydrated transaction (all legs present).
/// * `accounts` - The resolved account-subtree id set, or `None` if inactive.
/// * `amount` - The amount predicate, or `None` if inactive.
/// * `tags` - The selected tag id set, or `None` if inactive.
#[must_use]
#[expect(
    clippy::implicit_hasher,
    reason = "callers always use the default std HashSet hasher"
)]
pub fn compute_matched_postings(
    tx: &Transaction,
    accounts: Option<&HashSet<AccountId>>,
    amount: Option<&AmountQuery>,
    tags: Option<&HashSet<TagId>>,
) -> HashSet<PostingId> {
    if accounts.is_none() && amount.is_none() && tags.is_none() {
        return tx.postings().iter().map(|p| p.id().clone()).collect();
    }

    let tx_level_tag_hit = tags.is_some_and(|set| tx.tag_ids().iter().any(|t| set.contains(t)));

    tx.postings()
        .iter()
        .filter(|p| leg_matches(p, accounts, amount, tags, tx_level_tag_hit))
        .map(|p| p.id().clone())
        .collect()
}

/// Whether a single posting satisfies every active posting-scoped dimension.
fn leg_matches(
    posting: &Posting,
    accounts: Option<&HashSet<AccountId>>,
    amount: Option<&AmountQuery>,
    tags: Option<&HashSet<TagId>>,
    tx_level_tag_hit: bool,
) -> bool {
    if let Some(set) = accounts
        && !set.contains(posting.account_id())
    {
        return false;
    }
    if let Some(q) = amount
        && !q.matches(posting.amount())
    {
        return false;
    }
    if let Some(set) = tags {
        let posting_hit = posting.tag_ids().iter().any(|t| set.contains(t));
        if !tx_level_tag_hit && !posting_hit {
            return false;
        }
    }
    true
}

/// A matched transaction plus the ids of the legs that satisfied the
/// posting-scoped predicates (all legs when the match was transaction-scoped).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MatchedTransaction {
    /// The whole matched transaction (never pruned).
    pub transaction: Transaction,
    /// The ids of the legs that matched the posting-scoped predicates.
    pub matched_postings: HashSet<PostingId>,
}

/// Escapes SQL `LIKE` *wildcard* metacharacters (`\`, `%`, `_`) in `input` so a
/// user-typed needle is matched literally inside a `LIKE ... ESCAPE '\'` pattern.
///
/// This is **not** SQL-injection escaping — that is handled separately by binding
/// the needle as a query parameter (`.bind(...)`), never by string interpolation.
/// The two concerns are orthogonal: parameter binding stops the value from being
/// parsed as SQL, but it does **not** neutralise `%` / `_`, which `LIKE` still
/// interprets as wildcards *within* a bound value (a bound `"50%"` would match
/// "50" followed by anything). Neither SQLite nor `sqlx` exposes a built-in
/// LIKE-escape helper, so the standard idiom is an explicit `ESCAPE` clause plus
/// this manual metacharacter escaping.
///
/// The backslash is escaped first so that the escapes subsequently inserted for
/// `%` and `_` are not themselves re-escaped.
pub(crate) fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Resolves each account root to its inclusive subtree and unions the results.
///
/// # Arguments
///
/// * `pool` - The SQLite connection pool.
/// * `roots` - Account roots; each expands to itself plus all descendants.
///
/// # Returns
///
/// `None` when `roots` is empty (dimension inactive), otherwise the unioned id set.
///
/// # Errors
///
/// Returns [`crate::BcError`] on database failure.
pub(crate) async fn resolve_account_subtrees(
    pool: &sqlx::SqlitePool,
    roots: &[AccountId],
) -> BcResult<Option<HashSet<AccountId>>> {
    if roots.is_empty() {
        return Ok(None);
    }
    let mut set = HashSet::new();
    for root in roots {
        let rows: Vec<(String,)> = sqlx::query_as(
            "WITH RECURSIVE subtree(id) AS ( \
                 VALUES(?) \
                 UNION ALL \
                 SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
             ) SELECT id FROM subtree",
        )
        .bind(root.to_string())
        .fetch_all(pool)
        .await?;
        for (id,) in rows {
            if let Ok(parsed) = id.parse::<AccountId>() {
                set.insert(parsed);
            }
        }
    }
    Ok(Some(set))
}

/// Resolves a tag root to its inclusive subtree (the tag plus all descendants).
///
/// # Arguments
///
/// * `pool` - The SQLite connection pool.
/// * `root` - The tag whose subtree to resolve.
///
/// # Returns
///
/// The set containing `root` and every descendant tag id.
///
/// # Errors
///
/// Returns [`crate::BcError`] on database failure.
pub(crate) async fn resolve_tag_subtree(
    pool: &sqlx::SqlitePool,
    root: &TagId,
) -> BcResult<HashSet<TagId>> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "WITH RECURSIVE subtree(id) AS ( \
             VALUES(?) \
             UNION ALL \
             SELECT tg.id FROM tags tg JOIN subtree s ON tg.parent_id = s.id \
         ) SELECT id FROM subtree",
    )
    .bind(root.to_string())
    .fetch_all(pool)
    .await?;
    let mut set = HashSet::new();
    for (id,) in rows {
        if let Ok(parsed) = id.parse::<TagId>() {
            set.insert(parsed);
        }
    }
    Ok(set)
}

impl Service {
    /// Runs a structured transaction query, returning whole matched transactions
    /// with per-leg match attribution (see [`compute_matched_postings`]).
    ///
    /// The query never prunes legs; strictness is a presentation concern.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data-parse failure.
    #[expect(
        clippy::too_many_lines,
        reason = "assembling dynamic candidate SQL with a clause per active dimension inherently requires several bind/clause blocks"
    )]
    pub async fn search(&self, query: &TransactionQuery) -> BcResult<Vec<MatchedTransaction>> {
        // 1. Resolve account subtrees to a concrete id set.
        let account_set = resolve_account_subtrees(self.pool(), &query.accounts).await?;

        // 2. Build candidate SQL with a WHERE clause per active dimension.
        let mut clauses: Vec<String> = Vec::new();
        if query.date_from.is_some() {
            clauses.push("t.date >= ?".to_owned());
        }
        if query.date_until.is_some() {
            clauses.push("t.date < ?".to_owned());
        }
        if query.text.is_some() {
            clauses.push(
                "(lower(t.payee) LIKE ? ESCAPE '\\' OR lower(t.description) LIKE ? ESCAPE '\\')"
                    .to_owned(),
            );
        }
        if query.reconciliation.is_some() {
            clauses.push("t.reconciliation = ?".to_owned());
        }
        if let Some(set) = &account_set {
            let placeholders = sql_placeholders(set.len());
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM postings p WHERE p.transaction_id = t.id AND p.account_id IN ({placeholders}))"
            ));
        }
        if query.amount.is_some() {
            clauses.push(
                "EXISTS (SELECT 1 FROM postings p WHERE p.transaction_id = t.id \
                 AND p.amount IS NOT NULL \
                 AND ABS(CAST(p.amount AS REAL)) >= ? AND ABS(CAST(p.amount AS REAL)) <= ? \
                 AND (? IS NULL OR p.commodity = ?))"
                    .to_owned(),
            );
        }
        if !query.tags.is_empty() {
            let placeholders = sql_placeholders(query.tags.len());
            clauses.push(format!(
                "(EXISTS (SELECT 1 FROM transaction_tags tt WHERE tt.transaction_id = t.id AND tt.tag_id IN ({placeholders})) \
                  OR EXISTS (SELECT 1 FROM posting_tags pt JOIN postings p ON pt.posting_id = p.id \
                             WHERE p.transaction_id = t.id AND pt.tag_id IN ({placeholders})))"
            ));
        }

        let where_sql = if clauses.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", clauses.join(" AND "))
        };
        let sql = format!(
            "SELECT t.id, t.date, t.payee, t.description, t.note, t.reconciliation, t.created_at \
             FROM transactions t {where_sql} ORDER BY t.date DESC, t.id ASC"
        );

        // 3. Bind values in clause order.
        let mut stmt = sqlx::query_as::<_, TxRow>(sqlx::AssertSqlSafe(sql));
        if let Some(from) = query.date_from {
            stmt = stmt.bind(from.to_string());
        }
        if let Some(until) = query.date_until {
            stmt = stmt.bind(until.to_string());
        }
        if let Some(text) = &query.text {
            // Fold the needle with `to_ascii_lowercase` to match SQLite's `lower()`,
            // which is ASCII-only. Using Rust's full-Unicode `to_lowercase` here
            // would desync the two sides (needle `É`->`é` vs column `É`->`É`) and
            // silently miss non-ASCII text. Consequence: non-ASCII letters are
            // matched case-sensitively; proper Unicode folding would need an
            // ICU-backed collation (deferred, see #242).
            let needle = format!("%{}%", escape_like(&text.to_ascii_lowercase()));
            stmt = stmt.bind(needle.clone()).bind(needle);
        }
        if let Some(rec) = query.reconciliation {
            stmt = stmt.bind(to_db_str(rec)?);
        }
        if let Some(set) = &account_set {
            let mut ids: Vec<&AccountId> = set.iter().collect();
            ids.sort_by_key(ToString::to_string);
            for id in ids {
                stmt = stmt.bind(id.to_string());
            }
        }
        if let Some(amount) = &query.amount {
            // Bind as REAL magnitudes; this is only a coarse candidate filter —
            // `compute_matched_postings` (Decimal-exact) is the source of truth.
            // Widen by a small epsilon so Decimal->f64 rounding can never make
            // the SQL bound narrower than the exact Decimal comparison: the
            // coarse filter must only ever over-match, never drop a real match.
            const AMOUNT_EPSILON: f64 = 0.0001;
            #[expect(
                clippy::float_arithmetic,
                reason = "widening a coarse SQL bound by a fixed epsilon; exactness lives in compute_matched_postings"
            )]
            let min = amount
                .min
                .and_then(|d| d.to_f64())
                .map_or(f64::MIN, |v| v - AMOUNT_EPSILON);
            #[expect(
                clippy::float_arithmetic,
                reason = "widening a coarse SQL bound by a fixed epsilon; exactness lives in compute_matched_postings"
            )]
            let max = amount
                .max
                .and_then(|d| d.to_f64())
                .map_or(f64::MAX, |v| v + AMOUNT_EPSILON);
            let commodity = amount.commodity.as_ref().map(|c| c.as_str().to_owned());
            stmt = stmt
                .bind(min)
                .bind(max)
                .bind(commodity.clone())
                .bind(commodity);
        }
        if !query.tags.is_empty() {
            for t in &query.tags {
                stmt = stmt.bind(t.to_string());
            }
            for t in &query.tags {
                stmt = stmt.bind(t.to_string());
            }
        }

        let tx_rows = stmt.fetch_all(self.pool()).await?;

        // 4. Hydrate whole transactions, then attribute matched legs in Rust.
        let hydrated: Vec<Transaction> = self.assemble_transactions(tx_rows).await?.collect();
        let tag_set: Option<HashSet<TagId>> =
            (!query.tags.is_empty()).then(|| query.tags.iter().cloned().collect());

        let out = hydrated
            .into_iter()
            .filter_map(|transaction| {
                let matched = compute_matched_postings(
                    &transaction,
                    account_set.as_ref(),
                    query.amount.as_ref(),
                    tag_set.as_ref(),
                );
                (!matched.is_empty()).then_some(MatchedTransaction {
                    transaction,
                    matched_postings: matched,
                })
            })
            .collect();
        Ok(out)
    }

    /// Computes filtered [`PeriodStats`](crate::balance::PeriodStats) for
    /// `account_id` in `commodity` over the window `[from, until)`.
    ///
    /// The filter selects a transaction set via [`Self::search`]; this method
    /// scopes that set to transactions touching `account_id` and sums the
    /// account's own legs, bucketing by the window edge. The query's own date
    /// bounds are ignored — `from`/`until` are the authority (the lower bound is
    /// dropped from the search so pre-window legs feed the opening balance).
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose legs are aggregated.
    /// * `commodity` - Commodity code; legs in other commodities are ignored in the sums.
    /// * `query` - The active query; its non-date dimensions and accounts drive membership.
    /// * `from` - Inclusive window start (use [`jiff::civil::Date::MIN`] for an open start).
    /// * `until` - Exclusive window end (use [`jiff::civil::Date::MAX`] for an open end).
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data-parse failure, or if a
    /// running total overflows [`Decimal`]'s range.
    pub async fn filtered_period_stats(
        &self,
        account_id: &AccountId,
        commodity: &str,
        query: &TransactionQuery,
        from: Date,
        until: Date,
    ) -> BcResult<crate::balance::PeriodStats> {
        let mut q = query.clone();
        q.date_from = None;
        q.date_until = Some(until);
        let matched = self.search(&q).await?;

        let mut opening = Decimal::ZERO;
        let mut income = Decimal::ZERO;
        let mut expenses = Decimal::ZERO;
        let mut in_window_txns: HashSet<TransactionId> = HashSet::new();

        for m in &matched {
            let tx = &m.transaction;
            let date = tx.date();
            let mut touches_in_window = false;
            let residual =
                crate::residual::residual_of(tx.postings().iter().map(bc_models::Posting::amount));
            for posting in tx.postings() {
                if posting.account_id() != account_id {
                    continue;
                }
                if date >= from && date < until {
                    touches_in_window = true;
                }
                let value = if let Some(amount) = posting.amount() {
                    if amount.commodity().as_str() != commodity {
                        continue;
                    }
                    amount.value()
                } else {
                    let Ok(crate::residual::Residual::Attributable(ref balances)) = residual else {
                        continue;
                    };
                    let Some(value) = balances.get(commodity) else {
                        continue;
                    };
                    value
                };
                if date < from {
                    opening = opening
                        .checked_add(value)
                        .ok_or_else(|| crate::BcError::BadData("opening overflow".into()))?;
                } else if date < until {
                    if value >= Decimal::ZERO {
                        income = income
                            .checked_add(value)
                            .ok_or_else(|| crate::BcError::BadData("income overflow".into()))?;
                    } else {
                        expenses = expenses
                            .checked_sub(value)
                            .ok_or_else(|| crate::BcError::BadData("expenses overflow".into()))?;
                    }
                }
            }
            if touches_in_window {
                in_window_txns.insert(tx.id().clone());
            }
        }

        let net = income
            .checked_sub(expenses)
            .ok_or_else(|| crate::BcError::BadData("net overflow".into()))?;
        let closing = opening
            .checked_add(net)
            .ok_or_else(|| crate::BcError::BadData("closing overflow".into()))?;

        Ok(crate::balance::PeriodStats {
            income: Amount::new(income, commodity),
            expenses: Amount::new(expenses, commodity),
            net: Amount::new(net, commodity),
            opening: Amount::new(opening, commodity),
            closing: Amount::new(closing, commodity),
            tx_count: u32::try_from(in_window_txns.len()).unwrap_or(u32::MAX),
        })
    }

    /// Computes filtered cash-flow [`PostingBucket`](crate::balance::PostingBucket)s
    /// for `account_id` in `commodity`, `count` buckets of `period` trailing from
    /// `as_of`.
    ///
    /// The filter selects a transaction set via [`Self::search`]; this method
    /// scopes that set to transactions touching `account_id` and buckets the
    /// account's own legs by date into inflow (positive) / outflow (`|negative|`).
    /// `matched_postings` decides membership only, never which legs are summed.
    /// The query's own date bounds are overridden with the bucket span, so the
    /// bucket ranges are the single date authority.
    ///
    /// # Bucket span vs. filter range
    ///
    /// Buckets are snapped to calendar boundaries, so the span
    /// `[oldest_bucket_start, newest_bucket_end)` may extend beyond the filter's
    /// own `date_from`/`date_until` **at both ends**. The oldest bucket can begin
    /// before `date_from`, and — less obviously — the newest bucket can end after
    /// `date_until`: with `before:2025-02-11` and weekly buckets, the span runs to
    /// 2025-02-17, so legs dated 2025-02-11..16 land in the newest bar even though
    /// the filter excludes them.
    ///
    /// This is deliberate. The sparkline is a trend chart, where calendar-aligned
    /// bars are worth more than bars clipped to the filter's exact range. The
    /// consequence is that the sparkline may not tie out exactly against the
    /// balance tiles rendered from [`Self::filtered_period_stats`], which honours
    /// the requested window edges precisely.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose legs are bucketed.
    /// * `commodity` - Commodity code; legs in other commodities are ignored.
    /// * `query` - The active query; its non-date dimensions and accounts drive membership.
    /// * `period` - Bucket width.
    /// * `count` - Number of buckets.
    /// * `as_of` - Reference date; the newest bucket contains it.
    ///
    /// # Returns
    ///
    /// A `Vec` of [`PostingBucket`](crate::balance::PostingBucket), oldest-first,
    /// of length `count`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data-parse failure, or if a
    /// bucket total overflows [`Decimal`]'s range.
    pub async fn filtered_posting_buckets(
        &self,
        account_id: &AccountId,
        commodity: &str,
        query: &TransactionQuery,
        period: &bc_models::Period,
        count: core::num::NonZeroUsize,
        as_of: Date,
    ) -> BcResult<Vec<crate::balance::PostingBucket>> {
        let ranges = crate::balance::bucket_ranges(period, count, as_of);
        let (Some(&(earliest_start, _)), Some(&(_, latest_end))) = (ranges.first(), ranges.last())
        else {
            return Ok(vec![]);
        };

        let mut q = query.clone();
        q.date_from = Some(earliest_start);
        q.date_until = Some(latest_end);
        let matched = self.search(&q).await?;

        // Per-bucket (inflow, outflow) accumulators aligned with `ranges`.
        let mut acc: Vec<(Decimal, Decimal)> = vec![(Decimal::ZERO, Decimal::ZERO); ranges.len()];

        for m in &matched {
            let tx = &m.transaction;
            let date = tx.date();
            let residual =
                crate::residual::residual_of(tx.postings().iter().map(bc_models::Posting::amount));
            for posting in tx.postings() {
                if posting.account_id() != account_id {
                    continue;
                }
                let value = if let Some(amount) = posting.amount() {
                    if amount.commodity().as_str() != commodity {
                        continue;
                    }
                    amount.value()
                } else {
                    let Ok(crate::residual::Residual::Attributable(ref balances)) = residual else {
                        continue;
                    };
                    let Some(value) = balances.get(commodity) else {
                        continue;
                    };
                    value
                };
                let Some(idx) = ranges
                    .iter()
                    .position(|(start, end)| date >= *start && date < *end)
                else {
                    continue;
                };
                let Some(slot) = acc.get_mut(idx) else {
                    continue;
                };
                if value >= Decimal::ZERO {
                    slot.0 = slot
                        .0
                        .checked_add(value)
                        .ok_or_else(|| crate::BcError::BadData("inflow overflow".into()))?;
                } else {
                    slot.1 = slot
                        .1
                        .checked_sub(value)
                        .ok_or_else(|| crate::BcError::BadData("outflow overflow".into()))?;
                }
            }
        }

        Ok(ranges
            .into_iter()
            .zip(acc)
            .map(
                |((start, end), (inflow, outflow))| crate::balance::PostingBucket {
                    start,
                    end,
                    inflow: Amount::new(inflow, commodity),
                    outflow: Amount::new(outflow, commodity),
                },
            )
            .collect())
    }
}

#[cfg(test)]
mod match_tests {
    use std::collections::HashSet;

    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::TagId;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::AmountQuery;
    use super::compute_matched_postings;

    fn posting(acc: &AccountId, value: rust_decimal::Decimal, tags: Vec<TagId>) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(acc.clone())
            .amount(Amount::new(value, CommodityCode::new("AUD")))
            .tag_ids(tags)
            .build()
    }

    fn tx(postings: Vec<Posting>, tx_tags: Vec<TagId>) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 6, 1))
            .description("Test")
            .postings(postings)
            .reconciliation(Reconciliation::Unreconciled)
            .tag_ids(tx_tags)
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    fn no_posting_scoped_dims_matches_all_legs() {
        let a = AccountId::new();
        let b = AccountId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![],
        );
        let matched = compute_matched_postings(&t, None, None, None);
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn account_dim_prunes_to_matching_leg() {
        let a = AccountId::new();
        let b = AccountId::new();
        let leg_a = posting(&a, dec!(100), vec![]);
        let leg_b = posting(&b, dec!(-100), vec![]);
        let want = leg_a.id().clone();
        let t = tx(vec![leg_a, leg_b], vec![]);
        let mut set = HashSet::new();
        set.insert(a.clone());
        let matched = compute_matched_postings(&t, Some(&set), None, None);
        assert_eq!(matched.into_iter().collect::<Vec<_>>(), vec![want]);
    }

    #[test]
    fn amount_dim_matches_by_magnitude_ignoring_sign() {
        let a = AccountId::new();
        let b = AccountId::new();
        // −100 leg should match a [50, 150] magnitude window.
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![],
        );
        let q = AmountQuery {
            min: Some(dec!(50)),
            max: Some(dec!(150)),
            commodity: None,
        };
        let matched = compute_matched_postings(&t, None, Some(&q), None);
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn tx_level_tag_matches_whole_transaction() {
        let a = AccountId::new();
        let b = AccountId::new();
        let tag = TagId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![tag.clone()],
        );
        let mut set = HashSet::new();
        set.insert(tag);
        let matched = compute_matched_postings(&t, None, None, Some(&set));
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn posting_level_tag_prunes_to_tagged_leg() {
        let a = AccountId::new();
        let b = AccountId::new();
        let tag = TagId::new();
        let tagged = posting(&a, dec!(100), vec![tag.clone()]);
        let want = tagged.id().clone();
        let t = tx(vec![tagged, posting(&b, dec!(-100), vec![])], vec![]);
        let mut set = HashSet::new();
        set.insert(tag);
        let matched = compute_matched_postings(&t, None, None, Some(&set));
        assert_eq!(matched.into_iter().collect::<Vec<_>>(), vec![want]);
    }

    #[test]
    fn elided_amount_never_matches() {
        // Direct unit coverage of the `None` short-circuit in `AmountQuery::matches`.
        let q = AmountQuery {
            min: Some(dec!(1)),
            max: None,
            commodity: None,
        };
        assert!(!q.matches(None));

        // A transaction whose only leg is elided has nothing to match against an
        // active amount query, so the whole transaction is excluded.
        let a = AccountId::new();
        let elided = Posting::builder()
            .id(PostingId::new())
            .account_id(a)
            .tag_ids(vec![])
            .build();
        let t = tx(vec![elided], vec![]);
        let matched = compute_matched_postings(&t, None, Some(&q), None);
        assert!(matched.is_empty());
    }

    #[test]
    fn commodity_mismatch_excludes_leg() {
        let a = AccountId::new();
        let b = AccountId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![],
        );
        let q = AmountQuery {
            min: None,
            max: None,
            commodity: Some(CommodityCode::new("USD")),
        };
        let matched = compute_matched_postings(&t, None, Some(&q), None);
        assert!(matched.is_empty());
    }

    #[test]
    fn magnitude_window_excludes_and_is_inclusive() {
        let a = AccountId::new();
        let b = AccountId::new();
        let big = posting(&a, dec!(100), vec![]);
        let want = big.id().clone();
        let small = posting(&b, dec!(10), vec![]);
        let t = tx(vec![big, small], vec![]);

        let window = AmountQuery {
            min: Some(dec!(50)),
            max: Some(dec!(150)),
            commodity: None,
        };
        let matched = compute_matched_postings(&t, None, Some(&window), None);
        assert_eq!(matched.into_iter().collect::<Vec<_>>(), vec![want.clone()]);

        // Boundary equal to both min and max is inclusive, not exclusive.
        let boundary = AmountQuery {
            min: Some(dec!(100)),
            max: Some(dec!(100)),
            commodity: None,
        };
        let boundary_matched = compute_matched_postings(&t, None, Some(&boundary), None);
        assert_eq!(boundary_matched.into_iter().collect::<Vec<_>>(), vec![want]);
    }

    #[test]
    fn conjunction_excludes_when_no_leg_satisfies_all() {
        // Tag hits at tx level (all legs) but the account dim matches no leg → empty.
        let a = AccountId::new();
        let b = AccountId::new();
        let other = AccountId::new();
        let tag = TagId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![tag.clone()],
        );
        let mut accounts = HashSet::new();
        accounts.insert(other);
        let mut tags = HashSet::new();
        tags.insert(tag);
        let matched = compute_matched_postings(&t, Some(&accounts), None, Some(&tags));
        assert!(matched.is_empty());
    }
}

#[cfg(test)]
mod search_tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::AmountQuery;
    use super::TransactionQuery;
    use crate::balance::Engine;
    use crate::transaction::Service;

    /// Builds a two-leg AUD transaction on the given date with payee text.
    fn tx_on(
        acc_a: &bc_models::AccountId,
        acc_b: &bc_models::AccountId,
        d: Date,
        payee: &str,
        value: rust_decimal::Decimal,
    ) -> Transaction {
        tx_with_id(TransactionId::new(), acc_a, acc_b, d, payee, value)
    }

    /// Builds a two-leg AUD transaction with an explicit id, so tests can
    /// control the relationship between insertion order and id order.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "negating a test fixture's Decimal magnitude to build the offsetting leg"
    )]
    fn tx_with_id(
        id: TransactionId,
        acc_a: &bc_models::AccountId,
        acc_b: &bc_models::AccountId,
        d: Date,
        payee: &str,
        value: rust_decimal::Decimal,
    ) -> Transaction {
        Transaction::builder()
            .id(id)
            .date(d)
            .payee(payee.to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(value, CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b.clone())
                    .amount(Amount::new(-value, CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn empty_query_returns_all(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "Amazon", dec!(100)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "Coffee", dec!(20)))
            .await
            .expect("t2");

        let out = svc
            .search(&TransactionQuery::default())
            .await
            .expect("search");
        assert_eq!(out.len(), 2);
        // Empty filter => all legs matched.
        assert!(out.iter().all(|m| m.matched_postings.len() == 2));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn search_orders_same_date_by_id(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());

        // Create the two same-date transactions out of id order (larger id
        // first): `ORDER BY t.id ASC` must return them ascending by id, not by
        // creation order. Without the secondary key SQLite leaves the same-date
        // tie unspecified, so this asserts the deterministic contract the clause
        // guarantees rather than relying on an incidental scan order.
        let (lo, hi) = {
            let x = TransactionId::new();
            let y = TransactionId::new();
            if x.to_string() < y.to_string() {
                (x, y)
            } else {
                (y, x)
            }
        };
        svc.create(tx_with_id(
            hi.clone(),
            &a,
            &b,
            date(2026, 6, 1),
            "Higher id",
            dec!(100),
        ))
        .await
        .expect("hi");
        svc.create(tx_with_id(
            lo.clone(),
            &a,
            &b,
            date(2026, 6, 1),
            "Lower id",
            dec!(20),
        ))
        .await
        .expect("lo");

        let out = svc
            .search(&TransactionQuery::default())
            .await
            .expect("search");
        let ids: Vec<_> = out.iter().map(|m| m.transaction.id().to_string()).collect();

        assert_eq!(ids, vec![lo.to_string(), hi.to_string()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn text_matches_payee_case_insensitively(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "Amazon", dec!(100)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "Coffee", dec!(20)))
            .await
            .expect("t2");

        let query = TransactionQuery {
            text: Some("amaz".to_owned()),
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.first().expect("one result").transaction.payee(),
            Some("Amazon")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn text_percent_is_literal_not_wildcard(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "50% off", dec!(100)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "50 dollars", dec!(20)))
            .await
            .expect("t2");

        let query = TransactionQuery {
            text: Some("50%".to_owned()),
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.first().expect("one result").transaction.payee(),
            Some("50% off")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn date_range_is_half_open(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        for d in [
            date(2026, 5, 31),
            date(2026, 6, 1),
            date(2026, 6, 30),
            date(2026, 7, 1),
        ] {
            svc.create(tx_on(&a, &b, d, "x", dec!(10)))
                .await
                .expect("t");
        }
        let query = TransactionQuery {
            date_from: Some(date(2026, 6, 1)),
            date_until: Some(date(2026, 7, 1)),
            ..Default::default()
        };
        let mut dates: Vec<_> = svc
            .search(&query)
            .await
            .expect("search")
            .into_iter()
            .map(|m| m.transaction.date())
            .collect();
        dates.sort();
        assert_eq!(dates, vec![date(2026, 6, 1), date(2026, 6, 30)]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_dim_prunes_legs(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "x", dec!(100)))
            .await
            .expect("t");

        let query = TransactionQuery {
            accounts: vec![a.clone()],
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        // Only the leg in account A is attributed as matched.
        assert_eq!(out.first().expect("one result").matched_postings.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amount_dim_matches_magnitude(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "big", dec!(500)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "small", dec!(5)))
            .await
            .expect("t2");

        let query = TransactionQuery {
            amount: Some(AmountQuery {
                min: Some(dec!(100)),
                max: None,
                commodity: None,
            }),
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.first().expect("one result").transaction.payee(),
            Some("big")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amount_boundary_value_is_not_dropped(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "boundary", dec!(100.10)))
            .await
            .expect("t1");

        let query = TransactionQuery {
            amount: Some(AmountQuery {
                min: Some(dec!(100.10)),
                max: Some(dec!(100.10)),
                commodity: None,
            }),
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        // A leg whose exact magnitude equals both min and max must not be
        // dropped by the coarse SQL filter's f64 rounding.
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.first().expect("one result").transaction.payee(),
            Some("boundary")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn text_underscore_is_literal_not_wildcard(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "a_b", dec!(100)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "axb", dec!(20)))
            .await
            .expect("t2");

        // `_` is a single-char LIKE wildcard; escaping must keep it literal so
        // "a_b" does not also match "axb".
        let query = TransactionQuery {
            text: Some("a_b".to_owned()),
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.first().expect("one result").transaction.payee(),
            Some("a_b")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reconciliation_dim_filters_by_status(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        // `tx_on` builds a Reconciled transaction; add an Unreconciled one.
        svc.create(tx_on(&a, &b, date(2026, 6, 1), "cleared", dec!(100)))
            .await
            .expect("reconciled");
        let pending = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 6, 2))
            .payee("pending".to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(20), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b.clone())
                    .amount(Amount::new(dec!(-20), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(pending).await.expect("unreconciled");

        let query = TransactionQuery {
            reconciliation: Some(Reconciliation::Unreconciled),
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        assert_eq!(
            out.first().expect("one result").transaction.payee(),
            Some("pending")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tags_dim_matches_via_transaction_tags(pool: sqlx::SqlitePool) {
        use bc_models::TagPath;

        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let tags = crate::tag::Service::new(pool.clone());
        let groceries = tags
            .create_path(&"groceries".parse::<TagPath>().expect("path"))
            .await
            .expect("tag");

        let svc = Service::new(pool.clone());
        // A transaction-level tag matches the whole transaction (all legs).
        let tagged = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 6, 1))
            .payee("with tag".to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .tag_ids(vec![groceries.clone()])
            .created_at(Timestamp::now())
            .build();
        svc.create(tagged).await.expect("tagged");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "untagged", dec!(20)))
            .await
            .expect("untagged");

        let query = TransactionQuery {
            tags: vec![groceries],
            ..Default::default()
        };
        let out = svc.search(&query).await.expect("search");
        assert_eq!(out.len(), 1);
        let hit = out.first().expect("one result");
        assert_eq!(hit.transaction.payee(), Some("with tag"));
        // A tx-level tag attributes every leg as matched.
        assert_eq!(hit.matched_postings.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_empty_query_matches_unfiltered(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        // Opening leg before the window, two in-window legs.
        svc.create(tx_on(&a, &b, date(2026, 5, 1), "pre", dec!(200)))
            .await
            .expect("t0");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "in1", dec!(100)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 3), "in2", dec!(30)))
            .await
            .expect("t2");

        let stats = svc
            .filtered_period_stats(
                &a,
                "AUD",
                &TransactionQuery::default(),
                date(2026, 6, 1),
                date(2026, 7, 1),
            )
            .await
            .expect("stats");

        // a's legs are all positive (money into A). opening = 200; income = 130; expenses = 0.
        assert_eq!(stats.opening.value(), dec!(200));
        assert_eq!(stats.income.value(), dec!(130));
        assert_eq!(stats.expenses.value(), dec!(0));
        assert_eq!(stats.net.value(), dec!(130));
        assert_eq!(stats.closing.value(), dec!(330));
        assert_eq!(stats.tx_count, 2);
        // Reduces to the unfiltered engine.
        let engine = crate::BalanceEngine::new(pool.clone());
        let real = engine
            .account_period_stats(&a, "AUD", date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("real");
        assert_eq!(stats.closing.value(), real.closing.value());
        assert_eq!(stats.opening.value(), real.opening.value());
    }

    /// Builds a two-leg transaction whose second leg (`elided_acc`) is elided,
    /// absorbing the concrete leg's negated amount as its residual.
    fn tx_with_elided(
        concrete_acc: &bc_models::AccountId,
        elided_acc: &bc_models::AccountId,
        d: Date,
        payee: &str,
        value: rust_decimal::Decimal,
    ) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(d)
            .payee(payee.to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(concrete_acc.clone())
                    .amount(Amount::new(value, CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(elided_acc.clone())
                    .maybe_amount(None)
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build()
    }

    /// A filtered and an unfiltered account page must agree on the closing
    /// balance for an account whose every posting is an elided residual leg —
    /// otherwise `filtered_period_stats` and `account_period_stats` disagree
    /// on the same underlying transactions (bug #354 finding 1).
    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_include_the_residual_for_all_elided_account(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Bank");
        let food = accts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Food");
        let svc = Service::new(pool.clone());
        // Bank is the elided leg on every transaction — the Beancount idiom.
        svc.create(tx_with_elided(
            &food,
            &bank,
            date(2026, 6, 2),
            "t1",
            dec!(50),
        ))
        .await
        .expect("t1");
        svc.create(tx_with_elided(
            &food,
            &bank,
            date(2026, 6, 10),
            "t2",
            dec!(25),
        ))
        .await
        .expect("t2");

        // A filter matching every transaction (empty query) must not collapse
        // the derived residual to zero.
        let filtered = svc
            .filtered_period_stats(
                &bank,
                "AUD",
                &TransactionQuery::default(),
                date(2026, 6, 1),
                date(2026, 7, 1),
            )
            .await
            .expect("filtered stats");

        let engine = crate::BalanceEngine::new(pool.clone());
        let real = engine
            .account_period_stats(&bank, "AUD", date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("real stats");

        assert_eq!(real.closing.value(), dec!(-75));
        assert_eq!(
            filtered.closing.value(),
            real.closing.value(),
            "filtered and unfiltered closing balances must agree for an all-elided account"
        );
        assert_eq!(filtered.expenses.value(), real.expenses.value());
    }

    /// The sparkline path must derive the same residual as the tiles path for
    /// an all-elided account (bug #354 finding 1).
    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_include_the_residual_for_all_elided_account(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let bank = accts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Bank");
        let food = accts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Food");
        let svc = Service::new(pool.clone());
        svc.create(tx_with_elided(
            &food,
            &bank,
            date(2026, 6, 2),
            "t1",
            dec!(50),
        ))
        .await
        .expect("t1");

        let count = core::num::NonZeroUsize::new(1).expect("nonzero");
        let buckets = svc
            .filtered_posting_buckets(
                &bank,
                "AUD",
                &TransactionQuery::default(),
                &Period::Monthly,
                count,
                date(2026, 6, 15),
            )
            .await
            .expect("filtered_posting_buckets");

        // Food debits 50, so Bank's elided leg absorbs a -50 residual — an outflow.
        let bucket = buckets.first().expect("one bucket");
        assert_eq!(bucket.inflow.value(), dec!(0));
        assert_eq!(bucket.outflow.value(), dec!(50));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_amount_dim_narrows_flows(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "big", dec!(500)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 3), "small", dec!(5)))
            .await
            .expect("t2");

        let query = TransactionQuery {
            amount: Some(AmountQuery {
                min: Some(dec!(100)),
                max: None,
                commodity: None,
            }),
            ..Default::default()
        };
        let stats = svc
            .filtered_period_stats(&a, "AUD", &query, date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("stats");

        // Only the 500 transaction is a member; a's leg there is +500.
        assert_eq!(stats.income.value(), dec!(500));
        assert_eq!(stats.tx_count, 1);
        assert_eq!(stats.closing.value(), dec!(500));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_foreign_account_is_zero(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let c = accts
            .create()
            .name("C")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("C");
        let svc = Service::new(pool.clone());
        // A<->B transaction only; filter targets C (which A never transacts with).
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "ab", dec!(100)))
            .await
            .expect("t1");

        let query = TransactionQuery {
            accounts: vec![c.clone()],
            ..Default::default()
        };
        let stats = svc
            .filtered_period_stats(&a, "AUD", &query, date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("stats");

        assert_eq!(stats.income.value(), dec!(0));
        assert_eq!(stats.expenses.value(), dec!(0));
        assert_eq!(stats.closing.value(), dec!(0));
        assert_eq!(stats.tx_count, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_negative_flows_and_opening(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        // Pre-window outflow from A (A receives the negative leg) -> negative opening.
        svc.create(tx_on(&b, &a, date(2026, 5, 1), "pre-out", dec!(50)))
            .await
            .expect("t0");
        // In-window inflow to A (+100) and a larger in-window outflow (-160).
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "in-inflow", dec!(100)))
            .await
            .expect("t1");
        svc.create(tx_on(&b, &a, date(2026, 6, 3), "in-outflow", dec!(160)))
            .await
            .expect("t2");

        let stats = svc
            .filtered_period_stats(
                &a,
                "AUD",
                &TransactionQuery::default(),
                date(2026, 6, 1),
                date(2026, 7, 1),
            )
            .await
            .expect("stats");

        // Exercises the expenses (`checked_sub`) branch, a negative opening, and a
        // negative net; the `closing = opening + net` invariant must still hold.
        assert_eq!(stats.opening.value(), dec!(-50));
        assert_eq!(stats.income.value(), dec!(100));
        assert_eq!(stats.expenses.value(), dec!(160));
        assert_eq!(stats.net.value(), dec!(-60));
        assert_eq!(stats.closing.value(), dec!(-110));
        assert_eq!(stats.tx_count, 2);
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "asserting the closing = opening + net invariant on small fixture Decimals"
        )]
        let expected_closing = stats.opening.value() + stats.net.value();
        assert_eq!(stats.closing.value(), expected_closing);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_tag_dim_sums_only_tagged_legs(pool: sqlx::SqlitePool) {
        use bc_models::TagPath;

        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let tags = crate::tag::Service::new(pool.clone());
        let recurring = tags
            .create_path(&"recurring".parse::<TagPath>().expect("path"))
            .await
            .expect("tag");

        // Builds a two-leg AUD transaction tagged at the transaction level, with
        // `a_amount` on A and its negation on B.
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "negating a test fixture's Decimal magnitude to build the offsetting leg"
        )]
        let tagged = |d: Date, payee: &str, a_amount: rust_decimal::Decimal| {
            Transaction::builder()
                .id(TransactionId::new())
                .date(d)
                .payee(payee.to_owned())
                .description("desc")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(a.clone())
                        .amount(Amount::new(a_amount, CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(b.clone())
                        .amount(Amount::new(-a_amount, CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .tag_ids(vec![recurring.clone()])
                .created_at(Timestamp::now())
                .build()
        };

        let svc = Service::new(pool.clone());
        // Tagged pre-window outflow -> negative opening from the tagged set only.
        svc.create(tagged(date(2026, 5, 1), "pre-out", dec!(-22)))
            .await
            .expect("pre");
        // Tagged in-window inflow (+100) and outflow (-40).
        svc.create(tagged(date(2026, 6, 2), "in-inflow", dec!(100)))
            .await
            .expect("in1");
        svc.create(tagged(date(2026, 6, 3), "in-outflow", dec!(-40)))
            .await
            .expect("in2");
        // Untagged in-window leg must be excluded entirely.
        svc.create(tx_on(&a, &b, date(2026, 6, 4), "untagged", dec!(999)))
            .await
            .expect("untagged");

        let query = TransactionQuery {
            tags: vec![recurring],
            ..Default::default()
        };
        let stats = svc
            .filtered_period_stats(&a, "AUD", &query, date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("stats");

        // Only tagged legs count: opening -22, income 100, expenses 40 -> net 60,
        // closing 38; the untagged 999 is absent.
        assert_eq!(stats.opening.value(), dec!(-22));
        assert_eq!(stats.income.value(), dec!(100));
        assert_eq!(stats.expenses.value(), dec!(40));
        assert_eq!(stats.net.value(), dec!(60));
        assert_eq!(stats.closing.value(), dec!(38));
        assert_eq!(stats.tx_count, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_one_sided_date_bounds(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        svc.create(tx_on(&a, &b, date(2026, 5, 1), "may", dec!(200)))
            .await
            .expect("t0");
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "jun", dec!(100)))
            .await
            .expect("t1");

        // Open start (`from = Date::MIN`, as the client resolves a lone `before:`):
        // nothing precedes the window, so opening is 0 and both legs are in-window.
        let open_start = svc
            .filtered_period_stats(
                &a,
                "AUD",
                &TransactionQuery::default(),
                Date::MIN,
                date(2026, 7, 1),
            )
            .await
            .expect("open start");
        assert_eq!(open_start.opening.value(), dec!(0));
        assert_eq!(open_start.income.value(), dec!(300));
        assert_eq!(open_start.closing.value(), dec!(300));
        assert_eq!(open_start.tx_count, 2);

        // Open end (`until = Date::MAX`, as the client resolves a lone `after:`):
        // the May leg is pre-window opening, June is the only in-window flow.
        let open_end = svc
            .filtered_period_stats(
                &a,
                "AUD",
                &TransactionQuery::default(),
                date(2026, 6, 1),
                Date::MAX,
            )
            .await
            .expect("open end");
        assert_eq!(open_end.opening.value(), dec!(200));
        assert_eq!(open_end.income.value(), dec!(100));
        assert_eq!(open_end.closing.value(), dec!(300));
        assert_eq!(open_end.tx_count, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_stats_amount_boundary_is_inclusive_and_exact(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        // A leg exactly at the `min` bound must be summed (coarse SQL widens by an
        // epsilon so the f64 cast can never drop it); a leg just below must not
        // (exactness lives in `AmountQuery::matches`, not the SQL candidate filter).
        svc.create(tx_on(&a, &b, date(2026, 6, 2), "at-bound", dec!(100.00)))
            .await
            .expect("t1");
        svc.create(tx_on(&a, &b, date(2026, 6, 3), "just-below", dec!(99.99)))
            .await
            .expect("t2");

        let query = TransactionQuery {
            amount: Some(AmountQuery {
                min: Some(dec!(100)),
                max: None,
                commodity: None,
            }),
            ..Default::default()
        };
        let stats = svc
            .filtered_period_stats(&a, "AUD", &query, date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("stats");

        assert_eq!(stats.income.value(), dec!(100.00));
        assert_eq!(stats.tx_count, 1);
        assert_eq!(stats.closing.value(), dec!(100.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_match_unfiltered_for_empty_query(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        // +100 on A in Jan 2025, -40 on A in Feb 2025.
        svc.create(tx_on(&a, &b, date(2025, 1, 10), "jan-in", dec!(100)))
            .await
            .expect("jan");
        svc.create(tx_on(&b, &a, date(2025, 2, 5), "feb-out", dec!(40)))
            .await
            .expect("feb");

        let engine = Engine::new(pool.clone());
        let count = core::num::NonZeroUsize::new(2).expect("2 > 0");
        let as_of = date(2025, 2, 15);
        let period = Period::Monthly;

        let unfiltered = engine
            .posting_buckets(&a, "AUD", &period, count, as_of)
            .await
            .expect("posting_buckets");
        let filtered = svc
            .filtered_posting_buckets(
                &a,
                "AUD",
                &TransactionQuery::default(),
                &period,
                count,
                as_of,
            )
            .await
            .expect("filtered_posting_buckets");

        assert_eq!(filtered.len(), unfiltered.len());
        for (f, u) in filtered.iter().zip(&unfiltered) {
            assert_eq!(f.inflow.value(), u.inflow.value());
            assert_eq!(f.outflow.value(), u.outflow.value());
            assert_eq!((f.start, f.end), (u.start, u.end));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_scope_to_tag(pool: sqlx::SqlitePool) {
        use bc_models::TagPath;

        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let tags = crate::tag::Service::new(pool.clone());
        let t = tags
            .create_path(&"t".parse::<TagPath>().expect("path"))
            .await
            .expect("tag");

        let svc = Service::new(pool.clone());
        // Tagged +100 in Jan.
        let tagged = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 1, 10))
            .payee("tagged".to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .tag_ids(vec![t.clone()])
            .created_at(Timestamp::now())
            .build();
        svc.create(tagged).await.expect("tagged");
        // Untagged +50 in Jan; must be excluded since its transaction never
        // matches the tag predicate.
        svc.create(tx_on(&a, &b, date(2025, 1, 15), "untagged", dec!(50)))
            .await
            .expect("untagged");

        let query = TransactionQuery {
            tags: vec![t],
            ..Default::default()
        };
        let count = core::num::NonZeroUsize::new(1).expect("1 > 0");
        let as_of = date(2025, 1, 20);
        let period = Period::Monthly;

        let buckets = svc
            .filtered_posting_buckets(&a, "AUD", &query, &period, count, as_of)
            .await
            .expect("filtered_posting_buckets");

        assert_eq!(buckets.len(), 1);
        let jan = buckets.first().expect("one bucket");
        assert_eq!(jan.inflow.value(), dec!(100));
        assert_eq!(jan.outflow.value(), dec!(0));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_foreign_account_empty_when_no_shared_txn(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let c = accts
            .create()
            .name("C")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("C");
        let svc = Service::new(pool.clone());
        // A<->B transaction only; filter targets C (which A never transacts with).
        svc.create(tx_on(&a, &b, date(2025, 1, 10), "ab", dec!(100)))
            .await
            .expect("t1");

        let query = TransactionQuery {
            accounts: vec![c],
            ..Default::default()
        };
        let count = core::num::NonZeroUsize::new(2).expect("2 > 0");
        let as_of = date(2025, 2, 15);
        let period = Period::Monthly;

        let buckets = svc
            .filtered_posting_buckets(&a, "AUD", &query, &period, count, as_of)
            .await
            .expect("filtered_posting_buckets");

        assert_eq!(buckets.len(), 2);
        for bucket in &buckets {
            assert_eq!(bucket.inflow.value(), dec!(0));
            assert_eq!(bucket.outflow.value(), dec!(0));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_span_overrides_query_dates(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let svc = Service::new(pool.clone());
        // Weekly buckets ending in the week of 2025-02-12 span
        // [2025-01-27, 2025-02-17); the query's own bounds are strictly narrower.
        svc.create(tx_on(&a, &b, date(2025, 1, 27), "before-from", dec!(100)))
            .await
            .expect("leading");
        svc.create(tx_on(&a, &b, date(2025, 2, 5), "in-range", dec!(20)))
            .await
            .expect("middle");
        svc.create(tx_on(&a, &b, date(2025, 2, 13), "after-until", dec!(7)))
            .await
            .expect("trailing");

        let query = TransactionQuery {
            date_from: Some(date(2025, 1, 29)),
            date_until: Some(date(2025, 2, 11)),
            ..Default::default()
        };
        let count = core::num::NonZeroUsize::new(3).expect("3 > 0");
        let buckets = svc
            .filtered_posting_buckets(&a, "AUD", &query, &Period::Weekly, count, date(2025, 2, 12))
            .await
            .expect("filtered_posting_buckets");

        assert_eq!(buckets.len(), 3);
        let values: Vec<_> = buckets.iter().map(|bucket| bucket.inflow.value()).collect();
        // Both overshoot legs are summed: the bucket span, not the query's own
        // date bounds, is the single date authority.
        assert_eq!(values, vec![dec!(100), dec!(20), dec!(7)]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_sum_own_legs_not_matched_legs(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");
        let c = accts
            .create()
            .name("C")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("C");

        let svc = Service::new(pool.clone());
        // Deliberately asymmetric: A's own leg (+100) shares no magnitude with
        // B's matched leg (-30), so summing the matched legs instead of A's own
        // legs yields a visibly different answer.
        let split = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 1, 10))
            .payee("split".to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b.clone())
                    .amount(Amount::new(dec!(-30), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(c)
                    .amount(Amount::new(dec!(-70), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(split).await.expect("split");

        // `accounts: [B]` is posting-scoped: only B's leg is a matched posting.
        let query = TransactionQuery {
            accounts: vec![b],
            ..Default::default()
        };
        let count = core::num::NonZeroUsize::new(1).expect("1 > 0");
        let buckets = svc
            .filtered_posting_buckets(
                &a,
                "AUD",
                &query,
                &Period::Monthly,
                count,
                date(2025, 1, 20),
            )
            .await
            .expect("filtered_posting_buckets");

        assert_eq!(buckets.len(), 1);
        let jan = buckets.first().expect("one bucket");
        assert_eq!(jan.inflow.value(), dec!(100));
        assert_eq!(jan.outflow.value(), dec!(0));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_buckets_ignore_other_commodities(pool: sqlx::SqlitePool) {
        let accts = crate::account::Service::new(pool.clone());
        let a = accts
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("A");
        let b = accts
            .create()
            .name("B")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("B");

        let svc = Service::new(pool.clone());
        // A holds one AUD leg and one USD leg on the same date; the USD leg must
        // never reach an AUD bucket.
        let multi = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2025, 1, 10))
            .payee("multi".to_owned())
            .description("desc")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(a.clone())
                    .amount(Amount::new(dec!(50), CommodityCode::new("USD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(b)
                    .amount(Amount::new(dec!(-50), CommodityCode::new("USD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(Timestamp::now())
            .build();
        svc.create(multi).await.expect("multi");

        let count = core::num::NonZeroUsize::new(1).expect("1 > 0");
        let buckets = svc
            .filtered_posting_buckets(
                &a,
                "AUD",
                &TransactionQuery::default(),
                &Period::Monthly,
                count,
                date(2025, 1, 20),
            )
            .await
            .expect("filtered_posting_buckets");

        assert_eq!(buckets.len(), 1);
        let jan = buckets.first().expect("one bucket");
        assert_eq!(jan.inflow.value(), dec!(100));
        assert_eq!(jan.outflow.value(), dec!(0));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolve_account_subtrees_unions_inclusive_subtrees(pool: sqlx::SqlitePool) {
        use crate::account::Service as AccountService;
        let accounts = AccountService::new(pool.clone());
        let food = accounts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let dining = accounts
            .create()
            .name("Dining")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .parent_id(&food)
            .call()
            .await
            .expect("dining");

        let set = super::resolve_account_subtrees(&pool, core::slice::from_ref(&food))
            .await
            .expect("resolve")
            .expect("some");
        assert!(set.contains(&food));
        assert!(set.contains(&dining));

        let empty = super::resolve_account_subtrees(&pool, &[])
            .await
            .expect("resolve");
        assert_eq!(empty, None);
    }
}

#[cfg(test)]
#[cfg(feature = "ipc")]
mod tests {
    use bc_models::AccountId;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::TransactionQuery;

    #[test]
    fn try_from_filter_parses_ids_and_scalars() {
        let acc = AccountId::new();
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec![acc.to_string()];
        filter.text = Some("coffee".to_owned());
        let mut amount_filter = bc_ipc::AmountFilter::default();
        amount_filter.min = Some(Decimal::new(5, 0));
        amount_filter.commodity = Some("AUD".to_owned());
        filter.amount = Some(amount_filter);

        let query = TransactionQuery::try_from(filter).expect("valid filter");
        assert_eq!(query.accounts, vec![acc]);
        assert_eq!(query.text.as_deref(), Some("coffee"));
        let amount = query.amount.expect("amount present");
        assert_eq!(amount.min, Some(Decimal::new(5, 0)));
        assert_eq!(
            amount.commodity.map(|c| c.as_str().to_owned()),
            Some("AUD".to_owned())
        );
    }

    #[test]
    fn try_from_filter_rejects_bad_account_id() {
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec!["not-a-valid-id!!".to_owned()];
        let err = TransactionQuery::try_from(filter).expect_err("invalid account id must fail");
        assert!(matches!(err, crate::BcError::BadData(_)));
    }
}
