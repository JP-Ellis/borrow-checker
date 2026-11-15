// Pure helpers for the cost chip: buffer ↔ [`Cost`] and chip text.
// No Leptos here so it is host-tested through the shim in `main.rs`.

use bc_ipc::Amount;
use bc_ipc::CommodityInfo;
use bc_ipc::Cost;
use bc_ipc::Quote;
use jiff::civil::Date;

use crate::components::transaction_row::editable::parse_marked_amount;

/// The cost editor's string buffers, one per input.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[expect(
    clippy::module_name_repetitions,
    reason = "CostBuffers reads naturally at call sites"
)]
pub struct CostBuffers {
    /// `true` for `{{ }}` (total), `false` for `{ }` (per unit).
    pub is_total: bool,
    /// Basis text in the editor's marker grammar (`A$105`, `AUD 105`).
    pub basis: String,
    /// Lot date as `YYYY-MM-DD`, or blank.
    pub date: String,
    /// Lot label, or blank.
    pub label: String,
}

impl CostBuffers {
    /// Seeds the buffers from a stored cost, rendering the basis as
    /// `CODE value` (the form `from_posting` uses for the amount box).
    ///
    /// # Arguments
    ///
    /// * `cost` - The stored cost, or `None` for blank buffers.
    #[must_use]
    pub fn from_cost(cost: Option<&Cost>) -> Self {
        let Some(c) = cost else {
            return Self::default();
        };
        let a = c.basis.amount();
        Self {
            is_total: c.basis.is_total(),
            basis: format!("{} {}", a.currency_code, a.value),
            date: c.date.map_or_default(|d| d.to_string()),
            label: c.label.clone().unwrap_or_default(),
        }
    }
}

/// Builds a [`Cost`] from the editor buffers.
///
/// # Arguments
///
/// * `currencies` - The set of known commodities used to resolve the marker.
/// * `is_total` - `true` for `{{ }}`.
/// * `basis` - Basis text; blank means no cost.
/// * `date` - Lot date text; blank means none.
/// * `label` - Lot label; blank means none.
///
/// # Returns
///
/// `Ok(None)` when `basis` is blank, else the cost.
///
/// # Errors
///
/// The marker/number messages of the amount box, `negative cost not
/// allowed`, or `cost date must be YYYY-MM-DD`.
#[expect(
    clippy::module_name_repetitions,
    reason = "cost_from_buffers reads naturally at call sites"
)]
pub fn cost_from_buffers(
    currencies: &[CommodityInfo],
    is_total: bool,
    basis: &str,
    date: &str,
    label: &str,
) -> Result<Option<Cost>, String> {
    if basis.trim().is_empty() {
        return Ok(None);
    }
    let (value, code) = parse_marked_amount(currencies, basis)?;
    if value.is_sign_negative() {
        return Err("negative cost not allowed".to_owned());
    }
    let amount = Amount::new(value, code);
    let resolved_basis = if is_total {
        Quote::Total(amount)
    } else {
        Quote::PerUnit(amount)
    };
    let resolved_date = match date.trim() {
        "" => None,
        d => Some(
            d.parse::<Date>()
                .map_err(|_err| "cost date must be YYYY-MM-DD".to_owned())?,
        ),
    };
    let resolved_label = match label.trim() {
        "" => None,
        l => Some(l.to_owned()),
    };
    Ok(Some(Cost::new(
        resolved_basis,
        resolved_date,
        resolved_label,
    )))
}

/// Splits a Beancount cost block out of amount-box text.
///
/// `2 AAPL {105 AUD, 2024-03-01, "lot-a"} @ 110 AUD` becomes the box text
/// `2 AAPL @ 110 AUD` and buffers holding the basis, date and label. Inside
/// the block, a quoted component is the label, a `YYYY-MM-DD` component is
/// the date, and any other is the basis, which the chip parses as usual.
/// Commas and braces inside quotes are text.
///
/// # Arguments
///
/// * `text` - The amount box's text.
///
/// # Returns
///
/// `None` when `text` holds no `{`, else the remaining box text and the
/// block's buffers.
///
/// # Errors
///
/// A block that is unclosed, empty, repeated, or holds two of one kind of
/// component, or text after a closing quote.
pub fn lift_cost_block(text: &str) -> Option<Result<(String, CostBuffers), String>> {
    let start = text.find('{')?;
    Some(split_block(text, start))
}

