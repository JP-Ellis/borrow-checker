//! Text form of a posting leg for `--posting`:
//! `AMOUNT:CCY[{COST}|{{COST}}][@AMOUNT:CCY|@@AMOUNT:CCY]`.
//!
//! The parser returns plain data so the mapping into `bc_models` stays in
//! one place (`parse_posting_spec`) and the grammar can move to a crate the
//! GUI shares without dragging `bc_models` along.

use core::fmt::Write as _;
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

/// Renders a leg in Beancount form: `4.00 USD @@ 6.37 AUD`,
/// `2 AAPL {105 AUD, 2024-03-01, "lot-a"} @ 150 AUD`.
///
/// Returns `None` for an elided leg.
#[must_use]
pub fn render_leg(posting: &bc_models::Posting) -> Option<String> {
    let amount = posting.amount()?;
    let mut out = render_amount(amount);
    if let Some(cost) = posting.cost() {
        let mut inner = render_amount(cost.basis().amount());
        if let Some(date) = cost.date() {
            #[expect(
                clippy::let_underscore_must_use,
                reason = "write! to String never fails"
            )]
            let _ = write!(inner, ", {date}");
        }
        if let Some(label) = cost.label() {
            #[expect(
                clippy::let_underscore_must_use,
                reason = "write! to String never fails"
            )]
            let _ = write!(inner, ", \"{label}\"");
        }
        match *cost.basis() {
            bc_models::Quote::PerUnit(_) => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "write! to String never fails"
                )]
                let _ = write!(out, " {{{inner}}}");
            }
            bc_models::Quote::Total(_) => {
                #[expect(
                    clippy::let_underscore_must_use,
                    reason = "write! to String never fails"
                )]
                let _ = write!(out, " {{{{{inner}}}}}");
            }
        }
    }
    match posting.price() {
        Some(bc_models::Quote::PerUnit(figure)) => {
            #[expect(
                clippy::let_underscore_must_use,
                reason = "write! to String never fails"
            )]
            let _ = write!(out, " @ {}", render_amount(figure));
        }
        Some(bc_models::Quote::Total(figure)) => {
            #[expect(
                clippy::let_underscore_must_use,
                reason = "write! to String never fails"
            )]
            let _ = write!(out, " @@ {}", render_amount(figure));
        }
        None => {}
    }
    Some(out)
}

/// `VALUE CODE`, as `tx list` has always printed an amount.
fn render_amount(amount: &bc_models::Amount) -> String {
    format!("{} {}", amount.value(), amount.commodity().as_str())
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
/// whatever follows the code. The code ends at whitespace or one of
/// `{ @ : , } "`, so a stray one of those is reported by the caller instead
/// of being folded into the commodity code.
fn parse_figure(text: &str) -> Result<(Figure, &str), String> {
    #[expect(clippy::shadow_reuse, reason = "trim yields the same string")]
    let text = text.trim();
    let Some((value_str, after_colon)) = text.split_once(':') else {
        return Err("expected AMOUNT:COMMODITY".into());
    };
    let value = Decimal::from_str(value_str.trim())
        .map_err(|e| format!("invalid amount '{}': {e}", value_str.trim()))?;
    let code_end = after_colon
        .find(|c: char| c.is_whitespace() || matches!(c, '{' | '@' | ':' | ',' | '}' | '"'))
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

/// The wording for every form of lot selection (`{}`, `{2024-03-01}`, `*`),
/// none of which the store can honour without inventory booking.
const LOT_SELECTION: &str = "lot selection needs inventory booking, not supported yet (#518)";

/// Parses a cost block starting at the `{` of `text`, returning the block
/// and the text after its closing brace.
fn parse_cost_block(text: &str) -> Result<(CostBlock, &str), String> {
    let (kind, inner_start) = if text.starts_with("{{") {
        (Kind::Total, 2)
    } else {
        (Kind::PerUnit, 1)
    };
    #[expect(
        clippy::string_slice,
        reason = "inner_start is 1 or 2, both valid char boundaries"
    )]
    let inner = &text[inner_start..];
    let close = find_unquoted(inner, '}').ok_or_else(|| "cost block is not closed".to_owned())?;
    #[expect(
        clippy::string_slice,
        reason = "close is a char boundary from char_indices()"
    )]
    let body = &inner[..close];
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "close + 1 cannot overflow; close < len(inner)"
    )]
    #[expect(
        clippy::string_slice,
        reason = "close + 1 is a valid char boundary from char_indices()"
    )]
    let mut after = &inner[close + 1..];
    if kind == Kind::Total {
        after = after
            .strip_prefix('}')
            .ok_or_else(|| "cost block is not closed".to_owned())?;
    }

    let (basis_opt, date, label) = read_components(body)?;
    let basis = basis_opt.ok_or_else(|| LOT_SELECTION.to_owned())?;
    if basis.value.is_sign_negative() && !basis.value.is_zero() {
        return Err("negative cost not allowed".into());
    }
    Ok((
        CostBlock {
            kind,
            basis,
            date,
            label,
        },
        after,
    ))
}

