//! Text form of a posting leg in messages and `transaction list`, in
//! Beancount form: `2 AAPL {105 AUD, 2024-03-01, "lot-a"} @ 150 AUD`.

use core::fmt::Write as _;

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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::render_leg;

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
