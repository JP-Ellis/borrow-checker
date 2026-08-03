//! Pure planning for generated ledgers.
//!
//! Turns generator knobs into a deterministic list of transactions to write.
//! Holds no database or `bc-core` dependency, so the fidelity knobs — skew and
//! elided ratio — are testable without I/O.

use rust_decimal::Decimal;

use crate::generate::Config;
use crate::rng::Rng;

/// Commodity used by the overwhelming majority of generated transactions.
pub const BASE_COMMODITY: &str = "AUD";

/// Secondary commodity, present so multi-commodity resolution does real work.
pub const SECOND_COMMODITY: &str = "USD";

/// Largest generated amount, in minor units (i.e. up to 500.00).
const MAX_MINOR_UNITS: u64 = 50_000;

/// Day-of-month spread. Every generated month uses 28 days so no month is short.
const DAYS_PER_MONTH: u64 = 28;

/// Number of decimal places on a generated amount.
const AMOUNT_SCALE: u32 = 2;

/// One planned transaction: a category leg and a deposit leg.
#[expect(
    clippy::module_name_repetitions,
    reason = "TxPlan is a required interface name (crate::generate::plan::TxPlan); \
               the module holds only this one type"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxPlan {
    /// Canonical transaction date.
    pub date: jiff::civil::Date,
    /// Index into the generated deposit accounts. Index 0 is the dominant one.
    pub deposit: usize,
    /// Index into the generated category accounts.
    pub category: usize,
    /// Positive magnitude; the category leg debits it and the deposit leg credits it.
    pub amount: Decimal,
    /// Whether the deposit leg's amount is elided.
    pub elided: bool,
    /// Commodity code for both legs.
    pub commodity: &'static str,
}

