// Pure display-metadata resolution for monetary formatting. No Leptos or WASM
// here, so it is host-tested natively.

use core::cmp::Ordering;

use bc_ipc::CommodityInfo;
use rust_decimal::Decimal;
use rust_decimal::RoundingStrategy;

/// Formatting metadata resolved for a single currency code.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::module_name_repetitions,
    reason = "DisplayMeta is the canonical type name; the crate-level expect for this lint is wasm32-gated and does not reach the native `components_tests` shim used to test this pure module"
)]
pub struct DisplayMeta {
    /// Canonical code (e.g. `"AUD"`).
    pub code: String,
    /// Display symbol; falls back to the code when none is recorded.
    pub symbol: String,
    /// Minor-unit digits.
    pub decimals: u8,
    /// Whether this is an ISO 4217 currency.
    pub is_iso: bool,
    /// Whether the symbol follows the amount.
    pub symbol_after: bool,
}

/// Resolves formatting metadata for `code` against the served currency set.
///
/// Falls back to sane defaults (2 decimals; ISO when `code` is three ASCII
/// uppercase letters) when the code is absent or the set has not loaded yet,
/// so amounts never render blank.
///
/// # Arguments
///
/// * `code` - The currency code to resolve.
/// * `currencies` - The served commodity set.
///
/// # Returns
///
/// The resolved [`DisplayMeta`].
#[must_use]
pub fn display_meta_for(code: &str, currencies: &[CommodityInfo]) -> DisplayMeta {
    if let Some(c) = currencies
        .iter()
        .find(|c| c.code.eq_ignore_ascii_case(code))
    {
        return DisplayMeta {
            code: c.code.clone(),
            symbol: c.symbol.clone().unwrap_or_else(|| c.code.clone()),
            decimals: c.decimals,
            is_iso: c.is_iso,
            symbol_after: c.symbol_after,
        };
    }
    let looks_iso = code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase());
    DisplayMeta {
        code: code.to_owned(),
        symbol: code.to_owned(),
        decimals: 2,
        is_iso: looks_iso,
        symbol_after: false,
    }
}

/// The sign `value` shows once rounded to `decimals` places.
///
/// Rounds half away from zero, as `Intl.NumberFormat` does, so a value
/// that displays as zero carries no sign.
#[must_use]
pub fn display_sign(value: &Decimal, decimals: u8) -> Ordering {
    value
        .round_dp_with_strategy(u32::from(decimals), RoundingStrategy::MidpointAwayFromZero)
        .cmp(&Decimal::ZERO)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    fn aud() -> CommodityInfo {
        CommodityInfo::new(
            "commodity_1",
            "AUD",
            Some("A$".to_owned()),
            vec![],
            2,
            true,
            false,
        )
    }

    #[test]
    fn resolves_known_code_case_insensitively() {
        let set = vec![aud()];
        let m = display_meta_for("aud", &set);
        assert_eq!(m.symbol, "A$");
        assert_eq!(m.decimals, 2);
        assert!(m.is_iso);
    }

    #[test]
    fn unknown_iso_like_code_falls_back() {
        let m = display_meta_for("USD", &[]);
        assert_eq!(m.symbol, "USD");
        assert_eq!(m.decimals, 2);
        assert!(m.is_iso);
    }

    #[test]
    fn unknown_non_iso_code_is_not_iso() {
        let m = display_meta_for("DOGE", &[]);
        assert!(!m.is_iso);
    }

    #[rstest]
    #[case::rounds_to_zero_below(dec!(-0.004), 2, Ordering::Equal)]
    #[case::rounds_to_zero_above(dec!(0.004), 2, Ordering::Equal)]
    #[case::midpoint_rounds_away(dec!(-0.005), 2, Ordering::Less)]
    #[case::whole_units(dec!(0.4), 0, Ordering::Equal)]
    #[case::positive(dec!(1.25), 2, Ordering::Greater)]
    #[case::negative(dec!(-1.25), 2, Ordering::Less)]
    #[case::zero(dec!(0), 2, Ordering::Equal)]
    fn sign_follows_the_rounded_value(
        #[case] value: Decimal,
        #[case] decimals: u8,
        #[case] expected: Ordering,
    ) {
        assert_eq!(display_sign(&value, decimals), expected);
    }
}
