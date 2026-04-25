//! Beancount importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for Beancount plain-text accounting files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for BeancountImporter` block
//! to generate the required WASM export glue.

mod ast;
mod parser;

use bc_sdk::{Amount, ImportConfig, ImportError, RawTransaction};
use rust_decimal::Decimal;

use crate::ast::Directive;
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
    /// A list of [`RawTransaction`] values parsed from transaction directives.
    /// For multi-commodity transactions, only the first posting's commodity and
    /// amount are used; additional postings are ignored.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::Parse`] if the file is not valid UTF-8 or if a
    /// parse error is encountered.
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

            let first = tx
                .postings
                .first()
                .ok_or_else(|| ImportError::Parse("transaction has no postings".into()))?;

            // When multiple commodities are present, only the first posting's
            // commodity is used for `RawTransaction::amount`; the rest are dropped.
            // This is a known limitation of the single-commodity `RawTransaction` model.

            let amount = decimal_to_amount(first.amount, &first.currency)?;

            raw_txs.push(RawTransaction::new(
                tx.date,
                amount,
                None,
                tx.payee,
                tx.narration,
                None,
            ));
        }

        Ok(raw_txs)
    }
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
    use bc_sdk::Importer as _;
    use bc_sdk::{Amount, Date, ImportConfig};
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn imports_transaction_payee_and_narration() {
        let input = "2025-01-15 * \"Woolworths\" \"Weekly groceries\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank    -50.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 1);
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee.as_deref(), Some("Woolworths"));
        assert_eq!(tx.description, "Weekly groceries");
        assert_eq!(tx.date, Date::new(2025, 1, 15));
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
    fn import_multi_currency_transaction_uses_first_posting() {
        // A transaction with mixed currencies: the importer should succeed
        // and use the first posting's amount.
        let input =
            "2025-01-15 * \"FX Purchase\"\n  Assets:USD   100.00 USD\n  Assets:AUD  -150.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), ImportConfig::default())
            .expect("import should succeed even for multi-currency");
        let tx = txs.first().expect("should have one transaction");
        // First posting determines the amount
        assert_eq!(tx.description, "FX Purchase");
        assert_eq!(tx.amount, Amount::new(10000, "USD", 2));
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
        assert_eq!(decimal_to_amount(dec!(50.00), "AUD").expect("no overflow"), Amount::new(5000, "AUD", 2));
        assert_eq!(
            decimal_to_amount(dec!(-1234.56), "AUD").expect("no overflow"),
            Amount::new(-123_456, "AUD", 2)
        );
        assert_eq!(decimal_to_amount(dec!(0.00), "AUD").expect("no overflow"), Amount::new(0, "AUD", 2));
        assert_eq!(decimal_to_amount(dec!(1.0), "AUD").expect("no overflow"), Amount::new(10, "AUD", 1));
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
