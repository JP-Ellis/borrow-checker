//! Reads one numeric cell.
//!
//! A cell may carry a *denomination* — a currency symbol, a commodity code,
//! or a mix such as `A$` — before or after the magnitude. The parsers here
//! strip it, and [`parse_amount_cell`] also returns it so the row loop can
//! compare it with the commodity the cell is configured to post in.

use std::collections::BTreeSet;

use rust_decimal::Decimal;

/// Inclusive code-point ranges of Unicode general category `Sc` (currency
/// symbol), Unicode 16.0, sorted for binary search.
const CURRENCY_SYMBOL_RANGES: &[(u32, u32)] = &[
    (0x0024, 0x0024),
    (0x00A2, 0x00A5),
    (0x058F, 0x058F),
    (0x060B, 0x060B),
    (0x07FE, 0x07FF),
    (0x09F2, 0x09F3),
    (0x09FB, 0x09FB),
    (0x0AF1, 0x0AF1),
    (0x0BF9, 0x0BF9),
    (0x0E3F, 0x0E3F),
    (0x17DB, 0x17DB),
    (0x20A0, 0x20C1),
    (0xA838, 0xA838),
    (0xFDFC, 0xFDFC),
    (0xFE69, 0xFE69),
    (0xFF04, 0xFF04),
    (0xFFE0, 0xFFE1),
    (0xFFE5, 0xFFE6),
    (0x1_1FDD, 0x1_1FE0),
    (0x1_E2FF, 0x1_E2FF),
    (0x1_ECB0, 0x1_ECB0),
];

/// Returns whether `c` is a Unicode currency symbol (general category `Sc`).
///
/// # Arguments
///
/// * `c` - The character to classify.
#[inline]
pub(crate) fn is_currency_symbol(c: char) -> bool {
    let point = u32::from(c);
    CURRENCY_SYMBOL_RANGES
        .binary_search_by(|&(start, end)| {
            if point < start {
                std::cmp::Ordering::Greater
            } else if point > end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

/// Returns whether `c` can be part of a denomination.
#[inline]
fn is_denomination_char(c: char) -> bool {
    c.is_alphabetic() || is_currency_symbol(c)
}

/// A numeric cell split into its denomination and magnitude text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SplitCell<'a> {
    /// The symbol or code the cell states, when it states one.
    pub(crate) denomination: Option<&'a str>,
    /// The remaining text, trimmed, which should be a signed magnitude.
    pub(crate) magnitude: &'a str,
}

/// Splits a denomination off either end of `text`.
///
/// A denomination is a run of alphabetic or currency-symbol characters at
/// the start or the end of `text`, with any whitespace between it and the
/// digits discarded.
///
/// # Arguments
///
/// * `text` - The cell text after any outer sign has been removed.
///
/// # Returns
///
/// The denomination, if any, and the rest.
///
/// # Errors
///
/// Returns a message when `text` carries a denomination at both ends.
pub(crate) fn split_denomination(text: &str) -> Result<SplitCell<'_>, String> {
    let text = text.trim();
    let prefix_end = text
        .char_indices()
        .find(|&(_, c)| !is_denomination_char(c))
        .map_or(text.len(), |(index, _)| index);
    let (prefix, rest) = text.split_at(prefix_end);
    let rest = rest.trim_start();

    let suffix_start = rest
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_denomination_char(c))
        .last()
        .map_or(rest.len(), |(index, _)| index);
    let (magnitude, suffix) = rest.split_at(suffix_start);
    let magnitude = magnitude.trim_end();

    match (prefix.is_empty(), suffix.is_empty()) {
        (false, false) => Err(format!(
            "'{text}' carries two denominations, '{prefix}' and '{suffix}'"
        )),
        (false, true) => Ok(SplitCell {
            denomination: Some(prefix),
            magnitude,
        }),
        (true, false) => Ok(SplitCell {
            denomination: Some(suffix),
            magnitude,
        }),
        (true, true) => Ok(SplitCell {
            denomination: None,
            magnitude,
        }),
    }
}

