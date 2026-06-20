//! Display helpers for the accounts screen.
//!
//! Domain types (`AccountNode`, `Transaction`, etc.) live in `bc_ipc`.
//! This module retains display-layer helpers only.

use bc_ipc::Amount;
use bc_ipc::Transaction;

// MARK: WASM bindings

#[cfg(target_arch = "wasm32")]
/// Bindings to JavaScript `Date.UTC()` for constructing UTC epoch milliseconds.
mod wasm_bindings {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = Date, js_name = "UTC")]
        /// Computes the epoch milliseconds for a UTC date.
        ///
        /// Wraps `Date.UTC(year, month, date)` where month is 0-based.
        pub fn utc(year: f64, month: f64, date: f64) -> f64;
    }
}

// MARK: Pure helpers

/// Returns the first ASCII letter of `payee` as uppercase, or `'?'` if none.
///
/// Used for the payee avatar circle in transaction rows.
#[must_use]
#[inline]
pub fn payee_initial(payee: &str) -> char {
    payee
        .chars()
        .find(char::is_ascii_alphabetic)
        .map_or('?', |c| c.to_ascii_uppercase())
}

/// Returns the posting for `account_id` within `tx`.
///
/// Returns a zero-AUD `Amount` when the account has no posting.
#[must_use]
#[inline]
pub fn headline_amount(tx: &Transaction, account_id: &str) -> Amount {
    tx.postings
        .iter()
        .find(|p| p.account.id == account_id)
        .map_or_else(|| Amount::new(0, "AUD", 2), |p| p.amount.clone())
}

/// Formats a [`jiff::civil::Date`] for display.
///
/// On WASM: delegates to the browser's `Intl.DateTimeFormat` using UTC timezone.
/// Having a typed `Date` means callers can also access `date.year()`,
/// `date.month()`, and `date.day()` directly to build locale-aware `Intl.*`
/// expressions without any intermediate string.
/// Fallback (native test builds): returns `"MM/DD"`.
#[must_use]
#[inline]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "month() returns 1-12; minus one is 0-11 for JS Date.UTC()"
)]
pub fn format_date_display(date: jiff::civil::Date) -> String {
    #[cfg(target_arch = "wasm32")]
    {
        use js_sys::Array;
        use js_sys::Date;
        use js_sys::Intl::DateTimeFormat;
        use js_sys::Object;
        use js_sys::Reflect;
        use web_sys::wasm_bindgen::JsValue;

        let options = Object::new();
        drop(Reflect::set(
            &options,
            &JsValue::from_str("month"),
            &JsValue::from_str("2-digit"),
        ));
        drop(Reflect::set(
            &options,
            &JsValue::from_str("day"),
            &JsValue::from_str("2-digit"),
        ));
        drop(Reflect::set(
            &options,
            &JsValue::from_str("timeZone"),
            &JsValue::from_str("UTC"),
        ));

        let ts = wasm_bindings::utc(
            f64::from(date.year()),
            f64::from(i32::from(date.month()) - 1),
            f64::from(date.day()),
        );
        let js_date = Date::new(&JsValue::from_f64(ts));
        let fmt = DateTimeFormat::new(&Array::new(), &options);
        let format_fn = fmt.format();
        format_fn
            .call1(&JsValue::NULL, &js_date)
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| date.to_string())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        format!("{:02}/{:02}", date.month(), date.day())
    }
}

// MARK: Tests

#[cfg(test)]
mod tests {
    use bc_ipc::AccountRef;
    use bc_ipc::Amount;
    use bc_ipc::AuditEntry;
    use bc_ipc::Posting;
    use bc_ipc::Transaction;
    use bc_ipc::TxStatus;
    use pretty_assertions::assert_eq;

    use super::format_date_display;
    use super::headline_amount;
    use super::payee_initial;

