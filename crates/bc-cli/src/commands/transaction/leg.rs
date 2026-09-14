//! Text form of a posting leg for `--posting`:
//! `AMOUNT:CCY[{COST}|{{COST}}][@AMOUNT:CCY|@@AMOUNT:CCY]`.
//!
//! The parser returns plain data so the mapping into `bc_models` stays in
//! one place (`parse_posting_spec`) and the grammar can move to a crate the
//! GUI shares without dragging `bc_models` along.

use core::str::FromStr as _;

use rust_decimal::Decimal;

/// Whether a stated figure is per unit of the leg's amount or a total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `@ 150 AUD`, `{105 AUD}`.
    PerUnit,
    /// `@@ 6.37 AUD`, `{{210 AUD}}`.
    Total,
}

/// A number and a commodity code: `4.00:USD`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Figure {
    /// The number.
    pub value: Decimal,
    /// The commodity code, as written.
    pub code: String,
}

/// A price annotation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Price {
    /// `@` or `@@`.
    pub kind: Kind,
    /// The stated figure.
    pub figure: Figure,
}

/// A cost block: `{105:AUD:2024-03-01:lot-a}` or `{{210:AUD}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostBlock {
    /// Single braces are per unit, double braces are a total.
    pub kind: Kind,
    /// The stated cost figure.
    pub basis: Figure,
    /// Lot date, if stated.
    pub date: Option<jiff::civil::Date>,
    /// Lot label, if stated, without surrounding quotes.
    pub label: Option<String>,
}

/// A parsed leg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leg {
    /// The leg's own amount.
    pub units: Figure,
    /// The cost block, if any.
    pub cost: Option<CostBlock>,
    /// The price, if any.
    pub price: Option<Price>,
}

/// Parses a leg.
///
/// # Errors
///
/// Returns a message naming the first thing wrong, worded as the Beancount
/// importer words the same fault.
pub fn parse_leg(text: &str) -> Result<Leg, String> {
    #[expect(clippy::shadow_reuse, reason = "trim yields the same string")]
    let text = text.trim();
    let units_end = text.find(['{', '@']).unwrap_or(text.len());
    #[expect(
        clippy::string_slice,
        reason = "units_end is a valid char boundary from find()"
    )]
    let (units, leftover) = parse_figure(&text[..units_end])?;
    if !leftover.is_empty() {
        return Err(format!("unexpected '{leftover}' after the amount"));
    }

    #[expect(
        clippy::string_slice,
        reason = "units_end is a valid char boundary from find()"
    )]
    let mut rest = text[units_end..].trim_start();
    let cost = if rest.starts_with('{') {
        let (block, after) = parse_cost_block(rest)?;
        rest = after.trim_start();
        Some(block)
    } else {
        None
    };

    let price = if let Some(after) = rest.strip_prefix("@@") {
        rest = "";
        Some(parse_price(after, Kind::Total)?)
    } else if let Some(after) = rest.strip_prefix('@') {
        rest = "";
        Some(parse_price(after, Kind::PerUnit)?)
    } else {
        None
    };

    if !rest.is_empty() {
        return Err(format!("unexpected '{rest}' after the cost block"));
    }
    Ok(Leg { units, cost, price })
}

/// Parses the text after `@` or `@@`.
fn parse_price(text: &str, kind: Kind) -> Result<Price, String> {
    #[expect(clippy::shadow_reuse, reason = "trim yields the same string")]
    let text = text.trim();
    if text.is_empty() {
        return Err("expected AMOUNT:COMMODITY after '@'".into());
    }
    let (figure, leftover) = parse_figure(text)?;
    if leftover.starts_with('{') {
        return Err("cost block must come before the price".into());
    }
    if !leftover.is_empty() {
        return Err(format!("unexpected '{leftover}' after the price"));
    }
    if figure.value.is_sign_negative() && !figure.value.is_zero() {
        return Err("negative price not allowed".into());
    }
    Ok(Price { kind, figure })
}

