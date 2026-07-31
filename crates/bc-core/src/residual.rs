//! Residual resolution for elided postings.
//!
//! A posting whose amount the source document elides absorbs its transaction's
//! residual — the negation of its sibling legs' sum, per commodity. Nothing is
//! persisted: the residual is derived on every read, so it stays correct when a
//! sibling leg changes (`docs/DESIGN.md` §4.4).

use bc_models::Amount;
use bc_models::AmountError;
use bc_models::Balances;

/// The residual a transaction's elided leg absorbs.
#[expect(
    clippy::exhaustive_enums,
    reason = "Task 2 and Task 5 match on all three variants; a new variant is a deliberate breaking change they should feel"
)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Residual {
    /// No leg is elided, so there is nothing to derive.
    NotElided,
    /// Exactly one leg is elided and absorbs this per-commodity residual.
    ///
    /// Empty when the concrete legs already sum to zero, or when there are no
    /// concrete legs at all.
    Attributable(Balances),
    /// Two or more legs are elided. The residual is real but cannot be
    /// attributed to any single leg, so it contributes to no balance.
    Ambiguous,
}

/// Computes a transaction's residual from its legs' amounts.
///
/// # Arguments
///
/// * `amounts` - One entry per leg: `Some` for a concrete amount, `None` for an
///   elided leg. Order is irrelevant.
///
/// # Returns
///
/// [`Residual::Attributable`] carrying the negated per-commodity sum of the
/// concrete legs when exactly one leg is elided, [`Residual::Ambiguous`] when
/// two or more are, and [`Residual::NotElided`] when none is.
///
/// # Errors
///
/// Returns [`AmountError::Overflow`] if a per-commodity total would exceed
/// [`rust_decimal::Decimal`]'s range.
///
/// # Example
///
/// ```rust
/// use bc_core::residual::Residual;
/// use bc_core::residual::residual_of;
/// use bc_models::Amount;
/// use rust_decimal_macros::dec;
///
/// let food = Amount::new(dec!(50), "AUD");
/// let residual = residual_of([Some(&food), None]).expect("residual");
/// let Residual::Attributable(balances) = residual else {
///     panic!("expected an attributable residual");
/// };
/// assert_eq!(balances.get("AUD"), Some(dec!(-50)));
/// ```
#[inline]
#[expect(
    clippy::module_name_repetitions,
    reason = "residual_of is the module's sole public function; the brief mandates this exact name for Task 2 and Task 5"
)]
pub fn residual_of<'a, I>(amounts: I) -> Result<Residual, AmountError>
where
    I: IntoIterator<Item = Option<&'a Amount>>,
{
    let mut balances = Balances::new();
    let mut elided = 0_usize;
    for amount in amounts {
        match amount {
            // Subtracting accumulates the negation, which is the residual.
            Some(a) => balances.try_sub(a)?,
            None => elided = elided.saturating_add(1),
        }
    }
    match elided {
        0 => Ok(Residual::NotElided),
        1 => Ok(Residual::Attributable(balances)),
        _ => Ok(Residual::Ambiguous),
    }
}

#[cfg(test)]
mod tests {
    use bc_models::Amount;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    /// Builds a concrete AUD amount.
    fn aud(value: rust_decimal::Decimal) -> Amount {
        Amount::new(value, "AUD")
    }

    /// Builds a concrete USD amount.
    fn usd(value: rust_decimal::Decimal) -> Amount {
        Amount::new(value, "USD")
    }

    #[test]
    fn single_elided_leg_absorbs_the_negated_sum() {
        let food = aud(dec!(50));
        let residual = residual_of([Some(&food), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert_eq!(balances.get("AUD"), Some(dec!(-50)));
        assert_eq!(balances.len(), 1);
    }

    #[test]
    fn two_elided_legs_are_ambiguous() {
        let food = aud(dec!(50));
        let residual = residual_of([Some(&food), None, None]).expect("residual");
        assert_eq!(residual, Residual::Ambiguous);
    }

    #[test]
    fn no_elided_leg_yields_not_elided() {
        let debit = aud(dec!(50));
        let credit = aud(dec!(-50));
        let residual = residual_of([Some(&debit), Some(&credit)]).expect("residual");
        assert_eq!(residual, Residual::NotElided);
    }

    #[test]
    fn concrete_legs_summing_to_zero_leave_an_empty_residual() {
        let debit = aud(dec!(50));
        let credit = aud(dec!(-50));
        let residual = residual_of([Some(&debit), Some(&credit), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert!(balances.is_empty());
    }

    #[test]
    fn lone_elided_leg_has_an_empty_residual() {
        let residual = residual_of([None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert!(balances.is_empty());
    }

    #[test]
    fn residual_spans_every_commodity_the_siblings_use() {
        let a = aud(dec!(50));
        let u = usd(dec!(30));
        let residual = residual_of([Some(&a), Some(&u), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert_eq!(balances.get("AUD"), Some(dec!(-50)));
        assert_eq!(balances.get("USD"), Some(dec!(-30)));
        assert_eq!(balances.len(), 2);
    }

    #[rstest]
    #[case(dec!(50), dec!(-50))]
    #[case(dec!(-50), dec!(50))]
    #[case(dec!(0.01), dec!(-0.01))]
    fn residual_negates_the_sibling(
        #[case] sibling: rust_decimal::Decimal,
        #[case] expected: rust_decimal::Decimal,
    ) {
        let amount = aud(sibling);
        let residual = residual_of([Some(&amount), None]).expect("residual");
        let Residual::Attributable(balances) = residual else {
            panic!("expected an attributable residual");
        };
        assert_eq!(balances.get("AUD"), Some(expected));
    }
}
