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

/// Escapes SQL `LIKE` metacharacters (`\`, `%`, `_`) in `input` so it can be
/// embedded in a `LIKE ... ESCAPE '\'` pattern and matched literally.
///
/// The backslash is escaped first so that the escapes subsequently inserted
/// for `%` and `_` are not themselves re-escaped.
fn escape_like(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
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
        let account_set: Option<HashSet<AccountId>> = if query.accounts.is_empty() {
            None
        } else {
            let mut set = HashSet::new();
            for root in &query.accounts {
                let rows: Vec<(String,)> = sqlx::query_as(
                    "WITH RECURSIVE subtree(id) AS ( \
                         VALUES(?) \
                         UNION ALL \
                         SELECT a.id FROM accounts a JOIN subtree s ON a.parent_id = s.id \
                     ) SELECT id FROM subtree",
                )
                .bind(root.to_string())
                .fetch_all(self.pool())
                .await?;
                for (id,) in rows {
                    if let Ok(parsed) = id.parse::<AccountId>() {
                        set.insert(parsed);
                    }
                }
            }
            Some(set)
        };

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
             FROM transactions t {where_sql} ORDER BY t.date DESC"
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
            let needle = format!("%{}%", escape_like(&text.to_lowercase()));
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
    use crate::transaction::Service;

    /// Builds a two-leg AUD transaction on the given date with payee text.
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "negating a test fixture's Decimal magnitude to build the offsetting leg"
    )]
    fn tx_on(
        acc_a: &bc_models::AccountId,
        acc_b: &bc_models::AccountId,
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
