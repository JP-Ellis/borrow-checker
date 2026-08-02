//! Multi-commodity balance accumulator.

use core::ops::AddAssign;
use core::ops::SubAssign;

use rust_decimal::Decimal;

use crate::Amount;
use crate::AmountError;
use crate::CommodityCode;

/// A running total of [`Amount`]s across possibly differing commodities.
///
/// Adding amounts of different commodities keeps them in separate buckets — a
/// commodity mismatch is never an error here (unlike [`Amount::add`]). A
/// commodity whose running total reaches exactly zero is dropped, so
/// [`Balances::len`] / [`Balances::is_empty`] reflect only non-zero holdings.
/// Iteration order is the order commodities were first seen.
///
/// # Example
///
/// ```
/// use bc_models::{Amount, Balances};
/// use rust_decimal_macros::dec;
///
/// let mut b = Balances::new();
/// b += &Amount::new(dec!(10), "AUD");
/// b += &Amount::new(dec!(50), "USD");
/// b -= &Amount::new(dec!(10), "USD");
/// assert_eq!(b.get("AUD"), Some(dec!(10)));
/// assert_eq!(b.get("USD"), Some(dec!(40)));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Balances {
    /// Per-commodity running totals, in first-seen order. Never holds a zero.
    entries: Vec<(CommodityCode, Decimal)>,
}

impl Balances {
    /// Creates an empty [`Balances`].
    #[must_use]
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Adds `amount` into the running total for its commodity.
    ///
    /// # Arguments
    ///
    /// * `amount` - The amount to add.
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::Overflow`] if the commodity's running total would
    /// exceed [`Decimal`]'s range.
    #[inline]
    pub fn try_add(&mut self, amount: &Amount) -> Result<(), AmountError> {
        self.accumulate(amount.commodity(), amount.value())
    }

    /// Subtracts `amount` from the running total for its commodity.
    ///
    /// # Arguments
    ///
    /// * `amount` - The amount to subtract.
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::Overflow`] if the commodity's running total would
    /// exceed [`Decimal`]'s range.
    #[inline]
    pub fn try_sub(&mut self, amount: &Amount) -> Result<(), AmountError> {
        let negated = Decimal::ZERO
            .checked_sub(amount.value())
            .ok_or(AmountError::Overflow)?;
        self.accumulate(amount.commodity(), negated)
    }

    /// Returns the running total for `commodity`, or `None` if it holds nothing.
    ///
    /// # Arguments
    ///
    /// * `commodity` - The commodity code to look up.
    ///
    /// # Returns
    ///
    /// The non-zero running total, or `None`.
    #[must_use]
    #[inline]
    pub fn get(&self, commodity: &str) -> Option<Decimal> {
        self.entries
            .iter()
            .find(|(code, _)| code.as_str() == commodity)
            .map(|(_, value)| *value)
    }

    /// Iterates over `(commodity_code, value)` pairs in first-seen order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, Decimal)> {
        self.entries
            .iter()
            .map(|(code, value)| (code.as_str(), *value))
    }

    /// Returns the number of distinct non-zero commodities held.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no commodity holds a non-zero amount.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds `delta` to `commodity`'s total, dropping the bucket if it reaches zero.
    fn accumulate(&mut self, commodity: &CommodityCode, delta: Decimal) -> Result<(), AmountError> {
        let mut found = false;
        for entry in &mut self.entries {
            if entry.0 == *commodity {
                entry.1 = entry.1.checked_add(delta).ok_or(AmountError::Overflow)?;
                found = true;
                break;
            }
        }
        if found {
            self.entries.retain(|(_, value)| !value.is_zero());
        } else if !delta.is_zero() {
            self.entries.push((commodity.clone(), delta));
        }
        Ok(())
    }
}

