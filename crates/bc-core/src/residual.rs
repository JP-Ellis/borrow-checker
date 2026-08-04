//! Residual resolution for elided postings.
//!
//! A posting whose amount the source document elides absorbs its transaction's
//! residual — the negation of its sibling legs' sum, per commodity. Nothing is
//! persisted: the residual is derived on every read, so it stays correct when a
//! sibling leg changes (`docs/DESIGN.md` §4.4).

use std::collections::BTreeMap;
use std::collections::HashMap;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::AmountError;
use bc_models::Balances;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// The residual a transaction's elided leg absorbs.
#[expect(
    clippy::exhaustive_enums,
    reason = "Task 2 and Task 5 match on all three variants; a new variant is a deliberate breaking change they should feel"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Residual {
    /// No leg is elided, so there is nothing to derive.
    NotElided,
    /// Exactly one leg is elided and absorbs this per-commodity residual.
    ///
    /// Empty when the concrete legs already sum to zero, or when there are no
    /// concrete legs at all.
    Attributable(Balances),
    /// Two or more legs are elided. The residual is real but cannot be
    /// attributed to any single leg, so it contributes to no balance.
    Ambiguous,
}

/// Computes a transaction's residual from its legs' amounts.
///
/// # Arguments
///
/// * `amounts` - One entry per leg: `Some` for a concrete amount, `None` for an
///   elided leg. Order is irrelevant.
///
/// # Returns
///
/// [`Residual::Attributable`] carrying the negated per-commodity sum of the
/// concrete legs when exactly one leg is elided, [`Residual::Ambiguous`] when
/// two or more are, and [`Residual::NotElided`] when none is.
///
/// # Errors
///
/// Returns [`AmountError::Overflow`] if a per-commodity total would exceed
/// [`rust_decimal::Decimal`]'s range.
///
/// # Example
///
/// ```rust
/// use bc_core::residual::Residual;
/// use bc_core::residual::residual_of;
/// use bc_models::Amount;
/// use rust_decimal_macros::dec;
///
/// let food = Amount::new(dec!(50), "AUD");
/// let residual = residual_of([Some(&food), None]).expect("residual");
/// let Residual::Attributable(balances) = residual else {
///     panic!("expected an attributable residual");
/// };
/// assert_eq!(balances.get("AUD"), Some(dec!(-50)));
/// ```
#[inline]
#[expect(
    clippy::module_name_repetitions,
    reason = "residual_of is the module's sole public function; the brief mandates this exact name for Task 2 and Task 5"
)]
pub fn residual_of<'a, I>(amounts: I) -> Result<Residual, AmountError>
where
    I: IntoIterator<Item = Option<&'a Amount>>,
{
    let mut balances = Balances::new();
    let mut elided = 0_usize;
    for amount in amounts {
        match amount {
            // Subtracting accumulates the negation, which is the residual.
            Some(a) => balances.try_sub(a)?,
            None => elided = elided.saturating_add(1),
        }
    }
    match elided {
        0 => Ok(Residual::NotElided),
        1 => Ok(Residual::Attributable(balances)),
        _ => Ok(Residual::Ambiguous),
    }
}

/// One row of the set-based residual query.
type ResidualRow = (String, String, String, Option<String>, Option<String>);

/// Residuals of elided postings, resolved in one pass.
///
/// Built by a single query per engine call — never one query per transaction,
/// which would be N+1 on every balance read.
#[derive(Debug, Clone, Default)]
pub(crate) struct Residuals {
    /// Per-commodity residual balances, keyed by elided posting id, one entry
    /// per elided posting whose transaction was attributable.
    entries: HashMap<String, Balances>,
    /// Entries' balances, pre-aggregated by account id.
    by_account: HashMap<String, Balances>,
}