/// The body of [`lift_cost_block`], once a `{` is known to sit at `start`.
fn split_block(text: &str, start: usize) -> Result<(String, CostBuffers), String> {
    let (before, opened) = text.split_at(start);
    let is_total = opened.starts_with("{{");
    let (open, close) = if is_total { ("{{", "}}") } else { ("{", "}") };
    let body = opened.get(open.len()..).unwrap_or_default();

    let mut in_quote = false;
    let mut escaped = false;
    let mut close_at = None;
    for (i, c) in body.char_indices() {
        if escaped {
            escaped = false;
        } else if in_quote {
            match c {
                '\\' => escaped = true,
                '"' => in_quote = false,
                _ => {}
            }
        } else if c == '"' {
            in_quote = true;
        } else if body.get(i..).is_some_and(|rest| rest.starts_with(close)) {
            close_at = Some(i);
            break;
        }
    }
    let Some(end) = close_at else {
        return Err(format!("cost block needs a closing {close}"));
    };
    let inner = body.get(..end).unwrap_or_default();
    let after = body
        .get(end..)
        .and_then(|r| r.strip_prefix(close))
        .unwrap_or_default();
    if before.contains('{') || after.contains('{') || after.contains('}') {
        return Err("more than one cost block".to_owned());
    }

    let mut buffers = CostBuffers {
        is_total,
        ..CostBuffers::default()
    };
    for raw in split_outside_quotes(inner) {
        let part = raw.trim();
        if part.is_empty() {
            continue;
        }
        let (slot, value, kind) = if let Some(quoted) = part.strip_prefix('"') {
            let label = quoted
                .strip_suffix('"')
                .ok_or_else(|| "cost label needs a closing quote".to_owned())?;
            (&mut buffers.label, label.replace("\\\"", "\""), "labels")
        } else if part.parse::<Date>().is_ok() {
            (&mut buffers.date, part.to_owned(), "dates")
        } else {
            (&mut buffers.basis, part.to_owned(), "amounts")
        };
        if !slot.is_empty() {
            return Err(format!("cost block has two {kind}"));
        }
        *slot = value;
    }
    if buffers.basis.is_empty() {
        return Err(format!(
            "cost block needs a basis (e.g. {open}A$105{close})"
        ));
    }
    let rest = format!("{} {}", before.trim(), after.trim())
        .trim()
        .to_owned();
    Ok((rest, buffers))
}

/// Splits `inner` on the commas that sit outside double quotes.
fn split_outside_quotes(inner: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut in_quote = false;
    let mut escaped = false;
    let mut from = 0;
    for (i, c) in inner.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match c {
            '\\' if in_quote => escaped = true,
            '"' => in_quote = !in_quote,
            ',' if !in_quote => {
                parts.push(inner.get(from..i).unwrap_or_default());
                from = i.saturating_add(1);
            }
            _ => {}
        }
    }
    parts.push(inner.get(from..).unwrap_or_default());
    parts
}

/// Renders a cost chip's label in Beancount reading order.
///
/// # Arguments
///
/// * `cost` - The cost to render.
/// * `fmt` - Amount formatter (the component passes the display-aware one).
#[must_use]
#[expect(
    clippy::module_name_repetitions,
    reason = "cost_chip_text reads naturally at call sites"
)]
pub fn cost_chip_text(cost: &Cost, fmt: impl Fn(&Amount) -> String) -> String {
    let mut parts = vec![fmt(cost.basis.amount())];
    if let Some(d) = cost.date {
        parts.push(d.to_string());
    }
    if let Some(l) = &cost.label {
        parts.push(format!("\"{l}\""));
    }
    let inner = parts.join(" \u{00b7} ");
    if cost.basis.is_total() {
        format!("{{{{ {inner} }}}}")
    } else {
        format!("{{ {inner} }}")
    }
}

