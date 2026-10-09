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

use bc_models::AccountId;
use bc_models::TagId;
use jiff::civil::Date;

use crate::BcResult;

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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use sqlx::Row as _;

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
}