/// Builds the residual query, restricting the elided legs with `elided_predicate`.
///
/// `elided_predicate` is appended to the *inner* subquery, which selects which
/// transactions to resolve. It must never constrain `sib`: the outer query has to
/// return **every** leg of a selected transaction, because [`residual_of`] needs the
/// full leg set to detect the ambiguous two-or-more-elided case. Filtering `sib`
/// would silently reclassify an ambiguous transaction as attributable and inject a
/// residual that does not exist.
///
/// # Arguments
///
/// * `elided_predicate` - SQL fragment beginning with `AND`, or empty for no
///   restriction. Interpolated, so it must be a compile-time constant, never user
///   input.
///
/// # Returns
///
/// The complete query text.
fn residual_sql(elided_predicate: &str) -> String {
    format!(
        "SELECT sib.transaction_id, sib.id, sib.account_id, sib.amount, sib.commodity
         FROM postings sib
         WHERE sib.transaction_id IN (
             SELECT e.transaction_id
             FROM postings e
             WHERE e.amount IS NULL
               {elided_predicate}
         )
         ORDER BY sib.transaction_id, sib.id"
    )
}

impl Residuals {
    /// Loads residuals for every elided posting belonging to `account_id`.
    ///
    /// # Arguments
    ///
    /// * `pool` - Connection pool.
    /// * `account_id` - The account whose elided postings to resolve.
    ///
    /// # Returns
    ///
    /// The residuals, empty if the account holds no elided postings.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if
    /// a stored amount cannot be parsed or a total overflows.
    pub(crate) async fn for_account(pool: &SqlitePool, account_id: &AccountId) -> BcResult<Self> {
        Self::load(pool, Some(account_id.to_string())).await
    }

    /// Loads residuals for every elided posting in the database.
    ///
    /// # Arguments
    ///
    /// * `pool` - Connection pool.
    ///
    /// # Returns
    ///
    /// The residuals, empty if no elided postings exist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if
    /// a stored amount cannot be parsed or a total overflows.
    pub(crate) async fn for_all_accounts(pool: &SqlitePool) -> BcResult<Self> {
        Self::load(pool, None).await
    }

    /// Loads residuals, optionally scoped to one account.
    ///
    /// Fetches every leg of every transaction owning an in-scope elided posting,
    /// because [`residual_of`] needs the full leg set to detect the ambiguous
    /// two-or-more-elided case.
    async fn load(pool: &SqlitePool, account_id: Option<String>) -> BcResult<Self> {
        // Two query strings rather than `(?1 IS NULL OR e.account_id = ?1)`: the
        // disjunction is not sargable, so it full-scans `postings` on every call.
        // The all-accounts arm still scans, correctly — it wants every elided leg.
        let sql = match account_id {
            Some(_) => residual_sql("AND e.account_id = ?1"),
            None => residual_sql(""),
        };
        let mut query = sqlx::query_as(sqlx::AssertSqlSafe(sql));
        if let Some(id) = account_id {
            query = query.bind(id);
        }
        let rows: Vec<ResidualRow> = query.fetch_all(pool).await?;

        // Group legs by transaction, preserving each leg's identity. A `BTreeMap`
        // keeps iteration order deterministic rather than hash-order dependent.
        // The query's `ORDER BY` does the same within a transaction: leg order
        // sets the first-seen commodity order of the residual `Balances`, which
        // drives both the multi-commodity display order and the commodity
        // inferred by `BalanceEngine::residual_commodities`.
        let mut by_transaction: BTreeMap<String, Vec<(String, String, Option<Amount>)>> =
            BTreeMap::new();
        for (transaction_id, posting_id, acct_id, amount, commodity) in rows {
            let parsed = match (amount, commodity) {
                (Some(value), Some(code)) => {
                    let decimal = value.parse::<Decimal>().map_err(|e| {
                        BcError::BadData(format!("invalid posting amount '{value}': {e}"))
                    })?;
                    Some(Amount::new(decimal, code))
                }
                _ => None,
            };
            by_transaction
                .entry(transaction_id)
                .or_default()
                .push((posting_id, acct_id, parsed));
        }

        let mut entries: HashMap<String, Balances> = HashMap::new();
        let mut by_account: HashMap<String, Balances> = HashMap::new();
        for legs in by_transaction.values() {
            let residual = residual_of(legs.iter().map(|(_, _, amount)| amount.as_ref()))
                .map_err(|e| BcError::BadData(format!("residual overflow: {e}")))?;
            let Residual::Attributable(balances) = residual else {
                continue;
            };
            for (posting_id, acct_id, amount) in legs {
                if amount.is_none() {
                    entries.insert(posting_id.clone(), balances.clone());
                    let totals = by_account.entry(acct_id.clone()).or_default();
                    for (code, value) in balances.iter() {
                        totals.try_add(&Amount::new(value, code)).map_err(|e| {
                            BcError::BadData(format!("residual overflow for '{code}': {e}"))
                        })?;
                    }
                }
            }
        }

        Ok(Self {
            entries,
            by_account,
        })
    }

