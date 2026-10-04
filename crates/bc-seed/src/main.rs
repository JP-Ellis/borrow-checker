//! Seed a SQLite database for E2E tests or benchmarks.
//!
//! Two modes: the hand-written e2e fixture (the default) and a synthetic
//! ledger of arbitrary size (`generate`).
//!
//! Usage:
//!   `bc-seed [--db-path <PATH>] [--force] [generate ...]`.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]
#![expect(
    clippy::print_stdout,
    clippy::arithmetic_side_effects,
    clippy::too_many_lines,
    reason = "test fixture seeding binary; not a public library"
)]

mod fixture;
mod generate;
mod rng;

use std::path::PathBuf;

use clap::Parser;

#[derive(Parser)]
#[command(
    name = "bc-seed",
    about = "Seed a SQLite database with realistic test fixture data"
)]
/// CLI arguments for the seed binary.
struct Args {
    /// Path where the database file will be written.
    #[arg(long, default_value = "./borrow-checker-test.db")]
    db_path: PathBuf,

    /// Overwrite the database file if it already exists.
    #[arg(long)]
    force: bool,

    /// What to seed. Omit for the hand-authored E2E fixture.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Seeding modes.
#[derive(clap::Subcommand)]
enum Command {
    /// Generate a synthetic ledger of arbitrary size for benchmarking.
    Generate(GenerateOpts),
}

/// Knobs for the synthetic generator.
///
/// Defaults are rounded rather than exact: derived statistics from a real
/// ledger are themselves identifying.
#[derive(clap::Args)]
struct GenerateOpts {
    /// Number of deposit accounts under `Assets`.
    #[arg(long, default_value_t = 20)]
    deposit_accounts: usize,

    /// Number of category accounts under `Expenses`.
    #[arg(long, default_value_t = 80)]
    category_accounts: usize,

    /// Calendar months to span, starting at the fixed epoch.
    #[arg(long, default_value_t = 24)]
    months: u32,

    /// Transactions generated per month.
    #[arg(long, default_value_t = 200)]
    tx_per_month: u32,

    /// Share of transactions whose deposit leg is elided.
    #[arg(long, default_value_t = 0.80)]
    elided_ratio: f64,

    /// Share of transactions touching the single dominant deposit account.
    #[arg(long, default_value_t = 0.30)]
    skew: f64,

    /// Share of *non-elided* transactions denominated in the secondary
    /// commodity. The unconditional share across the whole ledger is
    /// `(1 - elided_ratio) * second_commodity_ratio`.
    #[arg(long, default_value_t = 0.01)]
    second_commodity_ratio: f64,

    /// PRNG seed. Fixing this fixes the entire ledger.
    #[arg(long, default_value_t = 42)]
    seed: u64,
}

impl GenerateOpts {
    /// Converts parsed CLI options into a generator [`generate::Config`].
    ///
    /// # Returns
    ///
    /// The equivalent configuration.
    fn to_config(&self) -> generate::Config {
        generate::Config {
            deposit_accounts: self.deposit_accounts,
            category_accounts: self.category_accounts,
            months: self.months,
            tx_per_month: self.tx_per_month,
            elided_ratio: self.elided_ratio,
            skew: self.skew,
            second_commodity_ratio: self.second_commodity_ratio,
            seed: self.seed,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.db_path.exists() && !args.force {
        anyhow::bail!(
            "database already exists at '{}'. Use --force to overwrite.",
            args.db_path.display()
        );
    }

    if args.db_path.exists() && args.force {
        std::fs::remove_file(&args.db_path)?;
    }

    let pool = bc_core::open_db_at(&args.db_path).await?;

    if let Some(Command::Generate(ref opts)) = args.command {
        let config = opts.to_config();
        println!(
            "Generating {} months x {} transactions/month…",
            config.months, config.tx_per_month
        );
        generate::run(&pool, &config).await?;
        // Checkpoint so the on-disk `.db` file holds the full ledger
        // deterministically; otherwise how much data sits in the `-wal`
        // sidecar depends on async-runtime shutdown timing at process exit.
        // `busy != 0` means another connection blocked the checkpoint and
        // frames were left behind — the pragma itself still reports success
        // in that case, so the row must be inspected rather than discarded.
        let (busy, log, checkpointed): (i64, i64, i64) =
            sqlx::query_as("PRAGMA wal_checkpoint(TRUNCATE);")
                .fetch_one(&pool)
                .await?;
        if busy != 0 {
            anyhow::bail!(
                "WAL checkpoint for '{}' was blocked by another connection \
                 (busy={busy}, log={log}, checkpointed={checkpointed}); \
                 the fixture may have uncheckpointed data left in its -wal file",
                args.db_path.display()
            );
        }
        pool.close().await;
        println!("Generated ledger written to {}", args.db_path.display());
        return Ok(());
    }

    fixture::seed(&pool).await?;
    pool.close().await;

    println!("Done.");
    println!("Created database at {}", args.db_path.display());
    println!("Accounts:     29 (5 root + 24 below them)");
    println!("Budgets:       7 (one per expense leaf account)");
    println!("Tags:         13 (9 roots + 4 children; recurring/business/shared/…)");
    println!("Revisions:     9 (7 initial + 2 mid-year bumps for groceries and electricity)");
    println!(
        "Transactions: 278 (cleared, pending, voided across 6 historical months + \
         current month, including 150 in the paging-fixture Archive account)"
    );

    Ok(())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn bare_invocation_still_seeds_the_fixture() {
        // e2e/wdio.conf.ts's onPrepare and e2e/mise.toml's test:web invoke exactly this form.
        // It must not break.
        let args = Args::parse_from(["bc-seed", "--db-path", "/tmp/x.db", "--force"]);
        assert!(args.force);
        assert!(
            args.command.is_none(),
            "no subcommand means the E2E fixture"
        );
    }

    #[test]
    fn generate_subcommand_parses_with_defaults() {
        let args = Args::parse_from(["bc-seed", "generate"]);
        let Some(Command::Generate(opts)) = args.command else {
            panic!("expected the generate subcommand");
        };
        assert_eq!(opts.deposit_accounts, 20);
        assert_eq!(opts.category_accounts, 80);
        assert_eq!(opts.months, 24);
        assert_eq!(opts.tx_per_month, 200);
        assert_eq!(opts.seed, 42);
    }

    #[test]
    fn generate_subcommand_accepts_overrides() {
        let args = Args::parse_from([
            "bc-seed",
            "generate",
            "--months",
            "120",
            "--tx-per-month",
            "2000",
        ]);
        let Some(Command::Generate(opts)) = args.command else {
            panic!("expected the generate subcommand");
        };
        assert_eq!(opts.months, 120);
        assert_eq!(opts.tx_per_month, 2000);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "values pass through clap parsing and struct construction \
                   unmodified, so bit-exact equality is the correct check"
    )]
    fn generate_options_convert_to_a_config() {
        let args = Args::parse_from(["bc-seed", "generate", "--skew", "0.5"]);
        let Some(Command::Generate(opts)) = args.command else {
            panic!("expected the generate subcommand");
        };
        let config = opts.to_config();
        assert_eq!(config.skew, 0.5_f64);
        assert_eq!(config.elided_ratio, 0.80_f64);
    }
}
