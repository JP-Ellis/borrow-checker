//! Beancount importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for Beancount plain-text accounting files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for BeancountImporter` block
//! to generate the required WASM export glue.

mod ast;
mod parser;

use bc_sdk::Amount;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use rust_decimal::Decimal;

use crate::ast::Directive;
use crate::ast::Posting;
use crate::parser::parse;

/// Implements [`bc_sdk::Importer`] for the Beancount plain-text accounting format.
///
/// Parses Beancount-formatted files and converts transaction directives into
/// [`RawTransaction`] values. Open, close, commodity, and balance directives
/// are silently ignored.
#[derive(Debug, Default)]
pub struct BeancountImporter;

#[bc_sdk::importer]
impl bc_sdk::Importer for BeancountImporter {
    /// Returns the stable identifier for this importer.
    #[inline]
    fn name(&self) -> &str {
        "beancount"
    }

    /// Returns `true` if `bytes` appear to be a Beancount file.
    ///
    /// Detection heuristic: at least one line looks like a dated transaction
    /// header (`YYYY-MM-DD * "..."` or `YYYY-MM-DD ! "..."`).
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        let Ok(text) = core::str::from_utf8(bytes) else {
            return false;
        };
        text.lines().any(|l| {
            let t = l.trim_start();
            t.len() > 12
                && t.as_bytes().get(4).copied() == Some(b'-')
                && (t.contains(" * \"") || t.contains(" ! \""))
        })
    }

    /// Parses `bytes` as a Beancount file and returns the transactions.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw Beancount file bytes.
    /// * `_config` - Unused; reserved for future configuration options.
    ///
    /// # Returns
    ///
    /// A list of [`RawTransaction`] values parsed from transaction directives,
    /// each carrying one [`RawPosting`] per source posting leg. When a
    /// transaction's legs share a single commodity and sum to zero, the final
    /// leg's amount is elided (`None`) since it is fully derivable from the
    /// others; otherwise every leg's amount is kept.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::Parse`] if the file is not valid UTF-8, if a
    /// parse error is encountered, or if a transaction directive has no
    /// postings.
    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        _config: ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError> {
        let text = core::str::from_utf8(bytes)
            .map_err(|e| ImportError::Parse(format!("file is not valid UTF-8: {e}")))?;

        let directives = parse(text).map_err(ImportError::Parse)?;
        let mut raw_txs = Vec::new();

        for directive in directives {
            let Directive::Transaction(tx) = directive else {
                continue;
            };

            if tx.postings.is_empty() {
                return Err(ImportError::Parse("transaction has no postings".into()));
            }

            let elide_last = legs_balance_to_zero(&tx.postings);
            let last_index = tx.postings.len().saturating_sub(1);

            let mut postings = Vec::with_capacity(tx.postings.len());
            for (index, posting) in tx.postings.iter().enumerate() {
                let amount = decimal_to_amount(posting.amount, &posting.currency)?;
                let elided = elide_last && index == last_index;
                postings.push(
                    RawPosting::builder()
                        .account(posting.account.clone())
                        .maybe_amount(if elided { None } else { Some(amount) })
                        .build(),
                );
            }

            raw_txs.push(
                RawTransaction::builder()
                    .date(tx.date)
                    .maybe_payee(tx.payee)
                    .description(tx.narration)
                    .postings(postings)
                    .build(),
            );
        }

        Ok(raw_txs)
    }
}

/// Returns `true` if `postings` all share one commodity and their amounts sum
/// to zero, meaning the final leg's amount is fully derivable from the rest.
///
/// # Arguments
///
/// * `postings` - The source posting legs of a transaction.
///
/// # Returns
///
/// `true` when there are at least two postings, all in the same currency,
/// whose amounts balance to zero.
#[inline]
fn legs_balance_to_zero(postings: &[Posting]) -> bool {
    let Some(first) = postings.first() else {
        return false;
    };
    if postings.len() < 2 || postings.iter().any(|p| p.currency != first.currency) {
        return false;
    }
    postings.iter().map(|p| p.amount).sum::<Decimal>() == Decimal::ZERO
}

