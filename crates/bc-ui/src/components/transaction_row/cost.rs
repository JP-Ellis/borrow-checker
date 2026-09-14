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
            date: c.date.map(|d| d.to_string()).unwrap_or_default(),
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
}
