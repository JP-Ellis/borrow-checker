//! A posting's price or cost basis at the IPC boundary.

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;

use crate::money::Amount;

/// A figure stated against a posting's amount, per unit or in total.
///
/// Mirrors `bc_models::Quote`: `@ 332 AUD` / `{105 AUD}` are per unit,
/// `@@ 6.37 AUD` / `{{210 AUD}}` are totals. Serialises as an externally
/// tagged enum (`{"per_unit": {...}}` / `{"total": {...}}`), matching the
/// model's `rename_all = "snake_case"`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Quote {
    /// One unit of the posting's amount is worth this much.
    PerUnit(Amount),
    /// The whole posting is worth this much; read without sign.
    Total(Amount),
}

impl Quote {
    /// Returns the stated amount, per unit or in total.
    #[must_use]
    #[inline]
    pub fn amount(&self) -> &Amount {
        match *self {
            Self::PerUnit(ref amount) | Self::Total(ref amount) => amount,
        }
    }

    /// Returns `true` for the total form.
    #[must_use]
    #[inline]
    pub fn is_total(&self) -> bool {
        matches!(*self, Self::Total(_))
    }

    /// Weighs `units` of the posting's amount in this quote's currency.
    ///
    /// # Arguments
    ///
    /// * `units` - The posting's amount value.
    ///
    /// # Returns
    ///
    /// `units × per_unit` for [`Self::PerUnit`]; the total's magnitude under
    /// the sign of `units` for [`Self::Total`] (zero when `units` is zero).
    /// `None` when a per-unit product overflows [`Decimal`].
    #[must_use]
    pub fn weigh(&self, units: Decimal) -> Option<Amount> {
        let value = match *self {
            Self::PerUnit(ref per_unit) => units.checked_mul(per_unit.value)?,
            Self::Total(ref total) => {
                if units.is_zero() {
                    Decimal::ZERO
                } else {
                    let mut magnitude = total.value.abs();
                    magnitude.set_sign_negative(units.is_sign_negative());
                    magnitude
                }
            }
        };
        Some(Amount::new(value, self.amount().currency_code.clone()))
    }
}

/// A posting's cost basis: the lot it was acquired into.
///
/// Mirrors `bc_models::Cost`. The basis is the only required part; the lot
/// date and label are Beancount's optional lot identifiers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Cost {
    /// `{105 AUD}` (per unit) or `{{210 AUD}}` (total).
    pub basis: Quote,
    /// Lot date, if stated.
    pub date: Option<jiff::civil::Date>,
    /// Lot label, if stated.
    pub label: Option<String>,
}

impl Cost {
    /// Creates a new [`Cost`].
    ///
    /// # Arguments
    ///
    /// * `basis` - Per-unit or total basis.
    /// * `date` - Lot date, or `None`.
    /// * `label` - Lot label, or `None`.
    #[must_use]
    #[inline]
    pub fn new(basis: Quote, date: Option<jiff::civil::Date>, label: Option<String>) -> Self {
        Self { basis, date, label }
    }
}

// MARK: models conversions

#[cfg(feature = "models")]
impl From<&bc_models::Quote> for Quote {
    /// Converts a model quote to its IPC form, value verbatim.
    #[inline]
    fn from(q: &bc_models::Quote) -> Self {
        match *q {
            bc_models::Quote::PerUnit(ref a) => Self::PerUnit(Amount::from(a)),
            bc_models::Quote::Total(ref a) => Self::Total(Amount::from(a)),
        }
    }
}

#[cfg(feature = "models")]
impl From<&Quote> for bc_models::Quote {
    /// Converts an IPC quote to the model form.
    #[inline]
    fn from(q: &Quote) -> Self {
        match *q {
            Quote::PerUnit(ref a) => Self::PerUnit(bc_models::Amount::from(a)),
            Quote::Total(ref a) => Self::Total(bc_models::Amount::from(a)),
        }
    }
}

