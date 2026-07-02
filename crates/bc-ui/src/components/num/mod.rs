//! Monetary value formatting and component.

use core::cmp::Ordering;

use bc_ipc::Amount;
use leptos::prelude::*;
use rust_decimal::Decimal;
use stylance::import_style;

use crate::components::num::meta::DisplayMeta;

/// Pure display-metadata resolution — native-testable.
pub mod meta;

import_style!(style, "num.module.scss");

// MARK: Formatting

/// Formats `value` with locale-aware grouping and currency symbol,
/// using the browser's `Intl.NumberFormat` API.
///
/// `value`'s magnitude is formatted; the sign prefix is added by
/// [`format_amount`].
#[cfg(target_arch = "wasm32")]
fn format_with_symbol(value: &Decimal, meta: &DisplayMeta) -> String {
    use js_sys::Array;
    use js_sys::Intl::NumberFormat;
    use js_sys::Object;
    use js_sys::Reflect;
    use rust_decimal::prelude::ToPrimitive as _;
    use web_sys::wasm_bindgen::JsValue;

    let abs = value.abs();
    let mut abs_scaled = abs;
    abs_scaled.rescale(u32::from(meta.decimals));
    let js_value = JsValue::from_f64(abs.to_f64().unwrap_or(0.0_f64));
    let d = JsValue::from_f64(f64::from(meta.decimals));

    let options = Object::new();
    if meta.is_iso {
        drop(Reflect::set(
            &options,
            &JsValue::from_str("style"),
            &JsValue::from_str("currency"),
        ));
        drop(Reflect::set(
            &options,
            &JsValue::from_str("currency"),
            &JsValue::from_str(&meta.code),
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
        .call1(&JsValue::NULL, &js_value)
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| {
            let decimal = abs_scaled.to_string();
            if meta.symbol_after {
                format!("{decimal}\u{00a0}{}", meta.symbol)
            } else {
                format!("{}{decimal}", meta.symbol)
            }
        });

    if meta.is_iso {
        output
    } else if meta.symbol_after {
        format!("{output}\u{00a0}{}", meta.symbol)
    } else {
        format!("{}{output}", meta.symbol)
    }
}

/// Formats `value` according to `currency`.
///
/// Sign convention:
/// - Positive: `+` prefix (e.g. `+$1,234.56`)
/// - Negative: `−` prefix using U+2212 MINUS SIGN (e.g. `−$1,234.56`)
/// - Zero: no sign (e.g. `$0.00`)
///
/// Grouping separators, decimal separators, and symbol placement are
/// determined by the browser locale via `Intl.NumberFormat`.
///
/// Fraction digits are driven from the currency's canonical `decimals` field, not the value's
/// intrinsic scale. For compact display using the value's own scale, see `Amount::format_short`.
#[must_use]
#[inline]
pub fn format_amount(value: &Decimal, meta: &DisplayMeta) -> String {
    let formatted = format_with_symbol(value, meta);
    match value.cmp(&Decimal::ZERO) {
        Ordering::Greater => format!("+{formatted}"),
        Ordering::Less => format!("\u{2212}{formatted}"),
        Ordering::Equal => formatted,
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

    let tone = match money.value.cmp(&Decimal::ZERO) {
        Ordering::Greater => style::positive,
        Ordering::Less => style::negative,
        Ordering::Equal => style::neutral,
    };
    let class = format!("{} {}", style::num, tone);

    view! { <span class=class>{format_amount(&money.value, currency)}</span> }
}

#[cfg(debug_assertions)]
pub mod qa;
