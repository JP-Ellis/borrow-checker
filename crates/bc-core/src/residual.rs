//! Residual arithmetic for elided postings.
//!
//! A posting whose amount the source document elides absorbs its transaction's
//! residual — the negation of its sibling legs' sum, per commodity. Nothing is
//! persisted: the residual is derived on every read, so it stays correct when a
//! sibling leg changes (`docs/DESIGN.md` §4.4). Database reads resolve it
//! in-stream through `crate::legs`; callers holding whole transactions use
//! [`residual_of`] or [`residual_of_postings`].

use bc_models::Amount;
use bc_models::AmountError;
use bc_models::Balances;
use bc_models::Posting;
use rust_decimal::Decimal;

/// The residual a transaction's elided leg absorbs.
#[expect(
    clippy::exhaustive_enums,
    reason = "callers match on all three variants; a new variant is a deliberate breaking change they should feel"
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

/// A transaction's residual over an arbitrary commodity key.
///
/// The streaming resolver keys commodities by interned handle; [`residual_of`]
/// keys them by [`bc_models::CommodityCode`]. Both share this arithmetic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum KeyedResidual<K> {
    /// No leg is elided.
    NotElided,
    /// Exactly one leg is elided and absorbs these per-commodity totals, in
    /// first-seen order. A total that reaches zero is dropped, and a commodity
    /// that returns after its drop is appended at the end.
    Attributable(Vec<(K, Decimal)>),
    /// Two or more legs are elided.
    Ambiguous,
}

/// Adds `delta` to `key`'s entry, as [`Balances`] does.
///
/// Entries keep first-seen order. A total that reaches zero is dropped, and a
/// later delta for the same key appends a new entry at the end.
fn accumulate<K>(
    entries: &mut Vec<(K, Decimal)>,
    key: &K,
    delta: Decimal,
) -> Result<(), AmountError>
where
    K: PartialEq + Clone,
{
    if let Some(entry) = entries.iter_mut().find(|(k, _)| k == key) {
        entry.1 = entry.1.checked_add(delta).ok_or(AmountError::Overflow)?;
        entries.retain(|(_, value)| !value.is_zero());
    } else if !delta.is_zero() {
        entries.push((key.clone(), delta));
    }
    Ok(())
}

/// Computes a transaction's residual from its legs' keyed weights.
///
/// # Arguments
///
/// * `weights` - One entry per leg: `Some((commodity, value))` for a concrete
///   weight, `None` for an elided leg.
///
/// # Returns
///
/// The residual, classified by the number of elided legs.
///
/// # Errors
///
/// Returns [`AmountError::Overflow`] if a negation or a per-commodity total
/// overflows.
pub(crate) fn keyed_residual<K, I>(weights: I) -> Result<KeyedResidual<K>, AmountError>
where
    K: PartialEq + Clone,
    I: IntoIterator<Item = Option<(K, Decimal)>>,
{
    let mut entries = Vec::new();
    let mut elided = 0_usize;
    for weight in weights {
        match weight {
            Some((key, value)) => {
                // Subtracting accumulates the negation, which is the residual.
                let negated = Decimal::ZERO
                    .checked_sub(value)
                    .ok_or(AmountError::Overflow)?;
                accumulate(&mut entries, &key, negated)?;
            }
            None => elided = elided.saturating_add(1),
        }
    }
    Ok(match elided {
        0 => KeyedResidual::NotElided,
        1 => KeyedResidual::Attributable(entries),
        _ => KeyedResidual::Ambiguous,
    })
}

/// Computes a transaction's residual from its legs' amounts.
///
/// # Arguments
///
/// * `amounts` - One entry per leg: `Some` for a concrete weight, `None` for an
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
    reason = "the residual_ prefix is the function's meaning, not a repetition of the module name"
)]
pub fn residual_of<'a, I>(amounts: I) -> Result<Residual, AmountError>
where
    I: IntoIterator<Item = Option<&'a Amount>>,
{
    let keyed = keyed_residual(
        amounts
            .into_iter()
            .map(|amount| amount.map(|a| (a.commodity(), a.value()))),
    )?;
    Ok(match keyed {
        KeyedResidual::NotElided => Residual::NotElided,
        KeyedResidual::Ambiguous => Residual::Ambiguous,
        KeyedResidual::Attributable(entries) => {
            let mut balances = Balances::new();
            for (code, value) in entries {
                balances.try_add(&Amount::new(value, code.clone()))?;
            }
            Residual::Attributable(balances)
        }
    })
}