/// Parses a numeric cell, returning its value and the denomination it states.
///
/// Accepts an optional leading `-` or accounting parentheses for the sign,
/// a denomination on either side of the digits, a `-` between a leading
/// denomination and the digits when no sign has been seen yet, a `+`, a
/// configured thousands separator, and a configured decimal separator.
///
/// # Arguments
///
/// * `raw` - The cell text.
/// * `decimal_sep` - The decimal separator character in use.
/// * `thousands_sep` - An optional thousands separator to strip.
///
/// # Returns
///
/// The value and the denomination, when the cell states one.
///
/// # Errors
///
/// Returns a [`String`] describing why `raw` is not a number.
pub(crate) fn parse_amount_cell(
    raw: &str,
    decimal_sep: char,
    thousands_sep: Option<char>,
) -> Result<(Decimal, Option<String>), String> {
    let trimmed = raw.trim();

    // Accounting notation (50.00) and a leading minus both mean negative.
    let (mut negative, body) =
        if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            (true, inner)
        } else if let Some(rest) = trimmed.strip_prefix('-') {
            (true, rest)
        } else {
            (false, trimmed)
        };

    let split = split_denomination(body).map_err(|e| format!("cannot parse '{raw}': {e}"))?;

    // A sign may also sit between a leading denomination and the digits.
    let magnitude = if let Some(rest) = split.magnitude.strip_prefix('-') {
        if negative {
            return Err(format!("cannot parse '{raw}': it carries two signs"));
        }
        negative = true;
        rest.trim_start()
    } else {
        split.magnitude
    };
    let magnitude = magnitude.trim_matches('+').trim();

    let mut digits = String::with_capacity(magnitude.len().saturating_add(1));
    if negative {
        digits.push('-');
    }
    for c in magnitude.chars() {
        if Some(c) == thousands_sep {
            continue;
        }
        digits.push(if c == decimal_sep { '.' } else { c });
    }

    let value = digits
        .parse::<Decimal>()
        .map_err(|e| format!("cannot parse '{raw}' as a decimal: {e}"))?;
    Ok((value, split.denomination.map(str::to_owned)))
}

/// Parses a numeric cell, discarding any denomination it states.
///
/// See [`parse_amount_cell`] for the accepted shapes.
///
/// # Arguments
///
/// * `raw` - The cell text.
/// * `decimal_sep` - The decimal separator character in use.
/// * `thousands_sep` - An optional thousands separator to strip.
///
/// # Returns
///
/// The parsed value.
///
/// # Errors
///
/// Returns a [`String`] describing why `raw` is not a number.
#[inline]
pub(crate) fn parse_number(
    raw: &str,
    decimal_sep: char,
    thousands_sep: Option<char>,
) -> Result<Decimal, String> {
    parse_amount_cell(raw, decimal_sep, thousands_sep).map(|(value, _)| value)
}

/// A cell whose stated denomination differs from the commodity it posts in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DenominationMismatch {
    /// The configured column, as [`crate::config::ColumnRef::describe`] spells it.
    pub(crate) field: String,
    /// The denomination the cell states, after aliasing.
    pub(crate) cell: String,
    /// The commodity the profile posts the cell in.
    pub(crate) commodity: String,
    /// The first data row the mismatch was seen on, 1-based.
    pub(crate) row: usize,
}

/// Decides, once per file, whether a cell's denomination deserves a warning.
///
/// A profile posting a `$` column in `AUD` would otherwise warn on every
/// row; one warning per distinct `(field, cell, commodity)` says the same
/// thing and names the first row.
#[derive(Debug, Default)]
pub(crate) struct DenominationCheck {
    seen: BTreeSet<(String, String, String)>,
}