#[cfg(feature = "models")]
impl From<&bc_models::Cost> for Cost {
    /// Converts a model cost basis to its IPC form.
    #[inline]
    fn from(c: &bc_models::Cost) -> Self {
        Self::new(
            Quote::from(c.basis()),
            c.date(),
            c.label().map(ToOwned::to_owned),
        )
    }
}

#[cfg(feature = "models")]
impl From<&Cost> for bc_models::Cost {
    /// Converts an IPC cost basis to the model form.
    #[inline]
    fn from(c: &Cost) -> Self {
        bc_models::Cost::builder()
            .basis(bc_models::Quote::from(&c.basis))
            .maybe_date(c.date)
            .maybe_label(c.label.clone())
            .build()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::Cost;
    use super::Quote;
    use crate::money::Amount;

    fn aud(cents: i64) -> Amount {
        Amount::new(Decimal::new(cents, 2), "AUD")
    }

    #[rstest]
    #[case::per_unit_positive(Quote::PerUnit(aud(30_000)), Decimal::TWO, 60_000)]
    #[case::per_unit_negative(Quote::PerUnit(aud(30_000)), -Decimal::TWO, -60_000)]
    #[case::total_positive(Quote::Total(aud(637)), Decimal::new(400, 2), 637)]
    #[case::total_negative(Quote::Total(aud(637)), Decimal::new(-400, 2), -637)]
    #[case::total_zero_units(Quote::Total(aud(637)), Decimal::ZERO, 0)]
    fn weigh_follows_the_units_sign(
        #[case] quote: Quote,
        #[case] units: Decimal,
        #[case] expected_cents: i64,
    ) {
        assert_eq!(quote.weigh(units), Some(aud(expected_cents)));
    }

    #[test]
    fn weigh_overflow_is_none() {
        let quote = Quote::PerUnit(Amount::new(Decimal::MAX, "AUD"));
        assert_eq!(quote.weigh(Decimal::TWO), None);
    }

    #[test]
    fn quote_serde_round_trip() {
        let quote = Quote::Total(aud(637));
        let json = serde_json::to_string(&quote).expect("serialises");
        assert_eq!(json, r#"{"total":{"value":"6.37","currency_code":"AUD"}}"#);
        let back: Quote = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, quote);
    }

    #[test]
    fn cost_serde_round_trip() {
        let cost = Cost::new(
            Quote::PerUnit(aud(10_500)),
            Some(jiff::civil::Date::constant(2024, 3, 1)),
            Some("lot-a".to_owned()),
        );
        let json = serde_json::to_string(&cost).expect("serialises");
        let back: Cost = serde_json::from_str(&json).expect("deserialises");
        assert_eq!(back, cost);
    }
}

#[cfg(test)]
#[cfg(feature = "models")]
#[cfg_attr(coverage_nightly, coverage(off))]
mod models_tests {
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::Cost;
    use super::Quote;
    use crate::money::Amount;

    #[test]
    fn quote_round_trips_through_the_model() {
        let ipc = Quote::Total(Amount::new(Decimal::new(637, 2), "AUD"));
        let model = bc_models::Quote::from(&ipc);
        assert_eq!(
            model,
            bc_models::Quote::Total(bc_models::Amount::new(Decimal::new(637, 2), "AUD"))
        );
        assert_eq!(Quote::from(&model), ipc);
    }

    #[test]
    fn cost_round_trips_through_the_model() {
        let ipc = Cost::new(
            Quote::PerUnit(Amount::new(Decimal::new(10_500, 2), "AUD")),
            Some(jiff::civil::Date::constant(2024, 3, 1)),
            Some("lot-a".to_owned()),
        );
        let model = bc_models::Cost::from(&ipc);
        assert_eq!(model.label(), Some("lot-a"));
        assert_eq!(model.date(), Some(jiff::civil::Date::constant(2024, 3, 1)));
        assert_eq!(Cost::from(&model), ipc);
    }
}
