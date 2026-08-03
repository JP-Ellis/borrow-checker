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
#[expect(
    dead_code,
    reason = "consumed by the per-account benchmark added in Task 6"
)]
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
#[expect(
    dead_code,
    reason = "consumed by the per-account benchmark added in Task 6"
)]
async fn dominant_account(pool: &SqlitePool) -> AccountId {
    let (id,): (String,) = sqlx::query_as(
        "SELECT account_id FROM postings GROUP BY account_id ORDER BY COUNT(*) DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("find dominant account");
    id.parse::<AccountId>().expect("parse account id")
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

criterion_group!(benches, bench_default_balances);
criterion_main!(benches);
