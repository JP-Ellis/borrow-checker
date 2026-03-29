//! Renders domain data as Ledger plain-text.

use bc_models::Amount;
use bc_models::TransactionStatus;
use jiff::civil::Date;

/// Renders a single transaction to Ledger syntax.
///
/// # Arguments
///
/// * `date` - The transaction date.
/// * `status` - The transaction status.
/// * `payee` - The payee / description for the header line.
/// * `comment` - Optional inline comment appended to the header line.
/// * `postings` - Slice of `(account_path, amount)` pairs.
///
/// # Returns
///
/// A `String` with Ledger-formatted output, or an empty string for
/// [`TransactionStatus::Voided`] transactions (which are omitted).
#[expect(
    clippy::format_push_string,
    reason = "write! to String requires propagating an infallible Result; push_str+format! is cleaner"
)]
pub(crate) fn render_transaction(
    date: Date,
    status: TransactionStatus,
    payee: &str,
    comment: Option<&str>,
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
        TransactionStatus::Cleared => " * ",
        TransactionStatus::Pending => " ! ",
        TransactionStatus::Voided => return String::new(),
        _ => return String::new(),
    };

    let comment_suffix = comment.map(|c| format!("  ; {c}")).unwrap_or_default();
    let mut out = format!("{date}{flag}{payee}{comment_suffix}\n");

    for (account, amount) in postings {
        let val = amount.value();
        let commodity = amount.commodity().as_str();
        if val.is_sign_negative() {
            let abs_val = val.abs();
            out.push_str(&format!("    {account}    -{abs_val} {commodity}\n"));
        } else {
            out.push_str(&format!("    {account}    {val} {commodity}\n"));
        }
    }

    out
}

/// Renders an `account` declaration line.
///
/// # Arguments
///
/// * `path` - The full account path (e.g. `"Assets:Bank"`).
///
/// # Returns
///
/// A `String` containing the `account` directive.
pub(crate) fn render_account_decl(path: &str) -> String {
    format!("account {path}\n")
}

/// Renders a `commodity` declaration line.
///
/// # Arguments
///
/// * `code` - The commodity code (e.g. `"AUD"`).
///
/// # Returns
///
/// A `String` containing the `commodity` directive.
pub(crate) fn render_commodity_decl(code: &str) -> String {
    format!("commodity {code}\n")
}

#[cfg(test)]
mod tests {
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::TransactionStatus;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn renders_cleared_transaction() {
        let output = render_transaction(
            date(2025, 1, 15),
            TransactionStatus::Cleared,
            "Woolworths",
            None,
            &[
                (
                    "Expenses:Food",
                    Amount::new(dec!(50.00), CommodityCode::new("AUD")),
                ),
                (
                    "Assets:Bank",
                    Amount::new(dec!(-50.00), CommodityCode::new("AUD")),
                ),
            ],
        );
        assert_eq!(
            output,
            "2025-01-15 * Woolworths\n    Expenses:Food    50.00 AUD\n    Assets:Bank    -50.00 AUD\n"
        );
    }

    #[test]
    fn renders_pending_transaction() {
        let output = render_transaction(
            date(2025, 1, 15),
            TransactionStatus::Pending,
            "Test",
            None,
            &[
                ("X", Amount::new(dec!(1.00), CommodityCode::new("AUD"))),
                ("Y", Amount::new(dec!(-1.00), CommodityCode::new("AUD"))),
            ],
        );
        assert!(output.contains("! Test"));
    }

    #[test]
    fn renders_voided_transaction_returns_empty() {
        let output = render_transaction(
            date(2025, 1, 15),
            TransactionStatus::Voided,
            "Test",
            None,
            &[],
        );
        assert_eq!(output, "");
    }
}
