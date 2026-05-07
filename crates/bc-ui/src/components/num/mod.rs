//! Monetary value component.
#![expect(
    clippy::mod_module_files,
    reason = "mod.rs collocates source with its SCSS module file"
)]

use core::cmp::Ordering;

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "num.module.scss");

/// Formats `i64` cents as a display string.
///
/// - Positive: `+$1,234.56`
/// - Negative: `−$1,234.56` (U+2212 MINUS SIGN, not a hyphen)
/// - Zero: `$0.00` (no sign)
#[must_use]
#[inline]
#[expect(
    clippy::arithmetic_side_effects,
    clippy::integer_division,
    clippy::integer_division_remainder_used,
    reason = "u64 / 100 and u64 % 100 with a non-zero literal constant cannot overflow or panic"
)]
pub fn format_monetary(cents: i64) -> String {
    let abs = cents.unsigned_abs();
    let dollars = abs / 100;
    let frac = abs % 100;

    let dollars_str = {
        let s = dollars.to_string();
        let mut out = String::with_capacity(s.len() + s.len() / 3);
        for (i, ch) in s.chars().rev().enumerate() {
            if i > 0 && i % 3 == 0 {
                out.push(',');
            }
            out.push(ch);
        }
        out.chars().rev().collect::<String>()
    };

    match cents.cmp(&0) {
        Ordering::Greater => format!("+${dollars_str}.{frac:02}"),
        Ordering::Less => format!("\u{2212}${dollars_str}.{frac:02}"),
        Ordering::Equal => format!("${dollars_str}.{frac:02}"),
    }
}

/// Renders a monetary amount in cents as a formatted string.
///
/// Positive values are coloured `good`, negative `bad`, zero neutral `ink`.
/// Uses Fira Code with tabular figures and U+2212 for the minus sign.
///
/// # Arguments
///
/// * `cents` - Amount in integer cents. Positive = credit; negative = debit.
#[component]
pub fn Num(
    /// Amount in integer cents. Positive = credit; negative = debit.
    cents: i64,
) -> impl IntoView {
    let tone = match cents.cmp(&0) {
        Ordering::Greater => style::positive,
        Ordering::Less => style::negative,
        Ordering::Equal => style::neutral,
    };
    let class = format!("{} {}", style::num, tone);

    view! { <span class=class>{format_monetary(cents)}</span> }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::format_monetary;

    #[test]
    fn positive_amount() {
        assert_eq!(format_monetary(128_456), "+$1,284.56");
    }

    #[test]
    fn negative_amount() {
        assert_eq!(format_monetary(-128_456), "\u{2212}$1,284.56");
    }

    #[test]
    fn zero_no_sign() {
        assert_eq!(format_monetary(0), "$0.00");
    }

    #[test]
    fn one_cent() {
        assert_eq!(format_monetary(1), "+$0.01");
    }

    #[test]
    fn minus_one_cent() {
        assert_eq!(format_monetary(-1), "\u{2212}$0.01");
    }

    #[test]
    fn thousands_separator() {
        assert_eq!(format_monetary(100_000_000), "+$1,000,000.00");
    }
}