/// Converts a [`Decimal`] value and currency string into a [`bc_sdk::Amount`].
///
/// # Arguments
///
/// * `value` - The decimal value to convert.
/// * `currency` - The ISO 4217 currency code (e.g. `"AUD"`).
///
/// # Returns
///
/// A [`bc_sdk::Amount`] with `minor_units`, `currency`, and `scale` derived
/// from the decimal's mantissa and exponent.
///
/// # Errors
///
/// Returns [`ImportError::Parse`] if the decimal value cannot be represented
/// as an `i64` minor-unit integer.
#[inline]
fn decimal_to_amount(value: Decimal, currency: impl Into<String>) -> Result<Amount, ImportError> {
    let scale = value.scale();
    // mantissa() gives the unscaled integer: 50.00 → mantissa=5000, scale=2 → minor_units=5000
    let minor_units = i64::try_from(value.mantissa())
        .map_err(|_| ImportError::Parse(format!("amount out of i64 range: {value}")))?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rust_decimal's max scale is 28, well within u8 range"
    )]
    Ok(Amount::new(minor_units, currency, scale as u8))
}

#[cfg(test)]
mod tests {
    use bc_sdk::Amount;
    use bc_sdk::Date;
    use bc_sdk::ImportConfig;
    use bc_sdk::Importer as _;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn imports_transaction_payee_and_narration() {
        let input = "2025-01-15 * \"Acme\" \"Salary\"\n  Assets:Bank:Checking   4321.00 AUD\n  Income:Salary:Acme  -4321.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 1);
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee.as_deref(), Some("Acme"));
        assert_eq!(tx.description, "Salary");
        assert_eq!(tx.date, Date::new(2025, 1, 15));
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "Assets:Bank:Checking");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(895_233, "AUD", 2)));
        assert_eq!(tx.postings[1].account, "Income:Salary:Acme");
        assert_eq!(tx.postings[1].amount, None);
    }

    #[test]
    fn imports_narration_only() {
        let input = "2025-01-15 * \"Transfer\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), ImportConfig::default())
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee, None);
        assert_eq!(tx.description, "Transfer");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "A:B");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(100, "AUD", 2)));
        assert_eq!(tx.postings[1].account, "A:C");
        assert_eq!(tx.postings[1].amount, None);
    }

    #[test]
    fn skips_open_commodity_directives() {
        let input = "2025-01-01 open Assets:Bank AUD\n2025-01-01 commodity AUD\n2025-01-15 * \"X\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 1);
    }

    #[test]
    fn detect_recognises_beancount() {
        let bytes = b"2025-01-15 * \"Payee\" \"Narration\"\n  Assets:Bank   50.00 AUD\n";
        assert!(BeancountImporter.detect(bytes));
    }

    #[test]
    fn detect_rejects_ledger() {
        let bytes = b"2025-01-15 * Payee without quotes\n    Assets:Bank    50.00 AUD\n";
        assert!(!BeancountImporter.detect(bytes));
    }

    #[test]
    fn import_multi_currency_transaction_emits_all_postings() {
        let input =
            "2025-01-15 * \"FX Purchase\"\n  Assets:USD   100.00 USD\n  Assets:AUD  -150.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), ImportConfig::default())
            .expect("import should succeed even for multi-currency");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.description, "FX Purchase");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "Assets:USD");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(10000, "USD", 2)));
        assert_eq!(tx.postings[1].account, "Assets:AUD");
        assert_eq!(tx.postings[1].amount, Some(Amount::new(-15000, "AUD", 2)));
    }

    #[test]
    fn import_transaction_with_no_postings_returns_error() {
        // A transaction directive with zero postings is invalid; the importer
        // must return an error rather than panic.
        let input = "2025-01-15 * \"Payee\" \"No postings\"\n";
        let result = BeancountImporter.import(input.as_bytes(), ImportConfig::default());
        assert!(
            result.is_err(),
            "expected error for zero-posting transaction"
        );
    }

    #[test]
    fn decimal_to_amount_round_trips_typical_bank_values() {
        use rust_decimal_macros::dec;
        assert_eq!(
            decimal_to_amount(dec!(50.00), "AUD").expect("no overflow"),
            Amount::new(5000, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(-1234.56), "AUD").expect("no overflow"),
            Amount::new(-123_456, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(0.00), "AUD").expect("no overflow"),
            Amount::new(0, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(1.0), "AUD").expect("no overflow"),
            Amount::new(10, "AUD", 1)
        );
    }

    #[test]
    fn decimal_to_amount_overflow_returns_error() {
        use rust_decimal_macros::dec;
        // A value whose mantissa exceeds i64::MAX should return an error,
        // not silently saturate to i64::MAX.
        let huge = dec!(99999999999999999999999999.99);
        assert!(
            decimal_to_amount(huge, "AUD").is_err(),
            "expected error for out-of-range mantissa"
        );
    }
}
