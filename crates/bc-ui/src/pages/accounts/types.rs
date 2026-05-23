//! Display helpers and static mock data for the accounts screen.
//!
//! Domain types (`AccountNode`, `Transaction`, etc.) now live in `bc_ipc`.
//! This module retains display-layer helpers and the static stub data used
//! until IPC is wired up in a later milestone.

use std::sync::LazyLock;

use bc_ipc::AccountNode;
use bc_ipc::AccountType;
use bc_ipc::AuditEntry;
use bc_ipc::Money;
use bc_ipc::Posting;
use bc_ipc::Transaction;
use bc_ipc::TxStatus;

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
/// Returns a zero-AUD `Money` when the account has no posting.
#[must_use]
#[inline]
pub fn headline_amount(tx: &Transaction, account_id: &str) -> Money {
    tx.postings
        .iter()
        .find(|p| p.account_id == account_id)
        .map_or_else(|| Money::new(0, "AUD"), |p| p.amount.clone())
}

/// Formats an ISO-8601 date string (`"2026-04-30"`) for display.
///
/// On WASM: delegates to the browser's `Intl.DateTimeFormat`.
/// Fallback (native test builds): returns `"MM/DD"` extracted from the string.
#[must_use]
#[inline]
pub fn format_date_display(iso: &str) -> String {
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

        let ts = Date::parse(iso);
        let date = Date::new(&JsValue::from_f64(ts));
        let fmt = DateTimeFormat::new(&Array::new(), &options);
        let format_fn = fmt.format();
        format_fn
            .call1(&JsValue::NULL, &date)
            .ok()
            .and_then(|v| v.as_string())
            .unwrap_or_else(|| iso.to_owned())
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let mut parts = iso.splitn(3, '-');
        let _year = parts.next();
        match (parts.next(), parts.next()) {
            (Some(m), Some(d)) => format!("{m}/{d}"),
            _ => iso.to_owned(),
        }
    }
}

// MARK: Static mock data

/// All accounts shown in the sidebar tree.
pub static ACCOUNTS: LazyLock<Vec<AccountNode>> = LazyLock::new(|| {
    vec![
        AccountNode::new(
            "cb-smart-access",
            "Smart Access",
            Some("4421"),
            Money::new(421_842, "AUD"),
            Some("commbank"),
            AccountType::Asset,
            vec![
                "institution:commbank".to_owned(),
                "type:transactional".to_owned(),
            ],
        ),
        AccountNode::new(
            "commbank",
            "CommBank",
            None::<&str>,
            Money::new(6_421_000, "AUD"),
            None::<&str>,
            AccountType::Asset,
            vec![],
        ),
        AccountNode::new(
            "cb-netbank",
            "NetBank Saver",
            Some("8832"),
            Money::new(2_899_200, "AUD"),
            Some("commbank"),
            AccountType::Asset,
            vec!["type:savings".to_owned()],
        ),
        AccountNode::new(
            "macquarie",
            "Macquarie",
            None::<&str>,
            Money::new(14_210_000, "AUD"),
            None::<&str>,
            AccountType::Asset,
            vec![],
        ),
        AccountNode::new(
            "mac-brokerage",
            "Brokerage",
            None::<&str>,
            Money::new(12_400_000, "AUD"),
            Some("macquarie"),
            AccountType::Asset,
            vec!["type:investment".to_owned()],
        ),
        AccountNode::new(
            "amex-platinum",
            "Amex Platinum",
            Some("9001"),
            Money::new(-244_000, "AUD"),
            None::<&str>,
            AccountType::Liability,
            vec!["type:credit".to_owned()],
        ),
        AccountNode::new(
            "mortgage",
            "Mortgage — Carlton",
            None::<&str>,
            Money::new(-29_840_000, "AUD"),
            None::<&str>,
            AccountType::Liability,
            vec!["type:loan".to_owned()],
        ),
    ]
});

