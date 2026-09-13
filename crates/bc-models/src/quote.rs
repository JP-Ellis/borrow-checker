//! A stated per-unit or total figure attached to a posting.

use rust_decimal::Decimal;

use crate::money::Amount;
use crate::money::AmountError;
use crate::money::CommodityCode;

/// A figure stated against a posting's amount, either per unit or in total.
///
/// Beancount writes a per-unit price as `@ 332 AUD` and a total as
/// `@@ 6.37 AUD`; a cost basis uses the same pair as `{105 AUD}` and
/// `{{210 AUD}}`. Both forms are kept as written, because neither converts to
/// the other exactly in general: `3 USD @@ 10 AUD` has no finite per-unit
/// price, while a per-unit figure multiplies out exactly.
///
/// Re-exported from the crate root as [`crate::Quote`].
///
/// # Example
///
/// ```
/// use bc_models::{Amount, Quote};
/// use rust_decimal_macros::dec;
///
/// let price = Quote::PerUnit(Amount::new(dec!(332), "AUD"));
/// let weight = price.weigh(dec!(-2)).expect("weighs");
/// assert_eq!(weight.value(), dec!(-664));
/// ```
#[expect(
    clippy::exhaustive_enums,
    reason = "the two forms are the whole vocabulary; bc-core and bc-cli match on both and must feel a third"
)]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quote {
    /// One unit of the posting's amount is worth this much: `@ 332 AUD`, `{105 AUD}`.
    PerUnit(Amount),
    /// The whole posting is worth this much: `@@ 6.37 AUD`, `{{210 AUD}}`.
    /// Stated without sign; the weight takes the sign of the posting's amount.
    Total(Amount),
}

impl Quote {
    /// Returns the stated amount, per unit or in total.
    #[inline]
    #[must_use]
    pub fn amount(&self) -> &Amount {
        match *self {
            Self::PerUnit(ref amount) | Self::Total(ref amount) => amount,
        }
    }

    /// Returns the commodity the figure is stated in.
    #[inline]
    #[must_use]
    pub fn commodity(&self) -> &CommodityCode {
        self.amount().commodity()
    }

    /// Returns `true` for the total form.
    #[inline]
    #[must_use]
    pub fn is_total(&self) -> bool {
        matches!(*self, Self::Total(_))
    }

    /// Weighs `units` of the posting's amount in this quote's commodity.
    ///
    /// # Arguments
    ///
    /// * `units` - The posting's amount value.
    ///
    /// # Returns
    ///
    /// `units × per_unit` for [`Self::PerUnit`]; the total carrying the sign
    /// of `units` for [`Self::Total`], zero when `units` is zero.
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::Overflow`] if the product exceeds
    /// [`Decimal`]'s range.
    pub fn weigh(&self, units: Decimal) -> Result<Amount, AmountError> {
        let value = match *self {
            Self::PerUnit(ref per_unit) => units
                .checked_mul(per_unit.value())
                .ok_or(AmountError::Overflow)?,
            Self::Total(ref total) => {
                if units.is_zero() {
                    Decimal::ZERO
                } else if units.is_sign_negative() {
                    Decimal::ZERO
                        .checked_sub(total.value())
                        .ok_or(AmountError::Overflow)?
                } else {
                    total.value()
                }
            }
        };
        Ok(Amount::new(value, self.commodity().clone()))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    fn aud(value: Decimal) -> Amount {
        Amount::new(value, CommodityCode::new("AUD"))
    }

    #[rstest]
    #[case::per_unit_positive(Quote::PerUnit(aud(dec!(332))), dec!(2), dec!(664))]
    #[case::per_unit_negative(Quote::PerUnit(aud(dec!(332))), dec!(-2.5), dec!(-830.0))]
    #[case::per_unit_zero(Quote::PerUnit(aud(dec!(332))), dec!(0), dec!(0))]
    #[case::total_positive(Quote::Total(aud(dec!(6.37))), dec!(4.00), dec!(6.37))]
    #[case::total_negative(Quote::Total(aud(dec!(6.37))), dec!(-4.00), dec!(-6.37))]
    #[case::total_zero(Quote::Total(aud(dec!(6.37))), dec!(0), dec!(0))]
    fn weigh_follows_the_units_sign(
        #[case] quote: Quote,
        #[case] units: Decimal,
        #[case] expected: Decimal,
    ) {
        let weight = quote.weigh(units).expect("weighs");
        assert_eq!(weight.value(), expected);
        assert_eq!(weight.commodity().as_str(), "AUD");
    }

    #[test]
    fn weigh_per_unit_overflow_is_an_error() {
        let quote = Quote::PerUnit(aud(Decimal::MAX));
        assert_eq!(quote.weigh(dec!(2)), Err(AmountError::Overflow));
    }

    #[test]
    fn accessors_expose_the_stated_amount() {
        let total = Quote::Total(aud(dec!(10)));
        assert!(total.is_total());
        assert_eq!(total.amount(), &aud(dec!(10)));
        assert_eq!(total.commodity().as_str(), "AUD");
        assert!(!Quote::PerUnit(aud(dec!(1))).is_total());
    }

    #[test]
    fn serde_round_trips_both_forms() {
        for quote in [Quote::PerUnit(aud(dec!(1.5))), Quote::Total(aud(dec!(3)))] {
            let json = serde_json::to_string(&quote).expect("serialise");
            let back: Quote = serde_json::from_str(&json).expect("deserialise");
            assert_eq!(back, quote);
        }
    }
}