impl AddAssign<&Amount> for Balances {
    /// Adds `amount` into the running total for its commodity.
    ///
    /// # Panics
    ///
    /// Panics if the commodity's running total overflows [`Decimal`]'s range.
    /// Use [`Balances::try_add`] to handle overflow without panicking.
    #[inline]
    fn add_assign(&mut self, rhs: &Amount) {
        match self.try_add(rhs) {
            Ok(()) => {}
            #[expect(
                clippy::panic,
                reason = "panic on overflow is documented in trait contract"
            )]
            Err(error) => panic!("Balances += {error}"),
        }
    }
}

impl SubAssign<&Amount> for Balances {
    /// Subtracts `amount` from the running total for its commodity.
    ///
    /// # Panics
    ///
    /// Panics if the commodity's running total overflows [`Decimal`]'s range.
    /// Use [`Balances::try_sub`] to handle overflow without panicking.
    #[inline]
    fn sub_assign(&mut self, rhs: &Amount) {
        match self.try_sub(rhs) {
            Ok(()) => {}
            #[expect(
                clippy::panic,
                reason = "panic on overflow is documented in trait contract"
            )]
            Err(error) => panic!("Balances -= {error}"),
        }
    }
}

impl Extend<Amount> for Balances {
    #[inline]
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Amount>,
    {
        for amount in iter {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "using += trait is the intended usage pattern"
            )]
            {
                *self += &amount;
            }
        }
    }
}

impl FromIterator<Amount> for Balances {
    #[inline]
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = Amount>,
    {
        let mut balances = Self::new();
        balances.extend(iter);
        balances
    }
}

impl IntoIterator for Balances {
    type Item = Amount;
    type IntoIter = std::vec::IntoIter<Amount>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.entries
            .into_iter()
            .map(|(code, value)| Amount::new(value, code))
            .collect::<Vec<_>>()
            .into_iter()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::Balances;
    use crate::Amount;

    #[test]
    fn nets_per_commodity() {
        let mut b = Balances::new();
        b += &Amount::new(dec!(10), "AUD");
        b += &Amount::new(dec!(50), "USD");
        b -= &Amount::new(dec!(10), "USD");
        assert_eq!(b.get("AUD"), Some(dec!(10)));
        assert_eq!(b.get("USD"), Some(dec!(40)));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn drops_commodity_on_zero() {
        let mut b = Balances::new();
        b += &Amount::new(dec!(10), "USD");
        b -= &Amount::new(dec!(10), "USD");
        assert!(b.is_empty());
        assert_eq!(b.get("USD"), None);
    }

    #[test]
    fn collect_matches_fold() {
        let amounts = [
            Amount::new(dec!(10), "AUD"),
            Amount::new(dec!(50), "USD"),
            Amount::new(dec!(5), "AUD"),
        ];
        let collected: Balances = amounts.iter().cloned().collect();
        let folded = amounts.iter().fold(Balances::new(), |mut acc, a| {
            acc += a;
            acc
        });
        assert_eq!(collected.get("AUD"), Some(dec!(15)));
        assert_eq!(collected.get("AUD"), folded.get("AUD"));
        assert_eq!(collected.get("USD"), folded.get("USD"));
    }

    #[test]
    fn iter_is_insertion_ordered() {
        let mut b = Balances::new();
        b += &Amount::new(dec!(1), "ZZZ");
        b += &Amount::new(dec!(1), "AAA");
        let codes: Vec<&str> = b.iter().map(|(c, _)| c).collect();
        assert_eq!(codes, vec!["ZZZ", "AAA"]);
    }

    #[test]
    fn try_add_overflow_errors() {
        let mut b = Balances::new();
        b += &Amount::new(rust_decimal::Decimal::MAX, "AUD");
        assert!(b.try_add(&Amount::new(dec!(1), "AUD")).is_err());
    }

    #[test]
    #[should_panic(expected = "Balances")]
    fn add_assign_overflow_panics() {
        let mut b = Balances::new();
        b += &Amount::new(rust_decimal::Decimal::MAX, "AUD");
        b += &Amount::new(dec!(1), "AUD");
    }
}