/// Reads a block body's components, classified by shape: at most one
/// amount with its code, one date and one label, in any order.
#[expect(clippy::type_complexity, reason = "three optional parts of one block")]
fn read_components(
    body: &str,
) -> Result<(Option<Figure>, Option<jiff::civil::Date>, Option<String>), String> {
    let mut basis_opt: Option<Figure> = None;
    let mut date: Option<jiff::civil::Date> = None;
    let mut label: Option<String> = None;
    let mut components = split_components(body).into_iter();
    while let Some(component) = components.next() {
        if component == "*" {
            return Err(LOT_SELECTION.into());
        }
        if let Some(unquoted) = component
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
        {
            if unquoted.contains('"') {
                return Err(format!("bad cost component '{component}'"));
            }
            if label.is_some() {
                return Err("cost block has two labels".into());
            }
            label = Some(unquoted.to_owned());
        } else if component.contains('"') {
            return Err(format!("bad cost component '{component}'"));
        } else if looks_like_date(&component) {
            if component.len() != 10 {
                return Err(format!("bad cost date '{component}'"));
            }
            let parsed = jiff::civil::Date::from_str(&component)
                .map_err(|_err| format!("bad cost date '{component}'"))?;
            if date.is_some() {
                return Err("cost block has two dates".into());
            }
            date = Some(parsed);
        } else if let Ok(value) = Decimal::from_str(&component) {
            if basis_opt.is_some() {
                return Err("cost block has two amounts".into());
            }
            let code = components
                .next()
                .filter(|c| !c.is_empty())
                .ok_or_else(|| format!("cost amount '{component}' has no commodity"))?;
            if code.starts_with('"')
                || code == "*"
                || Decimal::from_str(&code).is_ok()
                || jiff::civil::Date::from_str(&code).is_ok()
            {
                return Err(format!("cost amount '{component}' has no commodity"));
            }
            basis_opt = Some(Figure { value, code });
        } else {
            // A bare label never holds whitespace: `105 AUD` is an amount
            // written the Beancount way, and the fix is the colon form.
            if component.contains(char::is_whitespace) {
                return Err(format!(
                    "bad cost component '{component}': expected AMOUNT:CCY, a YYYY-MM-DD date or a label"
                ));
            }
            if label.is_some() {
                return Err("cost block has two labels".into());
            }
            label = Some(component);
        }
    }

    Ok((basis_opt, date, label))
}

/// Whether a bare cost component opens like an ISO date: four ASCII digits
/// then `-`. Used to tell a malformed date apart from a label that merely
/// starts with digits.
fn looks_like_date(component: &str) -> bool {
    let bytes = component.as_bytes();
    bytes.get(4) == Some(&b'-')
        && bytes
            .get(..4)
            .is_some_and(|d| d.iter().all(u8::is_ascii_digit))
}

/// Byte offset of the first `needle` outside `"…"`, or `None` when it is
/// absent or a quote is left open.
fn find_unquoted(text: &str, needle: char) -> Option<usize> {
    let mut in_quotes = false;
    for (idx, c) in text.char_indices() {
        if c == '"' {
            in_quotes = !in_quotes;
        } else if c == needle && !in_quotes {
            return Some(idx);
        }
    }
    None
}

