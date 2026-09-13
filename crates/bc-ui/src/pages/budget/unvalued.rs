//! Label for spend the engine could not value in the budget commodity.

use bc_ipc::Amount;

/// Builds the pill label for `unvalued`, or `None` when it is empty.
///
/// The code is spelled out after every value (`excludes 5.00 USD, 0.5 BTC`)
/// because the point of the label is which commodity the total is missing;
/// a symbol alone would not say.
pub(crate) fn unvalued_label(unvalued: &[Amount]) -> Option<String> {
    if unvalued.is_empty() {
        return None;
    }
    let parts = unvalued
        .iter()
        .map(|a| format!("{} {}", a.value, a.currency_code))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!("excludes {parts}"))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Amount;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::unvalued_label;

    #[test]
    fn empty_unvalued_has_no_label() {
        assert_eq!(unvalued_label(&[]), None);
    }

    #[test]
    fn lists_each_commodity_with_its_code() {
        let unvalued = vec![
            Amount::new(Decimal::new(500, 2), "USD"),
            Amount::new(Decimal::new(5, 1), "BTC"),
        ];
        assert_eq!(
            unvalued_label(&unvalued),
            Some("excludes 5.00 USD, 0.5 BTC".to_owned())
        );
    }
}
