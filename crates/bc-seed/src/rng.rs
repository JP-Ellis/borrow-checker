//! Deterministic pseudo-random number generation for fixture generation.
//!
//! Written inline rather than using `rand`, because `rand` does not guarantee
//! value-stability across releases and benchmark fixtures must be reproducible
//! byte-for-byte across toolchain updates.

#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "consumed only by generate::plan::build, which is not yet \
                   wired into main.rs until a later task"
    )
)]

/// Multiplier for the xorshift64\* output scrambler.
const SCRAMBLE: u64 = 0x2545_F491_4F6C_DD1D;

/// Substitute state for a zero seed, which xorshift cannot escape.
const NONZERO_FALLBACK: u64 = 0x9E37_79B9_7F4A_7C15;

/// Number of mantissa bits in an `f64`, used to map a `u64` into `[0, 1)`.
const F64_MANTISSA_BITS: u32 = 53;

/// A deterministic xorshift64\* pseudo-random number generator.
///
/// Reproducible across platforms and toolchain versions, which `rand` does not
/// promise. Not cryptographically secure and not intended to be.
#[derive(Debug, Clone)]
pub struct Rng {
    /// Current generator state. Never zero.
    state: u64,
}

impl Rng {
    /// Creates a generator from `seed`.
    ///
    /// # Arguments
    ///
    /// * `seed` - Starting state. Zero is substituted, since xorshift cannot
    ///   escape a zero state.
    ///
    /// # Returns
    ///
    /// A generator positioned at the start of `seed`'s sequence.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { NONZERO_FALLBACK } else { seed },
        }
    }

    /// Advances the generator and returns the next value.
    ///
    /// # Returns
    ///
    /// A pseudo-random `u64` drawn uniformly from the full range.
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x >> 12_u32;
        x ^= x << 25_u32;
        x ^= x >> 27_u32;
        self.state = x;
        x.wrapping_mul(SCRAMBLE)
    }

    /// Returns a value in `[0, n)`.
    ///
    /// Uses modulo reduction, whose bias is immaterial at fixture scale.
    ///
    /// # Arguments
    ///
    /// * `n` - Exclusive upper bound.
    ///
    /// # Returns
    ///
    /// A value below `n`, or zero when `n` is zero.
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            return 0;
        }
        self.next_u64().wrapping_rem(n)
    }

    /// Returns `true` with probability `p`.
    ///
    /// # Arguments
    ///
    /// * `p` - Probability in `[0.0, 1.0]`. Values outside the range saturate.
    ///
    /// # Returns
    ///
    /// `true` with probability `p`, `false` otherwise.
    #[expect(
        clippy::cast_precision_loss,
        reason = "mapping a 53-bit mantissa slice into f64 is exact by construction"
    )]
    #[expect(
        clippy::as_conversions,
        reason = "u64-to-f64 has no safe fallible-free conversion; precision loss is bounded by F64_MANTISSA_BITS"
    )]
    #[expect(
        clippy::float_arithmetic,
        reason = "mapping a random u64 into [0, 1) requires a floating-point division"
    )]
    pub fn chance(&mut self, p: f64) -> bool {
        if p <= 0.0_f64 {
            return false;
        }
        if p >= 1.0_f64 {
            return true;
        }
        let scale = (1_u64 << F64_MANTISSA_BITS) as f64;
        let unit = (self.next_u64() >> (u64::BITS - F64_MANTISSA_BITS)) as f64 / scale;
        unit < p
    }
}

#[cfg(test)]
mod tests {
    use std::iter::repeat_with;

    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;

    use super::*;

    #[test]
    fn same_seed_yields_same_sequence() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        let left: Vec<u64> = repeat_with(|| a.next_u64()).take(32).collect();
        let right: Vec<u64> = repeat_with(|| b.next_u64()).take(32).collect();
        assert_eq!(left, right);
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(43);
        assert_ne!(a.next_u64(), b.next_u64());
    }

    #[test]
    fn zero_seed_does_not_stick_at_zero() {
        let mut rng = Rng::new(0);
        let values: Vec<u64> = repeat_with(|| rng.next_u64()).take(8).collect();
        assert!(
            values.iter().any(|&v| v != 0),
            "xorshift with a zero state degenerates; seed must be substituted"
        );
    }

    #[test]
    fn below_stays_in_range() {
        let mut rng = Rng::new(7);
        for _ in 0_u32..1_000_u32 {
            assert!(rng.below(10) < 10);
        }
    }

    #[test]
    fn below_zero_returns_zero() {
        let mut rng = Rng::new(7);
        assert_eq!(rng.below(0), 0);
    }

    #[test]
    fn chance_zero_never_fires_and_one_always_does() {
        let mut rng = Rng::new(7);
        for _ in 0_u32..100_u32 {
            assert!(!rng.chance(0.0_f64));
            assert!(rng.chance(1.0_f64));
        }
    }

    #[test]
    fn chance_approximates_requested_probability() {
        let mut rng = Rng::new(7);
        let hits = repeat_with(|| rng.chance(0.30_f64))
            .take(10_000_usize)
            .filter(|&hit| hit)
            .count();
        assert!(
            (2700..3300).contains(&hits),
            "expected ~3000 hits at p=0.30, got {hits}"
        );
    }
}
