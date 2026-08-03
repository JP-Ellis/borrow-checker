//! Benchmarks for the balance read path.
//!
//! Measures the public `Engine` calls named in #362 and #370 against generated
//! ledgers at three sizes. Fixtures are produced separately by
//! `mise run bench:fixtures`; this suite only reads them.

#![expect(
    clippy::expect_used,
    clippy::print_stdout,
    reason = "benchmark harness, not library code"
)]

use std::path::PathBuf;

use bc_core::BalanceEngine;
use bc_models::AccountId;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use sqlx::SqlitePool;

/// Commodity the generator denominates the overwhelming majority of legs in.
const COMMODITY: &str = "AUD";

/// The tiers defined in the design spec, smallest first.
const TIERS: &[&str] = &["t0", "t1", "t2"];

/// Returns the directory holding cached benchmark fixtures.
fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/bench-fixtures")
}

/// Opens the cached fixture for `tier`.
///
/// Returns `None` when the fixture has not been generated, so a partial run is
/// still useful rather than aborting the whole suite.
async fn open_tier(tier: &str) -> Option<SqlitePool> {
    let path = fixture_dir().join(format!("{tier}.db"));
    if !path.exists() {
        println!(
            "skipping tier {tier}: {} not found — run `mise run bench:fixtures`",
            path.display()
        );
        return None;
    }
    Some(
        bc_core::open_db_at(&path)
            .await
            .expect("open benchmark fixture"),
    )
}

/// Returns the account holding the most postings — the dominant account the
/// generator's `--skew` knob produces, and the worst case for the residual query.
async fn dominant_account(pool: &SqlitePool) -> AccountId {
    let (id,): (String,) = sqlx::query_as(
        "SELECT account_id FROM postings GROUP BY account_id ORDER BY COUNT(*) DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("find dominant account");
    id.parse::<AccountId>().expect("parse account id")
}

/// Returns the calendar month containing the fixture's latest transaction
/// date, as `[from, until)`.
///
/// Mirrors what an account page shows: the most recent period, with the opening
/// balance spanning everything before it. `from` therefore always falls on a
/// populated day; `until` is the exclusive start of the following month and
/// may fall outside the data.
async fn last_month(pool: &SqlitePool) -> (jiff::civil::Date, jiff::civil::Date) {
    let (max_date,): (String,) = sqlx::query_as("SELECT MAX(date) FROM transactions")
        .fetch_one(pool)
        .await
        .expect("latest transaction date");
    let date = max_date
        .parse::<jiff::civil::Date>()
        .expect("parse latest date");
    let from = date.first_of_month();
    let until = from.saturating_add(jiff::Span::new().months(1_i32));
    (from, until)
}

/// Benchmarks the whole-ledger set-based pass — the floor against which the
/// #244 cache must justify itself.
fn bench_default_balances(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("default_balances");
    group.sample_size(10);

    for tier in TIERS {
        let Some(pool) = runtime.block_on(open_tier(tier)) else {
            continue;
        };
        let engine = BalanceEngine::new(pool);
        group.bench_function(*tier, |b| {
            b.to_async(&runtime)
                .iter(|| async { engine.default_balances().await.expect("default_balances") });
        });
    }

    group.finish();
}

/// Benchmarks a single account's balance — the per-iteration unit cost that
/// `net_worth` pays once per account (#362).
fn bench_balance_for(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("balance_for_dominant");
    group.sample_size(10);

    for tier in TIERS {
        let Some(pool) = runtime.block_on(open_tier(tier)) else {
            continue;
        };
        let account = runtime.block_on(dominant_account(&pool));
        let engine = BalanceEngine::new(pool);
        group.bench_function(*tier, |b| {
            b.to_async(&runtime).iter(|| async {
                engine
                    .balance_for(&account, COMMODITY)
                    .await
                    .expect("balance_for")
            });
        });
    }

    group.finish();
}

/// Benchmarks the N+1 loop across every asset and liability account (#362).
fn bench_net_worth(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("net_worth");
    group.sample_size(10);

    for tier in TIERS {
        let Some(pool) = runtime.block_on(open_tier(tier)) else {
            continue;
        };
        let engine = BalanceEngine::new(pool);
        group.bench_function(*tier, |b| {
            b.to_async(&runtime)
                .iter(|| async { engine.net_worth(COMMODITY).await.expect("net_worth") });
        });
    }

    group.finish();
}

/// Benchmarks the account page's period stats — three or more unbounded
/// residual loads per call (#370).
fn bench_account_period_stats(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("account_period_stats_dominant");
    group.sample_size(10);

    for tier in TIERS {
        let Some(pool) = runtime.block_on(open_tier(tier)) else {
            continue;
        };
        let account = runtime.block_on(dominant_account(&pool));
        let (from, until) = runtime.block_on(last_month(&pool));
        let engine = BalanceEngine::new(pool);
        group.bench_function(*tier, |b| {
            b.to_async(&runtime).iter(|| async {
                engine
                    .account_period_stats(&account, COMMODITY, from, until)
                    .await
                    .expect("account_period_stats")
            });
        });
    }

    group.finish();
}

/// Benchmarks a six-month sparkline — the third residual replay on an account
/// page render (#370).
fn bench_posting_buckets(c: &mut Criterion) {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut group = c.benchmark_group("posting_buckets_dominant");
    group.sample_size(10);

    let count = core::num::NonZeroUsize::new(6).expect("six is nonzero");
    let period = bc_models::Period::Monthly;

    for tier in TIERS {
        let Some(pool) = runtime.block_on(open_tier(tier)) else {
            continue;
        };
        let account = runtime.block_on(dominant_account(&pool));
        let (from, _) = runtime.block_on(last_month(&pool));
        let engine = BalanceEngine::new(pool);
        group.bench_function(*tier, |b| {
            b.to_async(&runtime).iter(|| async {
                engine
                    .posting_buckets(&account, COMMODITY, &period, count, from)
                    .await
                    .expect("posting_buckets")
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_balance_for,
    bench_net_worth,
    bench_default_balances,
    bench_account_period_stats,
    bench_posting_buckets,
);
criterion_main!(benches);
