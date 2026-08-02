//! Multi-commodity balance accumulator for the IPC boundary.

use core::ops::AddAssign;
use core::ops::SubAssign;

use rust_decimal::Decimal;

use crate::Amount;
use crate::AmountError;

/// A running total of [`Amount`]s across possibly differing currency codes.
///
/// Adding amounts of different currencies keeps them in separate buckets — a
/// currency mismatch is never an error here (unlike [`Amount::add`]). A currency
/// whose running total reaches exactly zero is dropped, so [`Balances::len`] /
/// [`Balances::is_empty`] reflect only non-zero holdings. Iteration order is the
/// order currencies were first seen.
///
/// # Example
///
/// ```
/// use bc_ipc::{Amount, Balances};
/// use rust_decimal::Decimal;
///
/// let mut b = Balances::new();
/// b += &Amount::new(Decimal::new(10, 0), "AUD");
/// b += &Amount::new(Decimal::new(50, 0), "USD");
/// b -= &Amount::new(Decimal::new(10, 0), "USD");
/// assert_eq!(b.get("AUD"), Some(Decimal::new(10, 0)));
/// assert_eq!(b.get("USD"), Some(Decimal::new(40, 0)));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Balances {
    /// Per-currency running totals, in first-seen order. Never holds a zero.
    entries: Vec<(String, Decimal)>,
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

    /// Adds `amount` into the running total for its currency.
    ///
    /// # Arguments
    ///
    /// * `amount` - The amount to add.
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::Overflow`] if the currency's running total would
    /// exceed [`Decimal`]'s range.
    #[inline]
    pub fn try_add(&mut self, amount: &Amount) -> Result<(), AmountError> {
        self.accumulate(&amount.currency_code, amount.value)
    }

    /// Subtracts `amount` from the running total for its currency.
    ///
    /// # Arguments
    ///
    /// * `amount` - The amount to subtract.
    ///
    /// # Errors
    ///
    /// Returns [`AmountError::Overflow`] if the currency's running total would
    /// exceed [`Decimal`]'s range.
    #[inline]
    pub fn try_sub(&mut self, amount: &Amount) -> Result<(), AmountError> {
        let negated = Decimal::ZERO
            .checked_sub(amount.value)
            .ok_or(AmountError::Overflow)?;
        self.accumulate(&amount.currency_code, negated)
    }

    /// Returns the running total for `currency_code`, or `None` if it holds nothing.
    ///
    /// # Arguments
    ///
    /// * `currency_code` - The currency code to look up.
    ///
    /// # Returns
    ///
    /// The non-zero running total, or `None`.
    #[must_use]
    #[inline]
    pub fn get(&self, currency_code: &str) -> Option<Decimal> {
        self.entries
            .iter()
            .find(|(code, _)| code == currency_code)
            .map(|(_, value)| *value)
    }

    /// Iterates over `(currency_code, value)` pairs in first-seen order.
    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (&str, Decimal)> {
        self.entries
            .iter()
            .map(|(code, value)| (code.as_str(), *value))
    }

    /// Returns the number of distinct non-zero currencies held.
    #[must_use]
    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns `true` if no currency holds a non-zero amount.
    #[must_use]
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Adds `delta` to `currency_code`'s total, dropping the bucket if it reaches zero.
    fn accumulate(&mut self, currency_code: &str, delta: Decimal) -> Result<(), AmountError> {
        let mut found = false;
        for entry in &mut self.entries {
            if entry.0 == currency_code {
                entry.1 = entry.1.checked_add(delta).ok_or(AmountError::Overflow)?;
                found = true;
                break;
            }
        }
        if found {
            self.entries.retain(|(_, value)| !value.is_zero());
        } else if !delta.is_zero() {
            self.entries.push((currency_code.to_owned(), delta));
        }
        Ok(())
    }
}

impl AddAssign<&Amount> for Balances {
    /// Adds `amount` into the running total for its currency.
    ///
    /// # Panics
    ///
    /// Panics if the currency's running total overflows [`Decimal`]'s range. Use
    /// [`Balances::try_add`] to handle overflow without panicking.
    #[inline]
    #[expect(
        clippy::panic,
        reason = "panics are intentional for unchecked arithmetic; see docstring"
    )]
    fn add_assign(&mut self, rhs: &Amount) {
        match self.try_add(rhs) {
            Ok(()) => {}
            Err(error) => panic!("Balances += {error}"),
        }
    }
}

impl SubAssign<&Amount> for Balances {
    /// Subtracts `amount` from the running total for its currency.
    ///
    /// # Panics
    ///
    /// Panics if the currency's running total overflows [`Decimal`]'s range. Use
    /// [`Balances::try_sub`] to handle overflow without panicking.
    #[inline]
    #[expect(
        clippy::panic,
        reason = "panics are intentional for unchecked arithmetic; see docstring"
    )]
    fn sub_assign(&mut self, rhs: &Amount) {
        match self.try_sub(rhs) {
            Ok(()) => {}
            Err(error) => panic!("Balances -= {error}"),
        }
    }
}

impl Extend<Amount> for Balances {
    #[inline]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "intentional += accumulation; side effect is the desired behavior"
    )]
    fn extend<I>(&mut self, iter: I)
    where
        I: IntoIterator<Item = Amount>,
    {
        for amount in iter {
            *self += &amount;
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
    use rust_decimal::Decimal;

    use super::Balances;
    use crate::Amount;

    fn aud(units: i64) -> Amount {
        Amount::new(Decimal::new(units, 0), "AUD")
    }

    fn usd(units: i64) -> Amount {
        Amount::new(Decimal::new(units, 0), "USD")
    }

    #[test]
    fn nets_per_commodity() {
        let mut b = Balances::new();
        b += &aud(10);
        b += &usd(50);
        b -= &usd(10);
        assert_eq!(b.get("AUD"), Some(Decimal::new(10, 0)));
        assert_eq!(b.get("USD"), Some(Decimal::new(40, 0)));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn drops_commodity_on_zero() {
        let mut b = Balances::new();
        b += &usd(10);
        b -= &usd(10);
        assert!(b.is_empty());
        assert_eq!(b.get("USD"), None);
    }

    #[test]
    fn collect_matches_fold() {
        let amounts = [aud(10), usd(50), aud(5)];
        let collected: Balances = amounts.iter().cloned().collect();
        let folded = amounts.iter().fold(Balances::new(), |mut acc, a| {
            acc += a;
            acc
        });
        assert_eq!(collected.get("AUD"), Some(Decimal::new(15, 0)));
        assert_eq!(collected.get("AUD"), folded.get("AUD"));
    }

    #[test]
    #[should_panic(expected = "Balances")]
    fn add_assign_overflow_panics() {
        let mut b = Balances::new();
        b += &Amount::new(Decimal::MAX, "AUD");
        b += &aud(1);
    }
}
