//! Column encoding for a posting's price and cost basis.
//!
//! Both annotations are a [`Quote`] stored as three nullable columns:
//! `<prefix>_value` (decimal string), `<prefix>_commodity`, and
//! `<prefix>_kind` (`'unit'` or `'total'`). All three are NULL together.

use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Cost;
use bc_models::Quote;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::BcError;
use crate::BcResult;

/// `*_kind` value for the per-unit form.
pub(crate) const KIND_UNIT: &str = "unit";
/// `*_kind` value for the total form.
pub(crate) const KIND_TOTAL: &str = "total";

/// Splits a quote into its `(value, commodity, kind)` column values.
pub(crate) fn quote_columns(
    quote: Option<&Quote>,
) -> (Option<String>, Option<String>, Option<&'static str>) {
    match quote {
        Some(q) => (
            Some(q.amount().value().to_string()),
            Some(q.commodity().as_str().to_owned()),
            Some(if q.is_total() { KIND_TOTAL } else { KIND_UNIT }),
        ),
        None => (None, None, None),
    }
}

/// Rebuilds a quote from its three columns.
///
/// # Arguments
///
/// * `what` - Column prefix for error messages (`"price"` or `"cost"`).
/// * `value`, `commodity`, `kind` - The three columns.
///
/// # Returns
///
/// `None` when `value` is NULL.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if the columns disagree about nullness, the
/// value does not parse, or the kind is neither `unit` nor `total`.
pub(crate) fn parse_quote(
    what: &str,
    value: Option<String>,
    commodity: Option<String>,
    kind: Option<String>,
) -> BcResult<Option<Quote>> {
    let Some(value_str) = value else {
        return Ok(None);
    };
    let commodity_str = commodity.ok_or_else(|| {
        BcError::BadData(format!(
            "{what}_commodity is NULL with non-NULL {what}_value"
        ))
    })?;
    let kind_str = kind.ok_or_else(|| {
        BcError::BadData(format!("{what}_kind is NULL with non-NULL {what}_value"))
    })?;
    let decimal = value_str
        .parse::<Decimal>()
        .map_err(|e| BcError::BadData(format!("invalid {what}_value '{value_str}': {e}")))?;
    let amount = Amount::new(decimal, CommodityCode::new(commodity_str));
    match kind_str.as_str() {
        KIND_UNIT => Ok(Some(Quote::PerUnit(amount))),
        KIND_TOTAL => Ok(Some(Quote::Total(amount))),
        other => Err(BcError::BadData(format!("invalid {what}_kind '{other}'"))),
    }
}

/// Rebuilds a cost basis from its five columns.
///
/// # Returns
///
/// `None` when `cost_value` is NULL.
///
/// # Errors
///
/// As [`parse_quote`], plus an unparsable `cost_date`.
#[expect(
    clippy::needless_pass_by_value,
    reason = "all parameters come from owned DB rows; passing by value is ergonomic at call sites"
)]
pub(crate) fn parse_cost(
    value: Option<String>,
    commodity: Option<String>,
    kind: Option<String>,
    cost_date: Option<String>,
    cost_label: Option<String>,
) -> BcResult<Option<Cost>> {
    let Some(basis) = parse_quote("cost", value, commodity, kind)? else {
        return Ok(None);
    };
    let date = cost_date
        .as_deref()
        .map(|s| {
            s.parse::<Date>()
                .map_err(|e| BcError::BadData(format!("invalid cost_date '{s}': {e}")))
        })
        .transpose()?;
    Ok(Some(
        Cost::builder()
            .basis(basis)
            .maybe_date(date)
            .maybe_label(cost_label)
            .build(),
    ))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    #[rstest]
    #[case::unit(Quote::PerUnit(Amount::new(dec!(1.5), "AUD")), "unit")]
    #[case::total(Quote::Total(Amount::new(dec!(3), "AUD")), "total")]
    fn quote_columns_round_trip(#[case] quote: Quote, #[case] kind: &str) {
        let (value, commodity, stored_kind) = quote_columns(Some(&quote));
        assert_eq!(stored_kind, Some(kind));
        let back =
            parse_quote("price", value, commodity, stored_kind.map(str::to_owned)).expect("parses");
        assert_eq!(back, Some(quote));
    }

    #[test]
    fn absent_quote_is_all_null() {
        assert_eq!(quote_columns(None), (None, None, None));
        assert_eq!(
            parse_quote("price", None, None, None).expect("parses"),
            None
        );
    }

    #[rstest]
    #[case::no_commodity(Some("1"), None, Some("unit"), "price_commodity is NULL")]
    #[case::no_kind(Some("1"), Some("AUD"), None, "price_kind is NULL")]
    #[case::bad_kind(Some("1"), Some("AUD"), Some("each"), "invalid price_kind 'each'")]
    #[case::bad_value(Some("x"), Some("AUD"), Some("unit"), "invalid price_value 'x'")]
    fn parse_quote_names_the_bad_column(
        #[case] value: Option<&str>,
        #[case] commodity: Option<&str>,
        #[case] kind: Option<&str>,
        #[case] expected: &str,
    ) {
        let err = parse_quote(
            "price",
            value.map(str::to_owned),
            commodity.map(str::to_owned),
            kind.map(str::to_owned),
        )
        .expect_err("rejects");
        assert!(err.to_string().contains(expected), "{err}");
    }
}