impl DenominationCheck {
    /// Creates a check with nothing reported yet.
    #[inline]
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Compares a cell's denomination with the commodity it posts in.
    ///
    /// # Arguments
    ///
    /// * `field` - The configured column the cell came from.
    /// * `cell` - The cell's denomination, after aliasing, if it states one.
    /// * `commodity` - The commodity the cell posts in.
    /// * `row` - The 1-based data-row number.
    ///
    /// # Returns
    ///
    /// The mismatch on the first row it is seen for this field; `None` when
    /// the cell states no denomination, states the same commodity, or the
    /// triple was already reported for this file.
    pub(crate) fn check(
        &mut self,
        field: &str,
        cell: Option<&str>,
        commodity: &str,
        row: usize,
    ) -> Option<DenominationMismatch> {
        let cell = cell?;
        if cell == commodity {
            return None;
        }
        let key = (field.to_owned(), cell.to_owned(), commodity.to_owned());
        if !self.seen.insert(key) {
            return None;
        }
        Some(DenominationMismatch {
            field: field.to_owned(),
            cell: cell.to_owned(),
            commodity: commodity.to_owned(),
            row,
        })
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn parse_number_strips_currency_symbols() {
        assert_eq!(parse_number("$50.00", '.', None), Ok(dec!(50.00)));
        assert_eq!(parse_number("£100.50", '.', None), Ok(dec!(100.50)));
        assert_eq!(parse_number("€9.99", '.', None), Ok(dec!(9.99)));
    }

    #[test]
    fn parse_number_strips_thousands_separator() {
        assert_eq!(parse_number("1,234.56", '.', Some(',')), Ok(dec!(1234.56)));
    }

    #[test]
    fn parse_number_normalises_decimal_separator() {
        assert_eq!(parse_number("1234,56", ',', None), Ok(dec!(1234.56)));
    }

    #[test]
    fn parse_number_negative() {
        assert_eq!(parse_number("-50.00", '.', None), Ok(dec!(-50.00)));
    }

    #[test]
    fn parse_number_parenthesised_accounting_notation() {
        // Many Australian bank exports use (50.00) to represent a debit.
        assert_eq!(parse_number("(50.00)", '.', None), Ok(dec!(-50.00)));
        assert_eq!(
            parse_number("(1,234.56)", '.', Some(',')),
            Ok(dec!(-1234.56))
        );
        assert_eq!(parse_number("($99.95)", '.', None), Ok(dec!(-99.95)));
    }

    #[rstest]
    #[case("AUD 2,887.07", '.', Some(','), dec!(2887.07))]
    #[case("AUD2887.07", '.', None, dec!(2887.07))]
    #[case("-AUD 5.00", '.', None, dec!(-5.00))]
    #[case("(AUD 5.00)", '.', None, dec!(-5.00))]
    #[case("AUD -5.00", '.', None, dec!(-5.00))]
    #[case("$-50.00", '.', None, dec!(-50.00))]
    #[case("A$5", '.', None, dec!(5))]
    #[case("US$ 5", '.', None, dec!(5))]
    #[case("¥1000", '.', None, dec!(1000))]
    #[case("₹1,00,000", '.', Some(','), dec!(100000))]
    #[case("₩5000", '.', None, dec!(5000))]
    #[case("USD 1.234,56", ',', Some('.'), dec!(1234.56))]
    fn parse_number_strips_leading_currency_code(
        #[case] raw: &str,
        #[case] decimal_sep: char,
        #[case] thousands_sep: Option<char>,
        #[case] expected: Decimal,
    ) {
        assert_eq!(parse_number(raw, decimal_sep, thousands_sep), Ok(expected));
    }

    #[rstest]
    #[case("500 AUD", '.', None, dec!(500))]
    #[case("500AUD", '.', None, dec!(500))]
    #[case("-500 AUD", '.', None, dec!(-500))]
    #[case("(500 AUD)", '.', None, dec!(-500))]
    #[case("1.234,56 EUR", ',', Some('.'), dec!(1234.56))]
    #[case("50.00 $", '.', None, dec!(50.00))]
    fn parse_number_strips_trailing_currency_code(
        #[case] raw: &str,
        #[case] decimal_sep: char,
        #[case] thousands_sep: Option<char>,
        #[case] expected: Decimal,
    ) {
        assert_eq!(parse_number(raw, decimal_sep, thousands_sep), Ok(expected));
    }

    #[test]
    fn parse_number_rejects_two_denominations() {
        let err = parse_number("AUD 5 USD", '.', None).expect_err("two codes");
        assert!(err.contains("two denominations"), "{err}");
    }

    #[test]
    fn parse_number_rejects_two_signs() {
        let err = parse_number("-AUD -5.00", '.', None).expect_err("two signs");
        assert!(err.contains("two signs"), "{err}");
    }

    #[test]
    fn parse_number_rejects_a_bare_code() {
        assert!(parse_number("abc", '.', None).is_err());
    }

    #[rstest]
    #[case("AUD 5.00", Some("AUD"))]
    #[case("5.00 AUD", Some("AUD"))]
    #[case("US$ 5.00", Some("US$"))]
    #[case("-$5.00", Some("$"))]
    #[case("5.00", None)]
    #[case("(5.00)", None)]
    fn parse_amount_cell_returns_the_denomination(
        #[case] raw: &str,
        #[case] expected: Option<&str>,
    ) {
        let (_, denomination) = parse_amount_cell(raw, '.', None).expect("parses");
        assert_eq!(denomination.as_deref(), expected);
    }

    #[rstest]
    #[case('$', true)]
    #[case('€', true)]
    #[case('₿', true)]
    #[case('﷼', true)]
    #[case('A', false)]
    #[case('5', false)]
    #[case(' ', false)]
    fn currency_symbol_membership(#[case] c: char, #[case] expected: bool) {
        assert_eq!(is_currency_symbol(c), expected);
    }

    #[test]
    fn denomination_check_reports_a_mismatch_once_per_triple() {
        let mut check = DenominationCheck::new();

        let first = check.check("column 'Amount'", Some("USD"), "AUD", 1);
        assert_eq!(
            first,
            Some(DenominationMismatch {
                field: "column 'Amount'".to_owned(),
                cell: "USD".to_owned(),
                commodity: "AUD".to_owned(),
                row: 1,
            })
        );
        assert_eq!(
            check.check("column 'Amount'", Some("USD"), "AUD", 2),
            None,
            "the same triple on a later row is already reported"
        );
        assert!(
            check
                .check("column 'Balance'", Some("USD"), "AUD", 2)
                .is_some(),
            "a different field is a different triple"
        );
    }

    #[test]
    fn denomination_check_passes_a_match() {
        let mut check = DenominationCheck::new();
        assert_eq!(check.check("column 'Amount'", Some("AUD"), "AUD", 1), None);
    }

    #[test]
    fn denomination_check_ignores_a_bare_cell() {
        let mut check = DenominationCheck::new();
        assert_eq!(check.check("column 'Amount'", None, "AUD", 1), None);
    }
}
