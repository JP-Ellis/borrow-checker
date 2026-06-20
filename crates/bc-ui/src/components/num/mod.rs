//! Monetary value formatting and component.

use core::cmp::Ordering;

use bc_ipc::Amount;
pub use bc_ipc::Currency;
pub use bc_ipc::USD;
pub use bc_ipc::currency_from_code;
use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "num.module.scss");

// MARK: Formatting

/// Converts `abs` minor units to a plain decimal string with no grouping.
///
/// E.g. `to_decimal_string(123456, 2)` → `"1234.56"`,
///      `to_decimal_string(9100, 0)` → `"9100"`.
///
/// Used as a fallback when `Intl.NumberFormat` is unavailable.
#[must_use]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    clippy::expect_used,
    reason = "division by 10^decimals (non-zero); overflow guarded by debug_assert and checked_pow"
)]
pub fn to_decimal_string(abs: u64, decimals: u8) -> String {
    if decimals == 0 {
        return abs.to_string();
    }
    debug_assert!(decimals <= 19, "decimals > 19 overflows u64 scale");
    let scale = 10_u64
        .checked_pow(u32::from(decimals))
        .expect("decimals ≤ 19; guarded by debug_assert above");
    let integer = abs / scale;
    let frac = abs % scale;
    format!("{integer}.{frac:0>width$}", width = usize::from(decimals))
}

/// Formats `abs` minor units with locale-aware grouping and currency symbol,
/// using the browser's `Intl.NumberFormat` API.
///
/// `abs` is unsigned; the sign prefix is added by [`format_amount`].
#[cfg(target_arch = "wasm32")]
#[expect(
    clippy::cast_precision_loss,
    reason = "monetary display: f64 precision is sufficient for all supported currency amounts"
)]
fn format_with_symbol(abs: u64, currency: &Currency) -> String {
    use js_sys::Array;
    use js_sys::Intl::NumberFormat;
    use js_sys::Object;
    use js_sys::Reflect;
    use web_sys::wasm_bindgen::JsValue;

    let scale = 10_f64.powi(i32::from(currency.decimals));
    #[expect(
        clippy::as_conversions,
        clippy::float_arithmetic,
        reason = "monetary display: cast to f64 then divide by scale for Intl.NumberFormat; precision loss is acceptable"
    )]
    let value = JsValue::from_f64(abs as f64 / scale);
    let d = JsValue::from_f64(f64::from(currency.decimals));

    let options = Object::new();
    if currency.is_iso {
        drop(Reflect::set(
            &options,
            &JsValue::from_str("style"),
            &JsValue::from_str("currency"),
        ));
        drop(Reflect::set(
            &options,
            &JsValue::from_str("currency"),
            &JsValue::from_str(currency.code),
        ));
    } else {
        drop(Reflect::set(
            &options,
            &JsValue::from_str("style"),
            &JsValue::from_str("decimal"),
        ));
    }
    drop(Reflect::set(
        &options,
        &JsValue::from_str("minimumFractionDigits"),
        &d,
    ));
    drop(Reflect::set(
        &options,
        &JsValue::from_str("maximumFractionDigits"),
        &d,
    ));

    let fmt = NumberFormat::new(&Array::new(), &options);
    let output = fmt
        .format()
        .call1(&JsValue::NULL, &value)
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| {
            let decimal = to_decimal_string(abs, currency.decimals);
            if currency.symbol_after {
                format!("{decimal}\u{00a0}{}", currency.symbol)
            } else {
                format!("{}{decimal}", currency.symbol)
            }
        });

    if currency.is_iso {
        output
    } else if currency.symbol_after {
        format!("{output}\u{00a0}{}", currency.symbol)
    } else {
        format!("{}{output}", currency.symbol)
    }
}

/// Formats `minor_units` according to `currency`.
///
/// Sign convention:
/// - Positive: `+` prefix (e.g. `+$1,234.56`)
/// - Negative: `−` prefix using U+2212 MINUS SIGN (e.g. `−$1,234.56`)
/// - Zero: no sign (e.g. `$0.00`)
///
/// Grouping separators, decimal separators, and symbol placement are
/// determined by the browser locale via `Intl.NumberFormat`.
#[must_use]
#[inline]
pub fn format_amount(minor_units: i64, currency: &Currency) -> String {
    let abs = minor_units.unsigned_abs();
    let formatted = format_with_symbol(abs, currency);

    match minor_units.cmp(&0) {
        Ordering::Greater => format!("+{formatted}"),
        Ordering::Less => format!("\u{2212}{formatted}"),
        Ordering::Equal => formatted,
    }
}

// MARK: Parsing

