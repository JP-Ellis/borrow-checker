//! Synthetic ledger generation for benchmarking.
//!
//! Produces deterministic ledgers of arbitrary size, written through real
//! `bc-core` services so the write path is exercised. Distinct from the
//! hand-authored E2E fixture in `main.rs`, which must not change.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "not yet wired into main.rs; a later task turns the plan into \
                   real accounts and transactions via bc-core services"
    )
)]

pub mod plan;

/// Knobs controlling a generated ledger.
///
/// Defaults are rounded rather than exact, deliberately: derived statistics
/// from a real ledger are themselves identifying.
#[derive(Debug, Clone)]
pub struct Config {
    /// Number of deposit accounts under `Assets`.
    pub deposit_accounts: usize,
    /// Number of category accounts under `Expenses`.
    pub category_accounts: usize,
    /// Number of calendar months to span, starting at [`Config::EPOCH`].
    pub months: u32,
    /// Transactions generated per month.
    pub tx_per_month: u32,
    /// Share of transactions whose deposit leg is elided.
    pub elided_ratio: f64,
    /// Share of transactions touching the single dominant deposit account.
    pub skew: f64,
    /// Share of *non-elided* transactions denominated in the secondary
    /// commodity.
    ///
    /// A second-commodity leg is only ever drawn for a transaction that was
    /// already decided not to be elided (see [`plan::build`]), so the
    /// unconditional share across the whole plan is
    /// `(1 - elided_ratio) * second_commodity_ratio`, not
    /// `second_commodity_ratio` itself.
    pub second_commodity_ratio: f64,
    /// PRNG seed. Fixing this fixes the entire ledger.
    pub seed: u64,
}

impl Config {
    /// Fixed start date for every generated ledger.
    ///
    /// Deliberately not the wall clock: benchmark runs must be comparable
    /// across days. (The hand-authored E2E fixture is date-relative on purpose;
    /// this is the opposite requirement.)
    pub const EPOCH: jiff::civil::Date = jiff::civil::date(2000, 1, 1);
}

use bc_core::AccountService;
use bc_core::CommodityService;
use bc_core::TransactionService;
use bc_models::AccountId;
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

use crate::generate::plan::BASE_COMMODITY;
use crate::generate::plan::SECOND_COMMODITY;

/// How often to report progress while generating, in transactions.
const PROGRESS_EVERY: usize = 10_000;