/// Computes a transaction's residual from its postings' weights.
///
/// Each leg contributes [`Posting::weight`] — at cost, else at price, else its
/// amount — so a priced or costed leg funds the residual in the quote's
/// commodity, as Beancount does.
///
/// # Errors
///
/// Returns [`AmountError::Overflow`] if a weight or a per-commodity total
/// overflows.
#[expect(
    clippy::module_name_repetitions,
    reason = "the residual_ prefix is the function's meaning, not a repetition of the module name"
)]
pub fn residual_of_postings<'a, I>(postings: I) -> Result<Residual, AmountError>
where
    I: IntoIterator<Item = &'a Posting>,
{
    let weights = postings
        .into_iter()
        .map(Posting::weight)
        .collect::<Result<Vec<_>, _>>()?;
    residual_of(weights.iter().map(Option::as_ref))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Quote;
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

    /// Drives `keyed_residual` and `residual_of` over the same legs and
    /// compares the entries in order.
    fn keyed_matches_balances(legs: &[Option<Amount>]) {
        let via_key = keyed_residual(
            legs.iter()
                .map(|leg| leg.as_ref().map(|a| (a.commodity().as_str(), a.value()))),
        )
        .expect("keyed residual");
        let via_code = residual_of(legs.iter().map(Option::as_ref)).expect("residual");
        match (via_key, via_code) {
            (KeyedResidual::NotElided, Residual::NotElided)
            | (KeyedResidual::Ambiguous, Residual::Ambiguous) => {}
            (KeyedResidual::Attributable(entries), Residual::Attributable(totals)) => {
                let expected: Vec<(&str, Decimal)> = totals.iter().collect();
                assert_eq!(entries, expected);
            }
            (left, right) => panic!("diverged: {left:?} vs {right:?}"),
        }
    }

    #[rstest]
    #[case::not_elided(vec![Some(aud(dec!(5))), Some(aud(dec!(-5)))])]
    #[case::one_elided(vec![Some(aud(dec!(50))), None])]
    #[case::ambiguous(vec![Some(aud(dec!(50))), None, None])]
    #[case::lone_elided(vec![None])]
    #[case::two_commodities(vec![Some(aud(dec!(3))), Some(usd(dec!(7))), None])]
    #[case::cancel_reorders(vec![
        Some(usd(dec!(5))),
        Some(aud(dec!(3))),
        Some(usd(dec!(-5))),
        Some(usd(dec!(2))),
        None,
    ])]
    fn keyed_residual_matches_residual_of(#[case] legs: Vec<Option<Amount>>) {
        keyed_matches_balances(&legs);
    }

    #[test]
    fn keyed_residual_drops_a_cancelled_commodity_and_appends_on_return() {
        let legs = [
            Some(("USD", dec!(5))),
            Some(("AUD", dec!(3))),
            Some(("USD", dec!(-5))),
            Some(("USD", dec!(2))),
            None,
        ];
        let residual = keyed_residual(legs).expect("residual");
        assert_eq!(
            residual,
            KeyedResidual::Attributable(vec![("AUD", dec!(-3)), ("USD", dec!(-2))])
        );
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

    #[test]
    fn residual_of_postings_uses_weights() {
        let usd = Posting::builder()
            .id(PostingId::new())
            .account_id(AccountId::new())
            .amount(Amount::new(dec!(4.00), "USD"))
            .price(Quote::Total(Amount::new(dec!(6.37), "AUD")))
            .build();
        let bank = Posting::builder()
            .id(PostingId::new())
            .account_id(AccountId::new())
            .build();
        let Residual::Attributable(balances) =
            residual_of_postings([&usd, &bank]).expect("residual")
        else {
            panic!("expected attributable");
        };
        assert_eq!(balances.get("AUD"), Some(dec!(-6.37)));
        assert_eq!(balances.get("USD"), None);
    }
}