/// Parses `AMOUNT:CCY` at the start of `text`, returning the figure and
/// whatever follows the code. The code ends at whitespace, `{` or `@`.
fn parse_figure(text: &str) -> Result<(Figure, &str), String> {
    #[expect(clippy::shadow_reuse, reason = "trim yields the same string")]
    let text = text.trim();
    let Some((value_str, after_colon)) = text.split_once(':') else {
        return Err("expected AMOUNT:COMMODITY".into());
    };
    let value = Decimal::from_str(value_str.trim())
        .map_err(|e| format!("invalid amount '{}': {e}", value_str.trim()))?;
    let code_end = after_colon
        .find(|c: char| c.is_whitespace() || c == '{' || c == '@')
        .unwrap_or(after_colon.len());
    #[expect(
        clippy::string_slice,
        reason = "code_end is a valid char boundary from find()"
    )]
    let code = &after_colon[..code_end];
    if code.is_empty() {
        return Err("expected AMOUNT:COMMODITY".into());
    }
    Ok((
        Figure {
            value,
            code: code.to_owned(),
        },
        #[expect(
            clippy::string_slice,
            reason = "code_end is a valid char boundary from find()"
        )]
        after_colon[code_end..].trim(),
    ))
}

/// Parses a cost block starting at the `{` of `text`, returning the block
/// and the text after its closing brace.
fn parse_cost_block(_text: &str) -> Result<(CostBlock, &str), String> {
    Err("cost block is not supported yet".into())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::Figure;
    use super::Kind;
    use super::Leg;
    use super::Price;
    use super::parse_leg;

    fn fig(value: rust_decimal::Decimal, code: &str) -> Figure {
        Figure {
            value,
            code: code.to_owned(),
        }
    }

    #[test]
    fn bare_units_parse() {
        assert_eq!(
            parse_leg("50.00:AUD").expect("parses"),
            Leg {
                units: fig(dec!(50.00), "AUD"),
                cost: None,
                price: None,
            }
        );
    }

    #[rstest]
    #[case::per_unit("-2:AAPL@150:AUD", Kind::PerUnit, dec!(150))]
    #[case::total("4.00:USD@@6.37:AUD", Kind::Total, dec!(6.37))]
    #[case::spaces_around_marker("4.00:USD @@ 6.37:AUD", Kind::Total, dec!(6.37))]
    #[case::zero_price("4.00:USD@0:AUD", Kind::PerUnit, dec!(0))]
    fn price_forms_parse(
        #[case] text: &str,
        #[case] kind: Kind,
        #[case] value: rust_decimal::Decimal,
    ) {
        let leg = parse_leg(text).expect("parses");
        assert_eq!(
            leg.price,
            Some(Price {
                kind,
                figure: fig(value, "AUD"),
            })
        );
        assert_eq!(leg.cost, None);
    }

    #[rstest]
    #[case::no_commodity("50.00", "expected AMOUNT:COMMODITY")]
    #[case::empty_commodity("50.00:", "expected AMOUNT:COMMODITY")]
    #[case::bad_amount("abc:AUD", "invalid amount 'abc'")]
    #[case::trailing_after_units("4:USD x@6:AUD", "unexpected 'x' after the amount")]
    #[case::negative_price("4:USD@-6:AUD", "negative price not allowed")]
    #[case::price_without_figure("4:USD@", "expected AMOUNT:COMMODITY after '@'")]
    #[case::trailing_after_price("4:USD@6:AUD extra", "unexpected 'extra' after the price")]
    #[case::cost_after_price("4:USD@6:AUD{5:AUD}", "cost block must come before the price")]
    fn errors_name_the_problem(#[case] text: &str, #[case] expected: &str) {
        let err = parse_leg(text).expect_err("rejects");
        assert!(err.contains(expected), "got: {err}");
    }
}
