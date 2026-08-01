//! BorrowChecker plugin for the Ledger plain-text accounting format.
//!
//! This crate implements [`bc_sdk::Importer`] for Ledger files and is compiled
//! to a WASM component for use with the BorrowChecker plugin host.

use bc_sdk::Amount;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use bc_sdk::SourceLocation;
use rust_decimal::Decimal;

mod ast;
mod config;
mod parser;

use ast::Entry;
use config::Config;
use parser::parse;

/// Implements [`bc_sdk::Importer`] for the Ledger plain-text accounting format.
#[non_exhaustive]
pub struct LedgerImporter;

impl LedgerImporter {
    /// Creates a new [`LedgerImporter`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LedgerImporter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

#[bc_sdk::importer]
impl bc_sdk::Importer for LedgerImporter {
    #[inline]
    fn name(&self) -> &str {
        "ledger"
    }

    /// Parses `cfg.source_file` as a Ledger file and returns the transactions.
    ///
    /// # Arguments
    ///
    /// * `config` - Importer configuration; must supply `source_file`.
    ///
    /// # Returns
    ///
    /// A list of [`RawTransaction`] values parsed from the ledger entries.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::BadValue`] if `source_file` cannot be read, or
    /// [`ImportError::Parse`] if the file is not valid UTF-8, if a parse
    /// error is encountered, or if a transaction entry has no postings.
    #[inline]
    fn import(&self, config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        let cfg: Config = config.as_typed()?;
        let bytes = std::fs::read(&cfg.source_file).map_err(|e| ImportError::BadValue {
            field: "source_file".to_owned(),
            detail: format!("cannot read {:?}: {e}", cfg.source_file),
        })?;
        let text = core::str::from_utf8(&bytes)
            .map_err(|e| ImportError::Parse(format!("file is not valid UTF-8: {e}")))?;

        let entries = parse(text).map_err(ImportError::Parse)?;
        let file = &cfg.source_file;

        let mut raw_txs = Vec::new();

        for entry in entries {
            let Entry::Transaction(tx) = entry else {
                continue;
            };

            if tx.postings.is_empty() {
                return Err(ImportError::Parse("transaction has no postings".into()));
            }

            let mut postings = Vec::with_capacity(tx.postings.len());
            for posting in tx.postings {
                let amount = posting
                    .amount
                    .map(|ast::PostingAmount { value, commodity }| {
                        decimal_to_amount(value, commodity)
                    })
                    .transpose()?;
                postings.push(
                    RawPosting::builder()
                        .account(posting.account)
                        .maybe_amount(amount)
                        .build(),
                );
            }

            let payee = if tx.payee.is_empty() {
                None
            } else {
                Some(tx.payee.clone())
            };
            let description = tx.comment.unwrap_or(tx.payee);

            raw_txs.push(
                RawTransaction::builder()
                    .date(tx.date)
                    .maybe_payee(payee)
                    .description(description)
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

    /// Accepts every configuration without checking it.
    ///
    /// Config validation rules for the ledger importer are not implemented
    /// yet; see [issue #361](https://github.com/JP-Ellis/borrow-checker/issues/361).
    #[inline]
    fn validate(&self, _config: ImportConfig) -> Result<(), ImportError> {
        bc_sdk::warn!(
            "config validation is not implemented for the ledger importer; config accepted without checks"
        );
        Ok(())
    }
}

/// Converts a [`rust_decimal::Decimal`] and currency string into a [`bc_sdk::Amount`].
///
/// The minor units are derived from the decimal's mantissa (already in minor-unit form).
///
/// # Arguments
///
/// * `value` - The decimal value to convert.
/// * `currency` - The ISO 4217 currency code, e.g. `"AUD"`.
///
/// # Returns
///
/// A [`bc_sdk::Amount`] with `minor_units`, `currency`, and `scale` set.
///
/// # Errors
///
/// Returns [`ImportError::Parse`] if the decimal mantissa does not fit in an
/// `i64` (i.e. the value is too large to represent as minor units).
#[inline]
fn decimal_to_amount(value: Decimal, currency: impl Into<String>) -> Result<Amount, ImportError> {
    // Decimal::mantissa() is already the unscaled integer (minor units).
    // For 50.00: mantissa=5000, scale=2 → minor_units=5000 (correct: 50.00 AUD = 5000 cents)
    let minor_units = i64::try_from(value.mantissa()).map_err(|_| {
        ImportError::Parse(format!(
            "amount mantissa overflows i64: {value} is too large to represent"
        ))
    })?;
    // rust_decimal caps scale at 28, well within u8::MAX (255); try_from makes
    // this invariant explicit and returns an error if it ever breaks.
    let scale = u8::try_from(value.scale()).map_err(|_| {
        ImportError::Parse(format!(
            "decimal scale {} exceeds u8 maximum ({})",
            value.scale(),
            u8::MAX
        ))
    })?;
    Ok(Amount::new(minor_units, currency, scale))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use bc_sdk::Importer as _;
    use pretty_assertions::assert_eq;

    use super::*;

    /// Writes `text` to a fresh ledger file inside a fresh temp directory
    /// unique to `test_name` and returns an [`ImportConfig`] pointing at it.
    fn test_config(test_name: &str, text: &str) -> ImportConfig {
        let dir = std::env::temp_dir().join(format!("bc-ledger-{test_name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("ledger.dat");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(text.as_bytes()).expect("write");
        let source_file = path.to_str().expect("utf8 path").to_owned();
        ImportConfig::from_json_string(
            serde_json::json!({ "source_file": source_file }).to_string(),
        )
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn imports_simple_transaction() {
        let input = "2025-01-15 * Woolworths\n    Expenses:Food    50.00 AUD\n    Assets:Bank   -50.00 AUD\n";
        let txs = LedgerImporter
            .import(test_config("imports_simple_transaction", input))
            .expect("import");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].payee.as_deref(), Some("Woolworths"));
        assert_eq!(txs[0].date, bc_sdk::Date::new(2025_i32, 1_u8, 15_u8));
        assert_eq!(txs[0].postings.len(), 2);
        assert_eq!(txs[0].postings[0].account, "Expenses:Food");
        assert_eq!(txs[0].postings[0].amount, Some(Amount::new(5000, "AUD", 2)));
        assert_eq!(txs[0].postings[1].account, "Assets:Bank");
        assert_eq!(
            txs[0].postings[1].amount,
            Some(Amount::new(-5000, "AUD", 2))
        );
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn elided_posting_maps_to_none_amount() {
        let input = "2025-01-17 Rent\n    Expenses:Rent    1500.00 AUD\n    Assets:Bank\n";
        let txs = LedgerImporter
            .import(test_config("elided_posting_maps_to_none_amount", input))
            .expect("import");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].postings.len(), 2);
        assert_eq!(txs[0].postings[0].account, "Expenses:Rent");
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(150_000, "AUD", 2))
        );
        assert_eq!(txs[0].postings[1].account, "Assets:Bank");
        assert_eq!(txs[0].postings[1].amount, None);
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let input = "; comment\n\n2025-01-15 * A\n    X    1.00 AUD\n    Y   -1.00 AUD\n";
        let txs = LedgerImporter
            .import(test_config("comments_and_blank_lines_ignored", input))
            .expect("import");
        assert_eq!(txs.len(), 1);
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
        let input = "; opening comment\n\n2025-01-15 * Woolworths\n    Expenses:Food    50.00 AUD\n    Assets:Bank   -50.00 AUD\n";
        let config = test_config("source_location_names_the_file", input);
        let expected = format!(
            "{}:3",
            std::env::temp_dir()
                .join("bc-ledger-source_location_names_the_file")
                .join("ledger.dat")
                .display()
        );

        let txs = LedgerImporter.import(config).expect("import");
        assert_eq!(txs.len(), 1);
        let location = txs[0]
            .source_location
            .as_ref()
            .expect("the ledger plugin reports where each transaction came from");
        assert_eq!(location.display, expected);
        assert!(
            location.uri.is_none(),
            "the ledger plugin does not populate a uri"
        );
    }
}