/// Renders a price annotation: `@ …` per unit, `@@ …` total.
///
/// # Arguments
///
/// * `quote` - The price.
/// * `fmt` - Amount formatter.
#[must_use]
pub fn quote_text(quote: &Quote, fmt: impl Fn(&Amount) -> String) -> String {
    let marker = if quote.is_total() { "@@" } else { "@" };
    format!("{marker} {}", fmt(quote.amount()))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Amount;
    use bc_ipc::CommodityInfo;
    use bc_ipc::Cost;
    use bc_ipc::Quote;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::CostBuffers;
    use super::cost_chip_text;
    use super::cost_from_buffers;
    use super::lift_cost_block;
    use super::quote_text;

    fn registry() -> Vec<CommodityInfo> {
        vec![CommodityInfo::new(
            "c2",
            "AUD",
            Some("A$".to_owned()),
            vec![],
            2,
            true,
            false,
        )]
    }

    fn aud(cents: i64) -> Amount {
        Amount::new(Decimal::new(cents, 2), "AUD")
    }

    fn plain(a: &Amount) -> String {
        format!("{} {}", a.currency_code, a.value)
    }

    #[test]
    fn blank_basis_is_no_cost() {
        assert_eq!(
            cost_from_buffers(&registry(), false, "  ", "", ""),
            Ok(None)
        );
    }

    #[test]
    fn basis_only_per_unit() {
        assert_eq!(
            cost_from_buffers(&registry(), false, "A$105", "", ""),
            Ok(Some(Cost::new(Quote::PerUnit(aud(10_500)), None, None)))
        );
    }

    #[test]
    fn total_with_date_and_label() {
        assert_eq!(
            cost_from_buffers(&registry(), true, "AUD 210.00", "2024-03-01", " lot-a "),
            Ok(Some(Cost::new(
                Quote::Total(aud(21_000)),
                Some(Date::constant(2024, 3, 1)),
                Some("lot-a".to_owned()),
            )))
        );
    }

    #[rstest]
    #[case::negative("A$-105", "", "negative cost not allowed")]
    #[case::bad_date("A$105", "2024-13-01", "cost date must be YYYY-MM-DD")]
    #[case::no_marker("105", "", "amount needs a currency (e.g. A$100)")]
    #[case::unknown_marker("X$105", "", "unknown currency 'X$'")]
    fn buffer_errors(#[case] basis: &str, #[case] date: &str, #[case] message: &str) {
        assert_eq!(
            cost_from_buffers(&registry(), false, basis, date, ""),
            Err(message.to_owned())
        );
    }

    #[rstest]
    #[case::per_unit(Cost::new(Quote::PerUnit(aud(10_500)), None, None), "{ AUD 105.00 }")]
    #[case::total(Cost::new(Quote::Total(aud(21_000)), None, None), "{{ AUD 210.00 }}")]
    #[case::dated(
        Cost::new(Quote::PerUnit(aud(10_500)), Some(Date::constant(2024, 3, 1)), None),
        "{ AUD 105.00 · 2024-03-01 }"
    )]
    #[case::full(
        Cost::new(
            Quote::PerUnit(aud(10_500)),
            Some(Date::constant(2024, 3, 1)),
            Some("lot-a".to_owned()),
        ),
        "{ AUD 105.00 · 2024-03-01 · \"lot-a\" }"
    )]
    fn chip_text(#[case] cost: Cost, #[case] expected: &str) {
        assert_eq!(cost_chip_text(&cost, plain), expected);
    }

    #[test]
    fn quote_text_marks_the_form() {
        assert_eq!(
            quote_text(&Quote::PerUnit(aud(30_000)), plain),
            "@ AUD 300.00"
        );
        assert_eq!(quote_text(&Quote::Total(aud(637)), plain), "@@ AUD 6.37");
    }

    #[test]
    fn buffers_round_trip() {
        let cost = Cost::new(
            Quote::Total(aud(21_000)),
            Some(Date::constant(2024, 3, 1)),
            Some("lot-a".to_owned()),
        );
        let b = CostBuffers::from_cost(Some(&cost));
        assert_eq!(b.is_total, true);
        assert_eq!(b.basis, "AUD 210.00");
        assert_eq!(b.date, "2024-03-01");
        assert_eq!(b.label, "lot-a");
        assert_eq!(
            cost_from_buffers(&registry(), b.is_total, &b.basis, &b.date, &b.label),
            Ok(Some(cost))
        );
        assert_eq!(CostBuffers::from_cost(None), CostBuffers::default());
    }

    fn buffers(is_total: bool, basis: &str, date: &str, label: &str) -> CostBuffers {
        CostBuffers {
            is_total,
            basis: basis.to_owned(),
            date: date.to_owned(),
            label: label.to_owned(),
        }
    }

    #[test]
    fn text_without_a_brace_lifts_nothing() {
        assert_eq!(lift_cost_block("AAPL 2 @ A$110"), None);
    }

    #[rstest]
    #[case::beancount_leg(
        r#"2 AAPL {105 AUD, 2024-03-01, "lot-a"}"#,
        "2 AAPL",
        buffers(false, "105 AUD", "2024-03-01", "lot-a")
    )]
    #[case::basis_only("AAPL 2 {A$105}", "AAPL 2", buffers(false, "A$105", "", ""))]
    #[case::total_keeps_the_price(
        "2 AAPL {{A$210}} @ A$110",
        "2 AAPL @ A$110",
        buffers(true, "A$210", "", "")
    )]
    #[case::any_order(
        r#"2 AAPL { "lot-b", 2024-03-01, A$105 }"#,
        "2 AAPL",
        buffers(false, "A$105", "2024-03-01", "lot-b")
    )]
    #[case::quoted_comma_and_brace(
        r#"2 AAPL {A$105, "a, b } \"c\""}"#,
        "2 AAPL",
        buffers(false, "A$105", "", r#"a, b } "c""#)
    )]
    fn lifts_the_block(#[case] text: &str, #[case] rest: &str, #[case] expected: CostBuffers) {
        assert_eq!(lift_cost_block(text), Some(Ok((rest.to_owned(), expected))));
    }

    #[rstest]
    #[case::unclosed("2 AAPL {A$105", "cost block needs a closing }")]
    #[case::unclosed_total("2 AAPL {{A$105}", "cost block needs a closing }}")]
    #[case::empty("2 AAPL {}", "cost block needs a basis (e.g. {A$105})")]
    #[case::two_blocks("2 AAPL {A$105} {A$106}", "more than one cost block")]
    #[case::two_dates("2 AAPL {A$105, 2024-03-01, 2024-03-02}", "cost block has two dates")]
    #[case::two_labels(r#"2 AAPL {A$105, "a", "b"}"#, "cost block has two labels")]
    #[case::open_quote(r#"2 AAPL {A$105, "lot}"#, "cost block needs a closing }")]
    fn malformed_blocks_say_why(#[case] text: &str, #[case] message: &str) {
        assert_eq!(lift_cost_block(text), Some(Err(message.to_owned())));
    }

    #[test]
    fn a_lifted_block_parses_as_a_cost() {
        let (_, b) = lift_cost_block(r#"2 AAPL {105 AUD, 2024-03-01, "lot-a"}"#)
            .expect("a block")
            .expect("lifts");
        assert_eq!(
            cost_from_buffers(&registry(), b.is_total, &b.basis, &b.date, &b.label),
            Ok(Some(Cost::new(
                Quote::PerUnit(aud(10_500)),
                Some(Date::constant(2024, 3, 1)),
                Some("lot-a".to_owned()),
            )))
        );
    }
}