/// Transactions for the Smart Access account stub.
pub static TRANSACTIONS: LazyLock<Vec<Transaction>> = LazyLock::new(|| {
    vec![
        Transaction::new(
            "tx-coles-2026-04-30",
            "2026-04-30",
            "Coles Carlton",
            TxStatus::Cleared,
            vec!["shared".to_owned()],
            vec![
                Posting::new(
                    "cb-smart-access",
                    "Assets :: Smart Access",
                    Money::new(-8_420, "AUD"),
                    None::<&str>,
                ),
                Posting::new(
                    "groceries",
                    "Expenses :: Groceries",
                    Money::new(8_420, "AUD"),
                    None::<&str>,
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
            "2026-04-30",
            "Salary — Atlassian",
            TxStatus::Cleared,
            vec!["work".to_owned()],
            vec![
                Posting::new(
                    "income-salary",
                    "Income :: Salary",
                    Money::new(-846_154, "AUD"),
                    Some("gross pay"),
                ),
                Posting::new(
                    "liabilities-tax",
                    "Liabilities :: Tax Withheld",
                    Money::new(327_692, "AUD"),
                    Some("PAYG withholding"),
                ),
                Posting::new(
                    "assets-super",
                    "Assets :: Super :: Employer",
                    Money::new(90_407, "AUD"),
                    Some("11.5% SGC"),
                ),
                Posting::new(
                    "cb-smart-access",
                    "Assets :: Smart Access",
                    Money::new(428_055, "AUD"),
                    Some("take-home"),
                ),
            ],
            vec![
                AuditEntry::new("09:04", "import", "from commbank-au.wasm@1.4.2"),
                AuditEntry::new(
                    "09:04",
                    "autocat",
                    "rule \"payee=~/atlassian/i → Income::Salary\"",
                ),
                AuditEntry::new("09:04", "split", "applied rule \"salary-split-au\""),
                AuditEntry::new("09:05", "tag", "auto-tagged work via employer rule"),
            ],
        ),
        Transaction::new(
            "tx-transfer-2026-04-29",
            "2026-04-29",
            "Transfer to NetBank Saver",
            TxStatus::Cleared,
            vec![],
            vec![
                Posting::new(
                    "cb-smart-access",
                    "Assets :: Smart Access",
                    Money::new(-150_000, "AUD"),
                    None::<&str>,
                ),
                Posting::new(
                    "cb-netbank",
                    "Assets :: NetBank Saver",
                    Money::new(150_000, "AUD"),
                    None::<&str>,
                ),
            ],
            vec![AuditEntry::new(
                "11:03",
                "import",
                "from commbank-au.wasm@1.4.2",
            )],
        ),
        Transaction::new(
            "tx-brunswick-2026-04-29",
            "2026-04-29",
            "Brunswick Cycles",
            TxStatus::Cleared,
            vec!["mine".to_owned()],
            vec![
                Posting::new(
                    "amex-platinum",
                    "Liabilities :: Amex Platinum",
                    Money::new(-32_900, "AUD"),
                    None::<&str>,
                ),
                Posting::new(
                    "hobbies-cycling",
                    "Expenses :: Hobbies :: Cycling",
                    Money::new(32_900, "AUD"),
                    None::<&str>,
                ),
            ],
            vec![
                AuditEntry::new("08:55", "import", "from amex-au.wasm@2.1.0"),
                AuditEntry::new(
                    "08:55",
                    "autocat",
                    "rule \"payee=~/brunswick cycles/i → Hobbies::Cycling\"",
                ),
            ],
        ),
        Transaction::new(
            "tx-origin-2026-04-28",
            "2026-04-28",
            "Origin Energy",
            TxStatus::Cleared,
            vec!["shared".to_owned()],
            vec![
                Posting::new(
                    "cb-smart-access",
                    "Assets :: Smart Access",
                    Money::new(-17_840, "AUD"),
                    None::<&str>,
                ),
                Posting::new(
                    "utilities-electricity",
                    "Expenses :: Utilities :: Electricity",
                    Money::new(17_840, "AUD"),
                    None::<&str>,
                ),
            ],
            vec![
                AuditEntry::new("07:31", "import", "from commbank-au.wasm@1.4.2"),
                AuditEntry::new(
                    "07:31",
                    "autocat",
                    "rule \"payee=~/origin energy/i → Utilities::Electricity\"",
                ),
            ],
        ),
    ]
});

// MARK: Tests

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::TRANSACTIONS;
    use super::format_date_display;
    use super::headline_amount;
    use super::payee_initial;

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
        let tx = &TRANSACTIONS[0]; // Coles Carlton, Smart Access = -8_420 AUD
        let amount = headline_amount(tx, "cb-smart-access");
        assert_eq!(amount.minor_units, -8_420);
        assert_eq!(amount.currency_code, "AUD");
    }

    #[test]
    fn headline_amount_multi_posting_salary() {
        let tx = &TRANSACTIONS[1]; // salary, take-home to cb-smart-access = 428_055 AUD
        let amount = headline_amount(tx, "cb-smart-access");
        assert_eq!(amount.minor_units, 428_055);
        assert_eq!(amount.currency_code, "AUD");
    }

    #[test]
    fn headline_amount_unknown_account_returns_zero_aud() {
        let tx = &TRANSACTIONS[0];
        let amount = headline_amount(tx, "does-not-exist");
        assert_eq!(amount.minor_units, 0);
        assert_eq!(amount.currency_code, "AUD");
    }

    #[test]
    fn format_date_display_standard() {
        assert_eq!(format_date_display("2026-04-30"), "04/30");
    }

    #[test]
    fn format_date_display_invalid_passthrough() {
        assert_eq!(format_date_display("not-a-date"), "not-a-date");
    }
}