    fn make_test_transactions() -> Vec<Transaction> {
        vec![
            Transaction::new(
                "tx-coles-2026-04-30",
                jiff::civil::Date::constant(2026, 4, 30),
                "Coles Carlton",
                TxStatus::Cleared,
                vec!["shared".to_owned()],
                vec![
                    Posting::new(
                        "posting-coles-debit",
                        AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                        Amount::new(-8_420, "AUD", 2),
                        None::<&str>,
                        None,
                        None,
                    ),
                    Posting::new(
                        "posting-coles-groceries",
                        AccountRef::new("groceries", "Expenses :: Groceries"),
                        Amount::new(8_420, "AUD", 2),
                        None::<&str>,
                        None,
                        None,
                    ),
                ],
                vec![
                    AuditEntry::new("14:21", "import", "from commbank-au.wasm@1.4.2"),
                    AuditEntry::new(
                        "14:21",
                        "autocat",
                        "rule \"merchant=~/coles/i → Groceries\"",
                    ),
                ],
            ),
            Transaction::new(
                "tx-salary-2026-04-30",
                jiff::civil::Date::constant(2026, 4, 30),
                "Salary — Atlassian",
                TxStatus::Cleared,
                vec!["work".to_owned()],
                vec![
                    Posting::new(
                        "posting-salary-income",
                        AccountRef::new("income-salary", "Income :: Salary"),
                        Amount::new(-846_154, "AUD", 2),
                        Some("gross pay"),
                        None,
                        None,
                    ),
                    Posting::new(
                        "posting-salary-tax",
                        AccountRef::new("liabilities-tax", "Liabilities :: Tax Withheld"),
                        Amount::new(327_692, "AUD", 2),
                        Some("PAYG withholding"),
                        None,
                        None,
                    ),
                    Posting::new(
                        "posting-salary-super",
                        AccountRef::new("assets-super", "Assets :: Super :: Employer"),
                        Amount::new(90_407, "AUD", 2),
                        Some("11.5% SGC"),
                        None,
                        None,
                    ),
                    Posting::new(
                        "posting-salary-takehome",
                        AccountRef::new("cb-smart-access", "Assets :: Smart Access"),
                        Amount::new(428_055, "AUD", 2),
                        Some("take-home"),
                        None,
                        None,
                    ),
                ],
                vec![],
            ),
        ]
    }

    #[test]
    fn payee_initial_first_letter() {
        assert_eq!(payee_initial("Coles Carlton"), 'C');
    }

    #[test]
    fn payee_initial_skips_non_alpha() {
        assert_eq!(payee_initial("123 Foo"), 'F');
    }

    #[test]
    fn payee_initial_empty_returns_question_mark() {
        assert_eq!(payee_initial(""), '?');
    }

    #[test]
    fn headline_amount_finds_matching_posting() {
        let txs = make_test_transactions();
        let tx = &txs[0]; // Coles Carlton, Smart Access = -8_420 AUD
        let amount = headline_amount(tx, "cb-smart-access");
        assert_eq!(amount.minor_units, -8_420);
        assert_eq!(amount.currency_code, "AUD");
    }

    #[test]
    fn headline_amount_multi_posting_salary() {
        let txs = make_test_transactions();
        let tx = &txs[1]; // salary, take-home to cb-smart-access = 428_055 AUD
        let amount = headline_amount(tx, "cb-smart-access");
        assert_eq!(amount.minor_units, 428_055);
        assert_eq!(amount.currency_code, "AUD");
    }

    #[test]
    fn headline_amount_unknown_account_returns_zero_aud() {
        let txs = make_test_transactions();
        let tx = &txs[0];
        let amount = headline_amount(tx, "does-not-exist");
        assert_eq!(amount.minor_units, 0);
        assert_eq!(amount.currency_code, "AUD");
    }

    #[test]
    fn format_date_display_standard() {
        assert_eq!(
            format_date_display(jiff::civil::Date::constant(2026, 4, 30)),
            "04/30"
        );
    }
}
