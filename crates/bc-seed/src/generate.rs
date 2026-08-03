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
    /// Share of transactions denominated in the secondary commodity.
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
