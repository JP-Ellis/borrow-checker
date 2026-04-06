//! Low-level rendering functions for Beancount output.

use bc_models::Amount;
use bc_models::TransactionStatus;
use jiff::civil::Date;

/// Renders a Beancount transaction to a string.
///
/// The status is mapped as follows:
/// - [`TransactionStatus::Cleared`] → `*`
/// - [`TransactionStatus::Pending`] → `!`
/// - [`TransactionStatus::Voided`] → returns an empty string (voided transactions are omitted)
///
/// # Arguments
///
/// * `date` - The transaction date.
/// * `status` - The transaction lifecycle status.
/// * `payee` - Optional payee name; included only when `Some`.
/// * `narration` - The narration or description string.
/// * `postings` - Slice of `(account_path, amount)` pairs.
///
/// # Returns
///
/// A formatted Beancount transaction string, or an empty string for voided transactions.
#[expect(
    clippy::format_push_string,
    reason = "push_str(&format!(...)) is the clearest pattern here; writeln! on String requires discarding an infallible fmt::Result which triggers other lints"
)]
pub(crate) fn render_transaction(
    date: Date,
    status: TransactionStatus,
    payee: Option<&str>,
    narration: &str,
    postings: &[(&str, Amount)],
) -> String {
    // TransactionStatus is #[non_exhaustive]: Voided is listed explicitly so any
    // newly added variants cause a compile warning here; the `_` arm handles the
    // mandatory non-exhaustive fallback.  Both arms omit the transaction.
    #[expect(
        clippy::match_same_arms,
        reason = "Voided is listed explicitly to surface future-variant review; the _ arm is the required non-exhaustive fallback"
    )]
    let flag = match status {
        TransactionStatus::Cleared => "*",
        TransactionStatus::Pending => "!",
        TransactionStatus::Voided => return String::new(),
        _ => return String::new(),
    };

    let mut out = String::new();

    // Header line
    if let Some(p) = payee {
        out.push_str(&format!("{date} {flag} \"{p}\" \"{narration}\"\n"));
    } else {
        out.push_str(&format!("{date} {flag} \"{narration}\"\n"));
    }

    // Posting lines
    for (account, amount) in postings {
        let val = amount.value();
        let commodity = amount.commodity();
        out.push_str(&format!("  {account}  {val} {commodity}\n"));
    }

    out
}

/// Renders a Beancount `open` directive to a string.
///
/// # Arguments
///
/// * `date` - The date the account was opened.
/// * `account` - The colon-separated account path.
/// * `currency` - Optional currency constraint for the account.
///
/// # Returns
///
/// A formatted `open` directive string.
#[inline]
pub(crate) fn render_open(date: Date, account: &str, currency: Option<&str>) -> String {
    if let Some(c) = currency {
        format!("{date} open {account} {c}\n")
    } else {
        format!("{date} open {account}\n")
    }
}

/// Renders a Beancount `commodity` directive to a string.
///
/// # Arguments
///
/// * `date` - The date from which the commodity is valid.
/// * `code` - The commodity code (e.g. `"AUD"`).
///
/// # Returns
///
/// A formatted `commodity` directive string.
#[inline]
pub(crate) fn render_commodity(date: Date, code: &str) -> String {
    format!("{date} commodity {code}\n")
}

#[cfg(test)]
mod tests {
    use bc_models::CommodityCode;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn render_transaction_cleared_with_payee() {
        let postings = [
            (
                "Expenses:Food",
                Amount::new(dec!(50.00), CommodityCode::new("AUD")),
            ),
            (
                "Assets:Bank",
                Amount::new(dec!(-50.00), CommodityCode::new("AUD")),
            ),
        ];
        let result = render_transaction(
            date(2025, 1, 15),
            TransactionStatus::Cleared,
            Some("Woolworths"),
            "Weekly groceries",
            &postings
                .iter()
                .map(|(a, amt)| (*a, amt.clone()))
                .collect::<Vec<_>>(),
        );
        assert!(result.starts_with("2025-01-15 * \"Woolworths\" \"Weekly groceries\""));
        assert!(result.contains("  Expenses:Food  50.00 AUD"));
        assert!(result.contains("  Assets:Bank  -50.00 AUD"));
    }

    #[test]
    fn render_transaction_pending_no_payee() {
        let postings = [("A:B", Amount::new(dec!(1.00), CommodityCode::new("AUD")))];
        let result = render_transaction(
            date(2025, 1, 1),
            TransactionStatus::Pending,
            None,
            "Pending payment",
            &postings
                .iter()
                .map(|(a, amt)| (*a, amt.clone()))
                .collect::<Vec<_>>(),
        );
        assert!(result.starts_with("2025-01-01 ! \"Pending payment\""));
    }

    #[test]
    fn render_transaction_voided_returns_empty() {
        let result = render_transaction(
            date(2025, 1, 1),
            TransactionStatus::Voided,
            None,
            "Voided",
            &[],
        );
        assert_eq!(result, "");
    }

    #[test]
    fn render_open_with_currency() {
        let result = render_open(date(2025, 1, 1), "Assets:Bank", Some("AUD"));
        assert_eq!(result, "2025-01-01 open Assets:Bank AUD\n");
    }

    #[test]
    fn render_open_without_currency() {
        let result = render_open(date(2025, 1, 1), "Equity:Opening", None);
        assert_eq!(result, "2025-01-01 open Equity:Opening\n");
    }

    #[test]
    fn render_commodity_formats_correctly() {
        let result = render_commodity(date(2025, 1, 1), "AUD");
        assert_eq!(result, "2025-01-01 commodity AUD\n");
    }
}