/// Parses a target input string into minor units at scale 2.
///
/// Returns `None` for empty input, bare `.`, negative values, or non-numeric
/// input. Supports both integer (`"250"`) and decimal (`"2.50"`) forms.
/// Multiple dots → `None`. At least one digit must be present.
///
/// Fractional digits beyond 2 are rounded half-up (round half away from zero)
/// at the 2nd decimal place, so `"1.999"` → `200` and `"1.994"` → `199`.
/// Overflow (result too large for `i64`) returns `None`.
#[must_use]
pub(crate) fn parse_target_minor(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() || s.starts_with('-') {
        return None;
    }

    // Reject multiple dots.
    let dot_count = s.chars().filter(|&c| c == '.').count();
    if dot_count > 1 {
        return None;
    }

    match s.split_once('.') {
        None => {
            // Integer form: all chars must be digits.
            if !s.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let whole: i64 = s.parse().ok()?;
            whole.checked_mul(100)
        }
        Some((whole, frac)) => {
            // Both sides must be all-digit (each may be empty, but together
            // at least one digit must exist).
            if !whole.chars().all(|c| c.is_ascii_digit())
                || !frac.chars().all(|c| c.is_ascii_digit())
            {
                return None;
            }
            // Require at least one digit somewhere.
            if whole.is_empty() && frac.is_empty() {
                return None;
            }

            let whole: i64 = if whole.is_empty() {
                0
            } else {
                whole.parse().ok()?
            };

            // Build a fixed-point minor value with rounding.
            // We need the first 2 frac digits plus whether any digit >= 5
            // exists at position 3 (the third decimal place).
            #[expect(
                clippy::string_slice,
                reason = "frac is validated as all-ASCII-digit by the guard above; byte-index slicing is safe"
            )]
            #[expect(
                clippy::indexing_slicing,
                reason = "frac.len() >= 3 is guaranteed by the match arm above"
            )]
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "d, prefix are small digit values; saturating ops not needed for this fixed-scale decimal"
            )]
            let cents: i64 = match frac.len() {
                0 => 0,
                1 => {
                    /* e.g. "5" → 50 */
                    let d: i64 = frac.parse().ok()?;
                    d * 10
                }
                2 => frac.parse().ok()?,
                _ => {
                    /* Round the 2-digit prefix based on the third digit. */
                    let prefix: i64 = frac[..2].parse().ok()?;
                    let third: u8 = frac.as_bytes()[2] - b'0';
                    if third >= 5 { prefix + 1 } else { prefix }
                }
            };

            whole.checked_mul(100)?.checked_add(cents)
        }
    }
}

// MARK: Component

/// Renders a monetary amount as a formatted, coloured string.
///
/// Positive values are coloured `good`, negative `bad`, zero neutral `ink`.
/// Uses Fira Code with tabular figures and U+2212 for the minus sign.
/// Grouping and symbol placement follow the browser locale via `Intl.NumberFormat`.
///
/// # Arguments
///
/// * `money` - Amount and currency code. Currency code must be in the
///   stop-gap registry; unknown codes fall back to USD formatting with a
///   warning in debug builds.
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos component props require owned types"
)]
pub fn Num(
    /// Monetary value (amount + currency code).
    money: Amount,
) -> impl IntoView {
    let currency = currency_from_code(&money.currency_code).unwrap_or_else(|| {
        #[cfg(debug_assertions)]
        leptos::logging::warn!(
            "Num: unknown currency code {:?}, falling back to USD",
            money.currency_code
        );
        &USD
    });

    let tone = match money.minor_units.cmp(&0) {
        Ordering::Greater => style::positive,
        Ordering::Less => style::negative,
        Ordering::Equal => style::neutral,
    };
    let class = format!("{} {}", style::num, tone);

    view! { <span class=class>{format_amount(money.minor_units, currency)}</span> }
}

#[cfg(debug_assertions)]
pub mod qa;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::to_decimal_string;

    #[test]
    fn two_decimal_places() {
        assert_eq!(to_decimal_string(128_456, 2), "1284.56");
    }

    #[test]
    fn zero_decimal_places() {
        assert_eq!(to_decimal_string(9_100, 0), "9100");
    }

    #[test]
    fn leading_zeros_in_fraction() {
        assert_eq!(to_decimal_string(1, 2), "0.01");
    }

    #[test]
    fn eight_decimal_places_btc() {
        assert_eq!(to_decimal_string(1_23456789, 8), "1.23456789");
    }

    #[test]
    fn one_satoshi() {
        assert_eq!(to_decimal_string(1, 8), "0.00000001");
    }

    #[test]
    fn zero_value() {
        assert_eq!(to_decimal_string(0, 2), "0.00");
    }

    #[test]
    fn zero_value_no_decimals() {
        assert_eq!(to_decimal_string(0, 0), "0");
    }
}