/// Generates a ledger matching `config` into `pool`.
///
/// Writes through real `bc-core` services rather than raw SQL, so the write
/// path — including the event log — is exercised. That is slower, and it is
/// what makes a future invalidation-cost measurement possible at all.
///
/// # Arguments
///
/// * `pool` - An open, migrated database. Expected to be empty.
/// * `config` - Generator knobs.
///
/// # Returns
///
/// Nothing on success.
///
/// # Errors
///
/// Returns any error from commodity, account, or transaction creation.
pub async fn run(pool: &sqlx::SqlitePool, config: &Config) -> anyhow::Result<()> {
    let commodities = CommodityService::new(pool.clone());
    for (code, symbol, name, decimals) in [
        (BASE_COMMODITY, "A$", "Australian Dollar", 2_u8),
        (SECOND_COMMODITY, "$", "US Dollar", 2_u8),
    ] {
        commodities
            .create(
                &bc_models::Commodity::builder()
                    .code(code)
                    .symbol(symbol)
                    .name(name)
                    .decimals(decimals)
                    .is_iso(true)
                    .symbol_after(false)
                    .build(),
            )
            .await?;
    }

    let accounts = AccountService::new(pool.clone());

    let assets = accounts
        .create()
        .name("Assets")
        .account_type(AccountType::Asset)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;
    let expenses = accounts
        .create()
        .name("Expenses")
        .account_type(AccountType::Expense)
        .kind(AccountKind::DepositAccount)
        .call()
        .await?;

    let mut deposits: Vec<AccountId> = Vec::with_capacity(config.deposit_accounts);
    for index in 0..config.deposit_accounts {
        deposits.push(
            accounts
                .create()
                .name(&format!("Deposit-{index:03}"))
                .account_type(AccountType::Asset)
                .kind(AccountKind::DepositAccount)
                .parent_id(&assets)
                .call()
                .await?,
        );
    }

    let mut categories: Vec<AccountId> = Vec::with_capacity(config.category_accounts);
    for index in 0..config.category_accounts {
        categories.push(
            accounts
                .create()
                .name(&format!("Category-{index:03}"))
                .account_type(AccountType::Expense)
                .kind(AccountKind::DepositAccount)
                .parent_id(&expenses)
                .call()
                .await?,
        );
    }

    let transactions = TransactionService::new(pool.clone());
    let plans = plan::build(config);
    let total = plans.len();

    for (index, item) in plans.iter().enumerate() {
        let Some(deposit_id) = deposits.get(item.deposit) else {
            anyhow::bail!("deposit index {} out of range", item.deposit);
        };
        let Some(category_id) = categories.get(item.category) else {
            anyhow::bail!("category index {} out of range", item.category);
        };
        let code = CommodityCode::new(item.commodity);

        let category_leg = Posting::builder()
            .id(PostingId::new())
            .account_id(category_id.clone())
            .amount(Amount::new(item.amount, code.clone()))
            .build();

        // The deposit leg is the elided one — the bank-leg-elided import idiom.
        let deposit_leg = if item.elided {
            Posting::builder()
                .id(PostingId::new())
                .account_id(deposit_id.clone())
                .build()
        } else {
            Posting::builder()
                .id(PostingId::new())
                .account_id(deposit_id.clone())
                .amount(Amount::new(-item.amount, code))
                .build()
        };

        transactions
            .create(
                Transaction::builder()
                    .id(TransactionId::new())
                    .date(item.date)
                    .description(format!("Generated {index:07}"))
                    .reconciliation(Reconciliation::Unreconciled)
                    .created_at(Timestamp::now())
                    .postings(vec![category_leg, deposit_leg])
                    .build(),
            )
            .await?;

        if index > 0 && index.wrapping_rem(PROGRESS_EVERY) == 0 {
            println!("  generated {index}/{total} transactions");
        }
    }

    println!("  generated {total}/{total} transactions");
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    /// A deliberately tiny ledger — enough to assert shape, fast enough to run
    /// in the normal test suite.
    fn tiny() -> Config {
        Config {
            deposit_accounts: 5,
            category_accounts: 10,
            months: 2,
            tx_per_month: 25,
            elided_ratio: 0.80,
            skew: 0.30,
            second_commodity_ratio: 0.01,
            seed: 42,
        }
    }

    /// Opens a migrated database in a temporary directory.
    async fn temp_pool(dir: &tempfile::TempDir) -> sqlx::SqlitePool {
        bc_core::open_db_at(&dir.path().join("gen.db"))
            .await
            .expect("open temp db")
    }

    #[tokio::test]
    async fn writes_the_expected_transaction_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = temp_pool(&dir).await;
        run(&pool, &tiny()).await.expect("generate");

        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM transactions")
            .fetch_one(&pool)
            .await
            .expect("count transactions");
        assert_eq!(count, 50);
    }

    #[tokio::test]
    async fn writes_two_postings_per_transaction() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = temp_pool(&dir).await;
        run(&pool, &tiny()).await.expect("generate");

        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM postings")
            .fetch_one(&pool)
            .await
            .expect("count postings");
        assert_eq!(count, 100);
    }

    #[tokio::test]
    async fn elided_postings_land_on_the_dominant_account() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = temp_pool(&dir).await;
        run(&pool, &tiny()).await.expect("generate");

        // The account holding the most postings must also hold the most
        // elided ones — this is the property the benchmark depends on.
        let (busiest,): (String,) = sqlx::query_as(
            "SELECT account_id FROM postings GROUP BY account_id ORDER BY COUNT(*) DESC LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("busiest account");

        let (elided_here,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM postings WHERE amount IS NULL AND account_id = ?")
                .bind(&busiest)
                .fetch_one(&pool)
                .await
                .expect("count elided");
        assert!(elided_here > 0, "dominant account must carry elided legs");
    }

    #[tokio::test]
    async fn creates_the_requested_account_tree() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = temp_pool(&dir).await;
        run(&pool, &tiny()).await.expect("generate");

        // 5 deposit + 10 category + 2 roots (Assets, Expenses).
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .expect("count accounts");
        assert_eq!(count, 17);
    }

    #[tokio::test]
    async fn every_transaction_has_exactly_one_or_zero_elided_legs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let pool = temp_pool(&dir).await;
        run(&pool, &tiny()).await.expect("generate");

        let (worst,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(n), 0) FROM (
                 SELECT COUNT(*) AS n FROM postings
                 WHERE amount IS NULL GROUP BY transaction_id
             )",
        )
        .fetch_one(&pool)
        .await
        .expect("max elided per transaction");
        assert!(
            worst <= 1,
            "two elided legs make a transaction ambiguous and contribute no residual"
        );
    }

    #[tokio::test]
    async fn generation_is_reproducible() {
        let first_dir = tempfile::tempdir().expect("tempdir");
        let first = temp_pool(&first_dir).await;
        run(&first, &tiny()).await.expect("generate");

        let second_dir = tempfile::tempdir().expect("tempdir");
        let second = temp_pool(&second_dir).await;
        run(&second, &tiny()).await.expect("generate");

        // IDs are random per run, so compare the shape rather than the rows.
        let shape = "SELECT t.date, p.amount FROM postings p \
                     JOIN transactions t ON t.id = p.transaction_id \
                     ORDER BY t.date, p.amount";
        let left: Vec<(String, Option<String>)> =
            sqlx::query_as(shape).fetch_all(&first).await.expect("left");
        let right: Vec<(String, Option<String>)> = sqlx::query_as(shape)
            .fetch_all(&second)
            .await
            .expect("right");
        assert_eq!(left, right);
    }
}
