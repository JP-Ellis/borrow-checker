//! Beancount importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for Beancount plain-text accounting files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for BeancountImporter` block
//! to generate the required WASM export glue.

mod ast;
mod config;
mod parser;

use bc_sdk::Amount;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use bc_sdk::SourceLocation;
use rust_decimal::Decimal;

use crate::ast::Directive;
use crate::ast::PostingAmount;
use crate::config::Config;
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

    /// Parses `cfg.source_file` as a Beancount file and returns the transactions.
    ///
    /// # Arguments
    ///
    /// * `config` - Importer configuration; must supply `source_file`.
    ///
    /// # Returns
    ///
    /// A list of [`RawTransaction`] values parsed from transaction directives,
    /// each carrying one [`RawPosting`] per source posting leg. A leg keeps
    /// its explicit amount when the source specifies one; a leg the source
    /// leaves elided (Beancount lets the tool derive it so the transaction
    /// balances) maps to a `None` amount.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::BadValue`] if `source_file` cannot be read, or
    /// [`ImportError::Parse`] if the file is not valid UTF-8, if a parse
    /// error is encountered, or if a transaction directive has no postings.
    #[inline]
    fn import(&self, config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        let cfg: Config = config.as_typed()?;
        let bytes = std::fs::read(&cfg.source_file).map_err(|e| ImportError::BadValue {
            field: "source_file".to_owned(),
            detail: format!("cannot read {:?}: {e}", cfg.source_file),
        })?;
        let text = core::str::from_utf8(&bytes)
            .map_err(|e| ImportError::Parse(format!("file is not valid UTF-8: {e}")))?;

        let directives = parse(text).map_err(ImportError::Parse)?;
        let file = &cfg.source_file;
        let mut raw_txs = Vec::new();

        for directive in directives {
            let Directive::Transaction(tx) = directive else {
                continue;
            };

            if tx.postings.is_empty() {
                return Err(ImportError::Parse("transaction has no postings".into()));
            }

            let mut postings = Vec::with_capacity(tx.postings.len());
            for posting in tx.postings {
                let amount = posting
                    .amount
                    .map(|PostingAmount { value, currency }| decimal_to_amount(value, currency))
                    .transpose()?;
                postings.push(
                    RawPosting::builder()
                        .account(posting.account)
                        .maybe_amount(amount)
                        .build(),
                );
            }

            raw_txs.push(
                RawTransaction::builder()
                    .date(tx.date)
                    .maybe_payee(tx.payee)
                    .description(tx.narration)
                    .tags(tx.tags)
                    .source_location(
                        SourceLocation::builder()
                            .display(format!("{file}:{}", tx.line))
                            .build(),
                    )
                    .postings(postings)
                    .build(),
            );
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
    use std::io::Write as _;

    use bc_sdk::Amount;
    use bc_sdk::Date;
    use bc_sdk::ImportConfig;
    use bc_sdk::Importer as _;
    use pretty_assertions::assert_eq;

    use super::*;

    /// Writes `text` to a fresh `.bean` file inside a fresh temp directory
    /// unique to `test_name` and returns an [`ImportConfig`] pointing at it.
    fn test_config(test_name: &str, text: &str) -> ImportConfig {
        let dir = std::env::temp_dir().join(format!("bc-beancount-{test_name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("ledger.bean");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(text.as_bytes()).expect("write");
        let source_file = path.to_str().expect("utf8 path").to_owned();
        ImportConfig::from_json_string(
            serde_json::json!({ "source_file": source_file }).to_string(),
        )
    }

    #[test]
    fn imports_transaction_payee_and_narration() {
        let input = "2025-01-15 * \"Acme\" \"Salary\"\n  Assets:Bank:Checking   4321.00 AUD\n  Income:Salary:Acme  -4321.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config(
                "imports_transaction_payee_and_narration",
                input,
            ))
            .expect("import");
        assert_eq!(txs.len(), 1);
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee.as_deref(), Some("Acme"));
        assert_eq!(tx.description, "Salary");
        assert_eq!(tx.date, Date::new(2025, 1, 15));
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "Assets:Bank:Checking");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(432_100, "AUD", 2)));
        assert_eq!(tx.postings[1].account, "Income:Salary:Acme");
        assert_eq!(tx.postings[1].amount, Some(Amount::new(-432_100, "AUD", 2)));
    }

    #[test]
    fn imports_narration_only() {
        let input = "2025-01-15 * \"Transfer\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config("imports_narration_only", input))
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee, None);
        assert_eq!(tx.description, "Transfer");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "A:B");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(100, "AUD", 2)));
        assert_eq!(tx.postings[1].account, "A:C");
        assert_eq!(tx.postings[1].amount, Some(Amount::new(-100, "AUD", 2)));
    }

    #[test]
    fn skips_open_commodity_directives() {
        let input = "2025-01-01 open Assets:Bank AUD\n2025-01-01 commodity AUD\n2025-01-15 * \"X\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config("skips_open_commodity_directives", input))
            .expect("import");
        assert_eq!(txs.len(), 1);
    }

    #[test]
    fn import_multi_currency_transaction_emits_all_postings() {
        let input =
            "2025-01-15 * \"FX Purchase\"\n  Assets:USD   100.00 USD\n  Assets:AUD  -150.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config(
                "import_multi_currency_transaction_emits_all_postings",
                input,
            ))
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
    fn import_elided_posting_maps_to_none_amount() {
        let input =
            "2025-01-15 * \"Payee\" \"Elided leg\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank\n";
        let txs = BeancountImporter
            .import(test_config(
                "import_elided_posting_maps_to_none_amount",
                input,
            ))
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "Expenses:Food");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(5000, "AUD", 2)));
        assert_eq!(tx.postings[1].account, "Assets:Bank");
        assert_eq!(tx.postings[1].amount, None);
    }

    #[test]
    fn import_transaction_header_tags_carry_into_raw_transaction() {
        let input = "2025-06-27 * \"Payee\" \"Narration\" #josh #groceries\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config(
                "import_transaction_header_tags_carry_into_raw_transaction",
                input,
            ))
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.tags, vec!["josh".to_owned(), "groceries".to_owned()]);
    }

    #[test]
    fn import_transaction_with_no_postings_returns_error() {
        // A transaction directive with zero postings is invalid; the importer
        // must return an error rather than panic.
        let input = "2025-01-15 * \"Payee\" \"No postings\"\n";
        let result = BeancountImporter.import(test_config(
            "import_transaction_with_no_postings_returns_error",
            input,
        ));
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
    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn source_location_names_the_file_and_the_header_line() {
        // A comment and a blank line precede the transaction, and its postings
        // follow the header, so a location naming line 1 or the posting's line
        // would both be wrong.
        let input = "; opening comment\n\n2025-01-15 * \"Woolworths\" \"Groceries\"\n    Expenses:Food    50.00 AUD\n    Assets:Bank   -50.00 AUD\n";
        let config = test_config("source_location_names_the_file", input);
        let expected = format!(
            "{}:3",
            std::env::temp_dir()
                .join("bc-beancount-source_location_names_the_file")
                .join("ledger.bean")
                .display()
        );

        let txs = BeancountImporter.import(config).expect("import");
        assert_eq!(txs.len(), 1);
        let location = txs[0]
            .source_location
            .as_ref()
            .expect("the beancount plugin reports where each transaction came from");
        assert_eq!(location.display, expected);
        assert!(
            location.uri.is_none(),
            "the beancount plugin does not populate a uri"
        );
    }
}