/// Builds the full transaction plan for `config`.
///
/// Emits `months * tx_per_month` transactions in nondecreasing date order, each
/// with exactly two legs. The deposit leg is the elided one when elided at all,
/// mirroring the bank-leg-elided import idiom that #354 identifies as the common
/// case — which puts the elided legs on the dominant account, the worst case for
/// `Residuals::for_account`.
///
/// `elided` is decided before `commodity`: a transaction only becomes a
/// second-commodity one if it was already decided not to be elided, keeping
/// the residual single-commodity by construction. This gates
/// `config.second_commodity_ratio` on the non-elided subset, so the
/// unconditional share of second-commodity transactions in the plan is
/// `(1 - config.elided_ratio) * config.second_commodity_ratio`, not
/// `config.second_commodity_ratio` itself.
///
/// # Arguments
///
/// * `config` - Generator knobs.
///
/// # Returns
///
/// The planned transactions, deterministic for a fixed `config.seed`.
#[must_use]
pub fn build(config: &Config) -> Vec<TxPlan> {
    let mut rng = Rng::new(config.seed);
    let capacity = usize::try_from(config.months)
        .unwrap_or(usize::MAX)
        .saturating_mul(usize::try_from(config.tx_per_month).unwrap_or(usize::MAX));
    let mut plans = Vec::with_capacity(capacity);

    for month in 0..config.months {
        let month_start = Config::EPOCH.saturating_add(jiff::Span::new().months(i64::from(month)));
        let mut month_plans = Vec::with_capacity(usize::try_from(config.tx_per_month).unwrap_or(0));

        for _ in 0..config.tx_per_month {
            let day_offset = rng.below(DAYS_PER_MONTH);
            let date = month_start
                .saturating_add(jiff::Span::new().days(i64::try_from(day_offset).unwrap_or(0)));

            // Index 0 is the dominant account; the rest share what is left.
            let deposit = if rng.chance(config.skew) || config.deposit_accounts <= 1 {
                0
            } else {
                let spread =
                    u64::try_from(config.deposit_accounts.saturating_sub(1)).unwrap_or(u64::MAX);
                1_usize.saturating_add(usize::try_from(rng.below(spread)).unwrap_or(usize::MAX))
            };

            let category_bound = u64::try_from(config.category_accounts).unwrap_or(u64::MAX);
            let category = usize::try_from(rng.below(category_bound)).unwrap_or(0);

            let minor = rng.below(MAX_MINOR_UNITS).saturating_add(1);
            let amount = Decimal::new(i64::try_from(minor).unwrap_or(1), AMOUNT_SCALE);

            // Elided is decided first; a second-commodity leg is only ever
            // drawn for a non-elided transaction, so the residual stays
            // single-commodity by construction rather than by a second check.
            let elided = rng.chance(config.elided_ratio);
            let secondary = !elided && rng.chance(config.second_commodity_ratio);
            let commodity = if secondary {
                SECOND_COMMODITY
            } else {
                BASE_COMMODITY
            };

            month_plans.push(TxPlan {
                date,
                deposit,
                category,
                amount,
                elided,
                commodity,
            });
        }

        // Days within a month are drawn out of order; sort so the plan as a
        // whole is emitted in nondecreasing date order.
        month_plans.sort_by_key(|plan| plan.date);
        plans.extend(month_plans);
    }

    plans
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rstest::rstest;

    use super::*;

    /// A small but statistically meaningful config for assertions.
    fn config() -> Config {
        Config {
            deposit_accounts: 20,
            category_accounts: 80,
            months: 24,
            tx_per_month: 200,
            elided_ratio: 0.80_f64,
            skew: 0.30_f64,
            second_commodity_ratio: 0.01_f64,
            seed: 42,
        }
    }

    /// Fraction of `count` out of `total`, for distribution assertions.
    #[expect(
        clippy::cast_precision_loss,
        reason = "fixture-scale counts (thousands, not quadrillions) are far below the point where usize-to-f64 loses precision"
    )]
    #[expect(
        clippy::as_conversions,
        reason = "usize-to-f64 has no fallible-free conversion; precision loss is bounded at fixture scale"
    )]
    #[expect(
        clippy::float_arithmetic,
        reason = "computing a distribution share for test assertions requires a floating-point division"
    )]
    fn share(count: usize, total: usize) -> f64 {
        count as f64 / total as f64
    }

    #[test]
    fn produces_months_times_rate_transactions() {
        let plans = build(&config());
        assert_eq!(plans.len(), 24 * 200);
    }

    #[test]
    fn is_deterministic_for_a_fixed_seed() {
        let left = build(&config());
        let right = build(&config());
        assert_eq!(left, right);
    }

    #[test]
    fn a_different_seed_produces_a_different_ledger() {
        let left = build(&config());
        let mut other = config();
        other.seed = 43;
        assert_ne!(left, build(&other));
    }

    #[test]
    fn dates_fall_inside_the_requested_span() {
        let plans = build(&config());
        let last = Config::EPOCH.saturating_add(jiff::Span::new().months(24_i32));
        for plan in &plans {
            assert!(plan.date >= Config::EPOCH, "{} precedes epoch", plan.date);
            assert!(plan.date < last, "{} runs past the span", plan.date);
        }
    }

    #[test]
    fn dates_are_nondecreasing() {
        let plans = build(&config());
        for pair in plans.windows(2) {
            let [earlier, later] = pair else {
                continue;
            };
            assert!(
                earlier.date <= later.date,
                "months must be emitted in order: {} then {}",
                earlier.date,
                later.date
            );
        }
    }

    #[test]
    fn elided_share_approximates_the_ratio() {
        let plans = build(&config());
        let elided = plans.iter().filter(|p| p.elided).count();
        let share = share(elided, plans.len());
        assert!(
            (0.77_f64..0.83_f64).contains(&share),
            "expected ~0.80 elided, got {share}"
        );
    }

    #[test]
    fn dominant_account_receives_the_skewed_share() {
        let plans = build(&config());
        let dominant = plans.iter().filter(|p| p.deposit == 0).count();
        let share = share(dominant, plans.len());
        assert!(
            (0.27_f64..0.33_f64).contains(&share),
            "expected ~0.30 on the dominant account, got {share}"
        );
    }

    #[test]
    fn dominant_account_outweighs_every_other_deposit() {
        let plans = build(&config());
        let dominant = plans.iter().filter(|p| p.deposit == 0).count();
        for index in 1..20 {
            let other = plans.iter().filter(|p| p.deposit == index).count();
            assert!(
                dominant > other * 3,
                "deposit 0 ({dominant}) should dominate deposit {index} ({other})"
            );
        }
    }

    #[rstest]
    #[case(0.0_f64)]
    #[case(1.0_f64)]
    fn extreme_elided_ratios_are_honoured(#[case] ratio: f64) {
        let mut cfg = config();
        cfg.elided_ratio = ratio;
        let plans = build(&cfg);
        let elided = plans.iter().filter(|p| p.elided).count();
        let expected = if ratio == 0.0_f64 { 0 } else { plans.len() };
        assert_eq!(elided, expected);
    }

    #[test]
    fn account_indices_stay_within_bounds() {
        let plans = build(&config());
        for plan in &plans {
            assert!(
                plan.deposit < 20,
                "deposit index {} out of range",
                plan.deposit
            );
            assert!(
                plan.category < 80,
                "category index {} out of range",
                plan.category
            );
        }
    }

    #[test]
    fn amounts_are_positive_with_two_decimal_places() {
        let plans = build(&config());
        for plan in &plans {
            assert!(plan.amount > rust_decimal::Decimal::ZERO);
            assert_eq!(plan.amount.scale(), 2);
        }
    }

    #[test]
    fn a_second_commodity_appears_but_stays_rare() {
        let cfg = config();
        let plans = build(&cfg);
        let secondary = plans
            .iter()
            .filter(|p| p.commodity != BASE_COMMODITY)
            .count();
        assert!(secondary > 0, "second commodity must appear at all");

        // A second-commodity leg is only ever drawn for a non-elided
        // transaction (see `build`'s doc comment), so the unconditional
        // share is diluted by the non-elided fraction, not
        // `second_commodity_ratio` alone.
        let share = share(secondary, plans.len());
        let expected = (1.0_f64 - cfg.elided_ratio) * cfg.second_commodity_ratio;
        let tolerance = expected * 0.6_f64;
        assert!(
            (expected - tolerance..expected + tolerance).contains(&share),
            "expected ~{expected} second-commodity share ((1 - elided_ratio) * \
             second_commodity_ratio), got {share} (tolerance {tolerance})"
        );
    }

    #[test]
    fn second_commodity_transactions_are_never_elided() {
        let plans = build(&config());
        for plan in plans.iter().filter(|p| p.commodity != BASE_COMMODITY) {
            assert!(
                !plan.elided,
                "cross-commodity legs are concrete so the residual stays single-commodity"
            );
        }
    }
}
