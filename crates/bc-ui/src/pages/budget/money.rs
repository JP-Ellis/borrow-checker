//! Amount formatting shared by every budget page component.

use bc_ipc::Amount;
use bc_ipc::CommodityInfo;

use crate::components::num::format_unsigned_positive;
use crate::components::num::meta::DisplayMeta;
use crate::components::num::meta::display_meta_for;

/// Formats `amount` at its commodity's display precision, marking only negatives.
#[must_use]
pub(crate) fn fmt(amount: &Amount, currencies: &[CommodityInfo]) -> String {
    if amount.currency_code.is_empty() {
        return "\u{2014}".to_owned();
    }
    format_unsigned_positive(
        &amount.value,
        &display_meta_for(&amount.currency_code, currencies),
    )
}

/// Formats each of `amounts` as [`fmt`] does, joined with `separator`.
#[must_use]
pub(crate) fn fmt_list(
    amounts: &[Amount],
    separator: &str,
    currencies: &[CommodityInfo],
) -> String {
    amounts
        .iter()
        .map(|a| fmt(a, currencies))
        .collect::<Vec<_>>()
        .join(separator)
}

/// Formats `amount`'s value at its commodity's display precision, without the
/// symbol, marking only negatives.
#[must_use]
pub(crate) fn fmt_value(amount: &Amount, currencies: &[CommodityInfo]) -> String {
    let meta = DisplayMeta {
        symbol: String::new(),
        is_iso: false,
        symbol_after: false,
        ..display_meta_for(&amount.currency_code, currencies)
    };
    format_unsigned_positive(&amount.value, &meta)
}