    /// Sums every held residual's `commodity` component.
    ///
    /// Intended for an account-scoped load, where every entry belongs to the
    /// account being queried.
    ///
    /// # Arguments
    ///
    /// * `commodity` - The commodity code to total.
    ///
    /// # Returns
    ///
    /// The summed residual, zero when nothing is held in `commodity`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the running total overflows.
    pub(crate) fn total_in(&self, commodity: &str) -> BcResult<Decimal> {
        self.entries
            .values()
            .try_fold(Decimal::ZERO, |acc, balances| {
                let component = balances.get(commodity).unwrap_or(Decimal::ZERO);
                acc.checked_add(component).ok_or_else(|| {
                    BcError::BadData("residual overflow: sum exceeds Decimal range".into())
                })
            })
    }

    /// Groups every held residual by account id.
    ///
    /// # Returns
    ///
    /// A map from account id string to that account's total residual across all
    /// commodities.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a per-commodity total overflows.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "Task 3/4 call sites expect BcResult per the interface contract, even though aggregation already happened in `load`"
    )]
    pub(crate) fn totals_by_account(&self) -> BcResult<HashMap<String, Balances>> {
        Ok(self.by_account.clone())
    }

    /// Returns the residual component of `posting_id` in `commodity`.
    ///
    /// # Arguments
    ///
    /// * `posting_id` - Id of the elided posting.
    /// * `commodity` - The commodity code to look up.
    ///
    /// # Returns
    ///
    /// The component, or `None` when the posting holds no residual in that
    /// commodity (including when its transaction was ambiguous).
    pub(crate) fn component(&self, posting_id: &str, commodity: &str) -> Option<Decimal> {
        let balances = self.entries.get(posting_id)?;
        balances.get(commodity)
    }

    /// Iterates every account holding a residual, with its aggregated balances.
    ///
    /// # Returns
    ///
    /// One `(account id, balances)` pair per account that holds at least one
    /// elided posting whose transaction was attributable.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no production call site yet iterates residuals by account without also needing the totals map; kept for the balance engine's future per-account residual views"
        )
    )]
    pub(crate) fn accounts_with_residuals(&self) -> impl Iterator<Item = (&str, &Balances)> {
        self.by_account
            .iter()
            .map(|(id, balances)| (id.as_str(), balances))
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use sqlx::Row as _;

    use super::*;

    /// Returns the `detail` column of every `EXPLAIN QUERY PLAN` row for `sql`.
    ///
    /// Parameters are left unbound; SQLite treats them as NULL, which is what the
    /// planner sees for a prepared statement. Only `detail` is read, so the number of
    /// columns SQLite returns is irrelevant.
    async fn query_plan(pool: &sqlx::SqlitePool, sql: &str) -> Vec<String> {
        sqlx::query(sqlx::AssertSqlSafe(format!("EXPLAIN QUERY PLAN {sql}")))
            .fetch_all(pool)
            .await
            .expect("explain query plan")
            .iter()
            .map(|row| row.get::<String, _>("detail"))
            .collect()
    }

    /// E1: the account-scoped residual load must not scan `postings`.
    ///
    /// A non-sargable account predicate still returns correct rows, so nothing but the
    /// query plan catches this regression.
    #[sqlx::test(migrations = "./migrations")]
    async fn account_scoped_residual_load_uses_an_index(pool: sqlx::SqlitePool) {
        let plan = query_plan(&pool, &residual_sql("AND e.account_id = ?1")).await;

        let joined = plan.join("\n");
        assert!(
            !joined.contains("SCAN e"),
            "account-scoped residual load full-scans postings:\n{joined}"
        );
        assert!(
            joined.contains("SEARCH e USING INDEX idx_postings_account"),
            "account-scoped residual load does not use idx_postings_account:\n{joined}"
        );
    }

    /// C5: the two query strings must agree.
    ///
    /// Splitting the disjunction duplicated the SQL. This guards the copies from
    /// drifting apart.
    #[sqlx::test(migrations = "./migrations")]
    async fn all_accounts_load_equals_the_fold_of_per_account_loads(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let card = make_account(&pool, "Card", AccountType::Liability).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, acct, amount) in [("1", &bank, "50.00"), ("2", &card, "25.00")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, "2026-01-01").await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_e_{n}"),
                &tx,
                &acct.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let all = Residuals::for_all_accounts(&pool)
            .await
            .expect("load all")
            .totals_by_account()
            .expect("totals");

        let mut folded: HashMap<String, Balances> = HashMap::new();
        for acct in [&bank, &card, &food] {
            let per = Residuals::for_account(&pool, acct)
                .await
                .expect("load one")
                .totals_by_account()
                .expect("totals");
            folded.extend(per);
        }

        assert_eq!(all, folded);
    }

    /// Builds a concrete AUD amount.
    fn aud(value: rust_decimal::Decimal) -> Amount {
        Amount::new(value, "AUD")
    }

    /// Builds a concrete USD amount.
    fn usd(value: rust_decimal::Decimal) -> Amount {
        Amount::new(value, "USD")
    }

    #[test]
    fn single_elided_leg_absorbs_the_negated_sum() {
        let food = aud(dec!(50));
        let residual = residual_of([Some(&food), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert_eq!(balances.get("AUD"), Some(dec!(-50)));
        assert_eq!(balances.len(), 1);
    }

    #[test]
    fn two_elided_legs_are_ambiguous() {
        let food = aud(dec!(50));
        let residual = residual_of([Some(&food), None, None]).expect("residual");
        assert_eq!(residual, Residual::Ambiguous);
    }

    #[test]
    fn no_elided_leg_yields_not_elided() {
        let debit = aud(dec!(50));
        let credit = aud(dec!(-50));
        let residual = residual_of([Some(&debit), Some(&credit)]).expect("residual");
        assert_eq!(residual, Residual::NotElided);
    }

    #[test]
    fn concrete_legs_summing_to_zero_leave_an_empty_residual() {
        let debit = aud(dec!(50));
        let credit = aud(dec!(-50));
        let residual = residual_of([Some(&debit), Some(&credit), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert!(balances.is_empty());
    }

    #[test]
    fn lone_elided_leg_has_an_empty_residual() {
        let residual = residual_of([None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert!(balances.is_empty());
    }

    #[test]
    fn residual_spans_every_commodity_the_siblings_use() {
        let a = aud(dec!(50));
        let u = usd(dec!(30));
        let residual = residual_of([Some(&a), Some(&u), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert_eq!(balances.get("AUD"), Some(dec!(-50)));
        assert_eq!(balances.get("USD"), Some(dec!(-30)));
        assert_eq!(balances.len(), 2);
    }

    #[rstest]
    #[case(dec!(50), dec!(-50))]
    #[case(dec!(-50), dec!(50))]
    #[case(dec!(0.01), dec!(-0.01))]
    fn residual_negates_the_sibling(
        #[case] sibling: rust_decimal::Decimal,
        #[case] expected: rust_decimal::Decimal,
    ) {
        let amount = aud(sibling);
        let residual = residual_of([Some(&amount), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert_eq!(balances.get("AUD"), Some(expected));
    }

    /// Inserts a transaction with the given id and date.
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

    /// Inserts a posting; `amount`/`commodity` are `None` for an elided leg.
    async fn insert_posting(
        pool: &sqlx::SqlitePool,
        id: &str,
        tx_id: &str,
        account_id: &str,
        amount: Option<&str>,
        commodity: Option<&str>,
        position: i64,
    ) {
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(tx_id)
        .bind(account_id)
        .bind(amount)
        .bind(commodity)
        .bind(position)
        .execute(pool)
        .await
        .expect("insert posting");
    }

    /// Creates an account and returns its id.
    async fn make_account(
        pool: &sqlx::SqlitePool,
        name: &str,
        account_type: AccountType,
    ) -> bc_models::AccountId {
        crate::account::Service::new(pool.clone())
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loader_attributes_the_residual_to_the_elided_leg(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_1",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(&pool, "p_bank", "tx_1", &bank.to_string(), None, None, 1).await;

        let residuals = Residuals::for_account(&pool, &bank).await.expect("load");

        assert_eq!(residuals.component("p_bank", "AUD"), Some(dec!(-50.00)));
        assert_eq!(residuals.total_in("AUD").expect("total"), dec!(-50.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loader_ignores_transactions_with_two_elided_legs(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        let fun = make_account(&pool, "Fun", AccountType::Expense).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_1",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(&pool, "p_bank", "tx_1", &bank.to_string(), None, None, 1).await;
        insert_posting(&pool, "p_fun", "tx_1", &fun.to_string(), None, None, 2).await;

        let residuals = Residuals::for_account(&pool, &bank).await.expect("load");

        assert_eq!(residuals.component("p_bank", "AUD"), None);
        assert_eq!(residuals.total_in("AUD").expect("total"), Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loader_scopes_to_the_requested_account(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let other = make_account(&pool, "Other", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_1",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(&pool, "p_bank", "tx_1", &bank.to_string(), None, None, 1).await;

        let residuals = Residuals::for_account(&pool, &other).await.expect("load");

        assert_eq!(residuals.total_in("AUD").expect("total"), Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loader_sums_every_commodity_across_all_accounts(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        let travel = make_account(&pool, "Travel", AccountType::Expense).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_1",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_travel",
            "tx_1",
            &travel.to_string(),
            Some("30.00"),
            Some("USD"),
            1,
        )
        .await;
        insert_posting(&pool, "p_bank", "tx_1", &bank.to_string(), None, None, 2).await;

        let residuals = Residuals::for_all_accounts(&pool).await.expect("load");
        let by_account = residuals.totals_by_account().expect("totals");
        let bank_totals = by_account.get(&bank.to_string()).expect("bank residual");

        assert_eq!(bank_totals.get("AUD"), Some(dec!(-50.00)));
        assert_eq!(bank_totals.get("USD"), Some(dec!(-30.00)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loader_accumulates_across_transactions(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, amount) in [("1", "50.00"), ("2", "25.00")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, "2026-01-01").await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let residuals = Residuals::for_account(&pool, &bank).await.expect("load");

        assert_eq!(residuals.total_in("AUD").expect("total"), dec!(-75.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loader_iterates_every_account_holding_a_residual(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_1", "2026-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_1",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(&pool, "p_bank", "tx_1", &bank.to_string(), None, None, 1).await;

        let residuals = Residuals::for_all_accounts(&pool).await.expect("load");
        let accounts: HashMap<&str, &Balances> = residuals.accounts_with_residuals().collect();

        assert_eq!(
            accounts
                .get(bank.to_string().as_str())
                .and_then(|b| b.get("AUD")),
            Some(dec!(-50.00))
        );
    }
}
