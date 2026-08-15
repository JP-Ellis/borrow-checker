//! Fitting an incoming metadata value to its key's registered type.
//!
//! The rule is *coerce, else flag*, and it is total over every pairing of an
//! asserted type with a registered one:
//!
//! - Identity always fits.
//! - Anything fits a registered `text` key, as its canonical string form.
//! - Asserted `text` into a registered `T` is parsed with `T`'s canonical
//!   parser; a parse failure is a mismatch.
//! - Every other cross-type pair is a mismatch.
//!
//! Nothing here reads a database. An account path is therefore never resolved:
//! text holding a path does not parse as an id and mismatches. Callers resolve
//! account paths to ids before reaching this module.

use bc_models::MetaType;
use bc_models::MetaValue;

/// The outcome of fitting a value to a key's registered type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Coerced {
    /// The value fits. Its [`MetaValue::ty`] is the registered type.
    Fits(MetaValue),
    /// The value cannot be represented in the registered type. The caller
    /// stores its canonical string and flags the entry rather than rejecting
    /// it, so nothing is lost and no import blocks.
    Mismatch,
}

/// Fits `value` to `registered`.
///
/// # Arguments
///
/// * `value` - The incoming value, carrying its asserted type.
/// * `registered` - The type the key is registered as.
///
/// # Returns
///
/// [`Coerced::Fits`] carrying a value of type `registered`, or
/// [`Coerced::Mismatch`].
pub(crate) fn coerce(value: &MetaValue, registered: MetaType) -> Coerced {
    if value.ty() == registered {
        return Coerced::Fits(value.clone());
    }
    if registered == MetaType::Text {
        return Coerced::Fits(MetaValue::Text(value.canonical()));
    }
    // The two returns above have consumed identity and every registered `text`
    // key, so what remains is a cross-type pair into a narrow key. Only an
    // asserted `text` has a rescue.
    match *value {
        MetaValue::Text(ref text) => match registered.parse_value(text) {
            Ok(parsed) => Coerced::Fits(parsed),
            Err(_err) => Coerced::Mismatch,
        },
        MetaValue::Number(_)
        | MetaValue::Boolean(_)
        | MetaValue::Date(_)
        | MetaValue::Timestamp(_)
        | MetaValue::Amount(_)
        | MetaValue::Account(_) => Coerced::Mismatch,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    /// Every metadata type, in declaration order. The matrix walks this twice.
    const ALL_TYPES: [MetaType; 7] = [
        MetaType::Text,
        MetaType::Number,
        MetaType::Boolean,
        MetaType::Date,
        MetaType::Timestamp,
        MetaType::Amount,
        MetaType::Account,
    ];

    /// The fixed instant every sample and expectation below uses.
    fn stamp() -> Timestamp {
        Timestamp::from_second(1_700_000_000).expect("valid timestamp")
    }

    /// One representative value of each type.
    ///
    /// The match is exhaustive on purpose: an eighth [`MetaType`] variant fails
    /// to compile here, which forces the matrix below to be extended rather
    /// than silently under-covering the space.
    fn sample_value(ty: MetaType, account: &AccountId) -> MetaValue {
        match ty {
            MetaType::Text => MetaValue::Text("Generic Grocer".to_owned()),
            MetaType::Number => MetaValue::Number(dec!(1502.50)),
            MetaType::Boolean => MetaValue::Boolean(true),
            MetaType::Date => MetaValue::Date(date(2026, 1, 15)),
            MetaType::Timestamp => MetaValue::Timestamp(stamp()),
            MetaType::Amount => {
                MetaValue::Amount(Amount::new(dec!(42.00), CommodityCode::new("AUD")))
            }
            MetaType::Account => MetaValue::Account(account.clone()),
        }
    }

    /// Shorthand for a fit carrying a text value.
    fn fits_text(text: &str) -> Coerced {
        Coerced::Fits(MetaValue::Text(text.to_owned()))
    }

    #[test]
    fn sample_value_covers_every_type_exactly_once() {
        let account = AccountId::new();
        let produced: Vec<MetaType> = ALL_TYPES
            .iter()
            .map(|ty| sample_value(*ty, &account).ty())
            .collect();
        assert_eq!(produced, ALL_TYPES.to_vec());
        let distinct: HashSet<MetaType> = ALL_TYPES.into_iter().collect();
        assert_eq!(distinct.len(), 7, "ALL_TYPES lists each type once");
    }

    #[test]
    fn the_coercion_matrix_covers_all_forty_nine_pairs() {
        let account = AccountId::new();
        let mut actual: Vec<(MetaType, MetaType, Coerced)> = Vec::new();
        for asserted in ALL_TYPES {
            for registered in ALL_TYPES {
                let outcome = coerce(&sample_value(asserted, &account), registered);
                actual.push((asserted, registered, outcome));
            }
        }
        assert_eq!(actual.len(), 49, "seven asserted types by seven registered");

        let text = MetaValue::Text("Generic Grocer".to_owned());
        let number = MetaValue::Number(dec!(1502.50));
        let boolean = MetaValue::Boolean(true);
        let day = MetaValue::Date(date(2026, 1, 15));
        let instant = MetaValue::Timestamp(stamp());
        let amount = MetaValue::Amount(Amount::new(dec!(42.00), CommodityCode::new("AUD")));
        let account_value = MetaValue::Account(account.clone());

        let expected: Vec<(MetaType, MetaType, Coerced)> = vec![
            // Asserted text. "Generic Grocer" parses as nothing but text, so
            // the rescue path is exercised and fails six times.
            (MetaType::Text, MetaType::Text, Coerced::Fits(text)),
            (MetaType::Text, MetaType::Number, Coerced::Mismatch),
            (MetaType::Text, MetaType::Boolean, Coerced::Mismatch),
            (MetaType::Text, MetaType::Date, Coerced::Mismatch),
            (MetaType::Text, MetaType::Timestamp, Coerced::Mismatch),
            (MetaType::Text, MetaType::Amount, Coerced::Mismatch),
            (MetaType::Text, MetaType::Account, Coerced::Mismatch),
            // Asserted number.
            (MetaType::Number, MetaType::Text, fits_text("1502.50")),
            (MetaType::Number, MetaType::Number, Coerced::Fits(number)),
            (MetaType::Number, MetaType::Boolean, Coerced::Mismatch),
            (MetaType::Number, MetaType::Date, Coerced::Mismatch),
            (MetaType::Number, MetaType::Timestamp, Coerced::Mismatch),
            (MetaType::Number, MetaType::Amount, Coerced::Mismatch),
            (MetaType::Number, MetaType::Account, Coerced::Mismatch),
            // Asserted boolean.
            (MetaType::Boolean, MetaType::Text, fits_text("true")),
            (MetaType::Boolean, MetaType::Number, Coerced::Mismatch),
            (MetaType::Boolean, MetaType::Boolean, Coerced::Fits(boolean)),
            (MetaType::Boolean, MetaType::Date, Coerced::Mismatch),
            (MetaType::Boolean, MetaType::Timestamp, Coerced::Mismatch),
            (MetaType::Boolean, MetaType::Amount, Coerced::Mismatch),
            (MetaType::Boolean, MetaType::Account, Coerced::Mismatch),
            // Asserted date.
            (MetaType::Date, MetaType::Text, fits_text("2026-01-15")),
            (MetaType::Date, MetaType::Number, Coerced::Mismatch),
            (MetaType::Date, MetaType::Boolean, Coerced::Mismatch),
            (MetaType::Date, MetaType::Date, Coerced::Fits(day)),
            (MetaType::Date, MetaType::Timestamp, Coerced::Mismatch),
            (MetaType::Date, MetaType::Amount, Coerced::Mismatch),
            (MetaType::Date, MetaType::Account, Coerced::Mismatch),
            // Asserted timestamp.
            (
                MetaType::Timestamp,
                MetaType::Text,
                fits_text("2023-11-14T22:13:20Z"),
            ),
            (MetaType::Timestamp, MetaType::Number, Coerced::Mismatch),
            (MetaType::Timestamp, MetaType::Boolean, Coerced::Mismatch),
            (MetaType::Timestamp, MetaType::Date, Coerced::Mismatch),
            (
                MetaType::Timestamp,
                MetaType::Timestamp,
                Coerced::Fits(instant),
            ),
            (MetaType::Timestamp, MetaType::Amount, Coerced::Mismatch),
            (MetaType::Timestamp, MetaType::Account, Coerced::Mismatch),
            // Asserted amount.
            (MetaType::Amount, MetaType::Text, fits_text("42.00 AUD")),
            (MetaType::Amount, MetaType::Number, Coerced::Mismatch),
            (MetaType::Amount, MetaType::Boolean, Coerced::Mismatch),
            (MetaType::Amount, MetaType::Date, Coerced::Mismatch),
            (MetaType::Amount, MetaType::Timestamp, Coerced::Mismatch),
            (MetaType::Amount, MetaType::Amount, Coerced::Fits(amount)),
            (MetaType::Amount, MetaType::Account, Coerced::Mismatch),
            // Asserted account. Into text this yields the *id*; the storage
            // layer substitutes the path, which this layer cannot see.
            (
                MetaType::Account,
                MetaType::Text,
                fits_text(&account.to_string()),
            ),
            (MetaType::Account, MetaType::Number, Coerced::Mismatch),
            (MetaType::Account, MetaType::Boolean, Coerced::Mismatch),
            (MetaType::Account, MetaType::Date, Coerced::Mismatch),
            (MetaType::Account, MetaType::Timestamp, Coerced::Mismatch),
            (MetaType::Account, MetaType::Amount, Coerced::Mismatch),
            (
                MetaType::Account,
                MetaType::Account,
                Coerced::Fits(account_value),
            ),
        ];

        assert_eq!(actual, expected);
    }

    #[test]
    fn identity_always_fits_and_keeps_the_value() {
        let account = AccountId::new();
        for ty in ALL_TYPES {
            let value = sample_value(ty, &account);
            assert_eq!(
                coerce(&value, ty),
                Coerced::Fits(value.clone()),
                "identity must not reshape the value"
            );
        }
    }

    #[test]
    fn every_type_fits_a_text_key_as_its_canonical_form() {
        let account = AccountId::new();
        for ty in ALL_TYPES {
            let value = sample_value(ty, &account);
            assert_eq!(
                coerce(&value, MetaType::Text),
                fits_text(&value.canonical()),
                "a text key accepts anything, as its canonical string"
            );
        }
    }

    #[rstest]
    #[case(MetaType::Text, "anything at all", MetaValue::Text("anything at all".to_owned()))]
    #[case(MetaType::Number, "1502.50", MetaValue::Number(dec!(1502.50)))]
    #[case(MetaType::Boolean, "true", MetaValue::Boolean(true))]
    #[case(MetaType::Boolean, "false", MetaValue::Boolean(false))]
    #[case(MetaType::Date, "2026-01-15", MetaValue::Date(date(2026, 1, 15)))]
    fn text_is_rescued_by_the_registered_types_parser(
        #[case] registered: MetaType,
        #[case] text: &str,
        #[case] expected: MetaValue,
    ) {
        assert_eq!(
            coerce(&MetaValue::Text(text.to_owned()), registered),
            Coerced::Fits(expected),
            "phase 3's rescue: text that reads as the registered type is not flagged"
        );
    }

    #[test]
    fn text_is_rescued_into_a_timestamp_and_an_amount() {
        assert_eq!(
            coerce(
                &MetaValue::Text("2023-11-14T22:13:20Z".to_owned()),
                MetaType::Timestamp
            ),
            Coerced::Fits(MetaValue::Timestamp(stamp()))
        );
        assert_eq!(
            coerce(&MetaValue::Text("42.00 AUD".to_owned()), MetaType::Amount),
            Coerced::Fits(MetaValue::Amount(Amount::new(
                dec!(42.00),
                CommodityCode::new("AUD")
            )))
        );
    }

    #[test]
    fn text_holding_an_account_id_is_rescued_into_an_account() {
        let account = AccountId::new();
        assert_eq!(
            coerce(&MetaValue::Text(account.to_string()), MetaType::Account),
            Coerced::Fits(MetaValue::Account(account))
        );
    }

    #[test]
    fn text_holding_an_account_path_is_a_mismatch() {
        assert_eq!(
            coerce(
                &MetaValue::Text("Assets:Bank:Savings".to_owned()),
                MetaType::Account
            ),
            Coerced::Mismatch,
            "coercion is database-free, so it resolves no path; the caller \
             resolves paths to ids before it gets here"
        );
    }

    #[rstest]
    #[case(MetaType::Number, "not-a-number")]
    #[case(MetaType::Boolean, "yes")]
    #[case(MetaType::Date, "2026-13-99")]
    #[case(MetaType::Timestamp, "yesterday")]
    #[case(MetaType::Amount, "42.00")]
    #[case(MetaType::Account, "not an id")]
    fn text_that_does_not_parse_is_a_mismatch(#[case] registered: MetaType, #[case] text: &str) {
        assert_eq!(
            coerce(&MetaValue::Text(text.to_owned()), registered),
            Coerced::Mismatch
        );
    }
}