/// Splits a block body on `,` and `:` outside `"…"`, trimming each piece
/// and dropping empty ones.
fn split_components(body: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    for c in body.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                current.push(c);
            }
            ',' | ':' if !in_quotes => {
                parts.push(core::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|p| p.trim().to_owned())
        .filter(|p| !p.is_empty())
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::CostBlock;
    use super::Figure;
    use super::Kind;
    use super::Leg;
    use super::Price;
    use super::parse_leg;
    use super::render_leg;

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
    #[case::stray_colon_after_units("5:AUD:", "unexpected ':' after the amount")]
    #[case::empty_code_double_colon("5::AUD", "expected AMOUNT:COMMODITY")]
    #[case::code_swallows_colons("-2:AAPL:105:AUD", "unexpected ':105:AUD' after the amount")]
    #[case::code_swallows_quote("5:AUD\"x\"", "unexpected '\"x\"' after the amount")]
    fn errors_name_the_problem(#[case] text: &str, #[case] expected: &str) {
        let err = parse_leg(text).expect_err("rejects");
        assert!(err.contains(expected), "got: {err}");
    }

    #[rstest]
    #[case::per_unit_colon("2:AAPL{105:AUD}", Kind::PerUnit, None, None)]
    #[case::total("2:AAPL{{210:AUD}}", Kind::Total, None, None)]
    #[case::colon_date_label(
        "2:AAPL{105:AUD:2024-03-01:lot-a}",
        Kind::PerUnit,
        Some(date(2024, 3, 1)),
        Some("lot-a")
    )]
    #[case::comma_date_label(
        "2:AAPL{105:AUD,2024-03-01,lot-a}",
        Kind::PerUnit,
        Some(date(2024, 3, 1)),
        Some("lot-a")
    )]
    #[case::mixed_any_order(
        "2:AAPL{lot-a,105:AUD:2024-03-01}",
        Kind::PerUnit,
        Some(date(2024, 3, 1)),
        Some("lot-a")
    )]
    #[case::label_only("2:AAPL{105:AUD:lot-a}", Kind::PerUnit, None, Some("lot-a"))]
    #[case::spaces(
        "2:AAPL{ 105:AUD , 2024-03-01 , lot-a }",
        Kind::PerUnit,
        Some(date(2024, 3, 1)),
        Some("lot-a")
    )]
    #[case::quoted_label("2:AAPL{105:AUD,\"lot-a\"}", Kind::PerUnit, None, Some("lot-a"))]
    #[case::quoted_label_with_comma("2:AAPL{105:AUD,\"a, b\"}", Kind::PerUnit, None, Some("a, b"))]
    #[case::quoted_label_with_colon("2:AAPL{105:AUD,\"x:y\"}", Kind::PerUnit, None, Some("x:y"))]
    #[case::quoted_label_with_brace(
        "2:AAPL{105:AUD,\"lot}1\"}",
        Kind::PerUnit,
        None,
        Some("lot}1")
    )]
    #[case::trailing_separator("2:AAPL{105:AUD,}", Kind::PerUnit, None, None)]
    #[case::quoted_label_looks_like_bad_date(
        "2:AAPL{105:AUD,\"2024-13-01\"}",
        Kind::PerUnit,
        None,
        Some("2024-13-01")
    )]
    fn cost_forms_parse(
        #[case] text: &str,
        #[case] kind: Kind,
        #[case] lot_date: Option<jiff::civil::Date>,
        #[case] label: Option<&str>,
    ) {
        let leg = parse_leg(text).expect("parses");
        let value = if kind == Kind::Total {
            dec!(210)
        } else {
            dec!(105)
        };
        assert_eq!(
            leg.cost,
            Some(CostBlock {
                kind,
                basis: fig(value, "AUD"),
                date: lot_date,
                label: label.map(ToOwned::to_owned),
            })
        );
    }

    #[test]
    fn cost_and_price_together() {
        let leg = parse_leg("-2:AAPL{105:AUD:2024-03-01:lot-a}@150:AUD").expect("parses");
        assert_eq!(leg.units, fig(dec!(-2), "AAPL"));
        assert_eq!(leg.cost.as_ref().map(|c| c.basis.value), Some(dec!(105)));
        assert_eq!(leg.price.as_ref().map(|p| p.figure.value), Some(dec!(150)));
    }

    #[rstest]
    #[case::empty(
        "2:AAPL{}",
        "lot selection needs inventory booking, not supported yet (#518)"
    )]
    #[case::date_only(
        "2:AAPL{2024-03-01}",
        "lot selection needs inventory booking, not supported yet (#518)"
    )]
    #[case::label_only_no_amount(
        "2:AAPL{lot-a}",
        "lot selection needs inventory booking, not supported yet (#518)"
    )]
    #[case::star(
        "2:AAPL{105:AUD,*}",
        "lot selection needs inventory booking, not supported yet (#518)"
    )]
    #[case::negative("2:AAPL{-105:AUD}", "negative cost not allowed")]
    #[case::two_amounts("2:AAPL{105:AUD,106:AUD}", "cost block has two amounts")]
    #[case::two_dates("2:AAPL{105:AUD,2024-03-01,2024-03-02}", "cost block has two dates")]
    #[case::two_labels("2:AAPL{105:AUD,lot-a,lot-b}", "cost block has two labels")]
    #[case::amount_without_code("2:AAPL{105}", "cost amount '105' has no commodity")]
    #[case::unclosed_single("2:AAPL{105:AUD", "cost block is not closed")]
    #[case::unclosed_double("2:AAPL{{210:AUD}", "cost block is not closed")]
    #[case::unclosed_quote("2:AAPL{105:AUD,\"lot}", "cost block is not closed")]
    #[case::trailing_after_block(
        "2:AAPL{105:AUD} extra",
        "unexpected 'extra' after the cost block"
    )]
    #[case::cost_code_is_date("2:AAPL{105,2024-03-01,lot-a}", "cost amount '105' has no commodity")]
    #[case::cost_code_is_star("2:AAPL{105:*}", "cost amount '105' has no commodity")]
    #[case::cost_code_is_quoted("2:AAPL{105:\"AUD\"}", "cost amount '105' has no commodity")]
    #[case::cost_code_is_number("2:AAPL{105:106:AUD}", "cost amount '105' has no commodity")]
    #[case::bad_date_month("2:AAPL{105:AUD,2024-13-01}", "bad cost date '2024-13-01'")]
    #[case::bad_date_short("2:AAPL{105:AUD,2024-3-1}", "bad cost date '2024-3-1'")]
    #[case::bad_date_datetime("2:AAPL{105:AUD,2024-03-01T00}", "bad cost date '2024-03-01T00'")]
    #[case::unmatched_quote_suffix("2:AAPL{105:AUD,\"a\"b}", "bad cost component '\"a\"b'")]
    #[case::doubled_quotes("2:AAPL{105:AUD,\"a\"\"b\"}", "bad cost component '\"a\"\"b\"'")]
    #[case::space_separated_amount("2:AAPL{105 AUD}", "bad cost component '105 AUD'")]
    #[case::space_separated_amount_with_date(
        "2:AAPL{105 AUD, 2024-03-01}",
        "bad cost component '105 AUD'"
    )]
    fn cost_errors_name_the_problem(#[case] text: &str, #[case] expected: &str) {
        let err = parse_leg(text).expect_err("rejects");
        assert!(err.contains(expected), "got: {err}");
    }

    fn posting(
        units: (rust_decimal::Decimal, &str),
        cost: Option<bc_models::Cost>,
        price: Option<bc_models::Quote>,
    ) -> bc_models::Posting {
        bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(bc_models::AccountId::new())
            .amount(bc_models::Amount::new(
                units.0,
                bc_models::CommodityCode::new(units.1),
            ))
            .maybe_cost(cost)
            .maybe_price(price)
            .build()
    }

    fn aud(value: rust_decimal::Decimal) -> bc_models::Amount {
        bc_models::Amount::new(value, bc_models::CommodityCode::new("AUD"))
    }

    #[test]
    fn render_bare_amount() {
        assert_eq!(
            render_leg(&posting((dec!(50.00), "AUD"), None, None)).as_deref(),
            Some("50.00 AUD")
        );
    }

    #[test]
    fn render_total_price() {
        let p = posting(
            (dec!(4.00), "USD"),
            None,
            Some(bc_models::Quote::Total(aud(dec!(6.37)))),
        );
        assert_eq!(render_leg(&p).as_deref(), Some("4.00 USD @@ 6.37 AUD"));
    }

    #[test]
    fn render_cost_with_date_label_and_price() {
        let cost = bc_models::Cost::builder()
            .basis(bc_models::Quote::PerUnit(aud(dec!(105))))
            .date(date(2024, 3, 1))
            .label("lot-a")
            .build();
        let p = posting(
            (dec!(2), "AAPL"),
            Some(cost),
            Some(bc_models::Quote::PerUnit(aud(dec!(150)))),
        );
        assert_eq!(
            render_leg(&p).as_deref(),
            Some("2 AAPL {105 AUD, 2024-03-01, \"lot-a\"} @ 150 AUD")
        );
    }

    #[test]
    fn render_total_cost() {
        let cost = bc_models::Cost::builder()
            .basis(bc_models::Quote::Total(aud(dec!(210))))
            .build();
        let p = posting((dec!(2), "AAPL"), Some(cost), None);
        assert_eq!(render_leg(&p).as_deref(), Some("2 AAPL {{210 AUD}}"));
    }

    #[test]
    fn render_elided_is_none() {
        let p = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(bc_models::AccountId::new())
            .build();
        assert_eq!(render_leg(&p), None);
    }
}
