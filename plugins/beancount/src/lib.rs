//! Beancount importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for Beancount plain-text accounting files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for BeancountImporter` block
//! to generate the required WASM export glue.

mod ast;
mod config;
mod parser;
mod source;

use bc_sdk::Amount;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::MetaEntry;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use bc_sdk::SourceLocation;

use crate::ast::Directive;
use crate::ast::PostingAmount;
use crate::config::Config;
use crate::source::Sourced;

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
    /// Returns [`ImportError::BadValue`] if `source_file` or any file it
    /// includes cannot be read — naming the include that referred to it — if
    /// the includes form a cycle, or if they nest deeper than the loader's
    /// limit. Returns [`ImportError::Parse`] if a file is not valid UTF-8, a
    /// parse error is encountered, or a transaction directive has no postings.
    #[inline]
    fn import(&self, config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        let cfg: Config = config.as_typed()?;
        let loaded = source::load(&cfg.source_file)?;
        for warning in &loaded.warnings {
            bc_sdk::warn!("beancount ledger warning"; detail = warning);
        }
        let mut raw_txs = Vec::new();

        for Sourced { file, directive } in loaded.directives {
            let Directive::Transaction(tx) = directive else {
                continue;
            };

            if tx.postings.is_empty() {
                return Err(ImportError::Parse(format!(
                    "{file}:{}: transaction has no postings",
                    tx.line
                )));
            }

            let mut postings = Vec::with_capacity(tx.postings.len());
            for posting in tx.postings {
                let amount = posting
                    .amount
                    .map(|PostingAmount { value, currency }| Amount::new(value, currency));
                postings.push(
                    RawPosting::builder()
                        .account(posting.account)
                        .maybe_amount(amount)
                        .metadata(meta_entries(posting.metadata))
                        .build(),
                );
            }

            // The payee leads, then the file's own metadata lines in source
            // order.
            let mut metadata: Vec<MetaEntry> = tx
                .payee
                .into_iter()
                .map(|name| MetaEntry::text("payee", name))
                .collect();
            metadata.extend(meta_entries(tx.metadata));

            raw_txs.push(
                RawTransaction::builder()
                    .date(tx.date)
                    .description(tx.narration)
                    .metadata(metadata)
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

    /// Accepts every configuration without checking it.
    ///
    /// Config validation rules for the beancount importer are not implemented
    /// yet; see [issue #361](https://github.com/JP-Ellis/borrow-checker/issues/361).
    #[inline]
    fn validate(&self, _config: ImportConfig) -> Result<(), ImportError> {
        Ok(())
    }
}

/// Converts parsed metadata lines into the SDK's entries.
///
/// A beancount date and amount each become the matching typed value; a
/// timestamp has no beancount spelling, so no entry ever produces one here.
///
/// # Arguments
///
/// * `entries` - The parsed entries, in source order.
///
/// # Returns
///
/// The same entries in the same order.
fn meta_entries(entries: Vec<ast::MetaEntry>) -> Vec<MetaEntry> {
    entries
        .into_iter()
        .map(|entry| {
            let value = match entry.value {
                ast::MetaValue::Text(text) => bc_sdk::MetaValue::Text(text),
                ast::MetaValue::Number(number) => bc_sdk::MetaValue::Number(number),
                ast::MetaValue::Boolean(flag) => bc_sdk::MetaValue::Boolean(flag),
                ast::MetaValue::Date(date) => bc_sdk::MetaValue::Date(date),
                ast::MetaValue::Amount(PostingAmount { value, currency }) => {
                    bc_sdk::MetaValue::Amount(Amount::new(value, currency))
                }
                ast::MetaValue::Account(path) => bc_sdk::MetaValue::Account(path),
            };
            MetaEntry::new(entry.key, value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use bc_sdk::Amount;
    use bc_sdk::Date;
    use bc_sdk::ImportConfig;
    use bc_sdk::Importer as _;
    use bc_sdk::MetaValue;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    /// Reads the first `payee` metadata entry, when the row states one.
    fn payee_of(tx: &RawTransaction) -> Option<&str> {
        tx.metadata.iter().find_map(|entry| match entry.value {
            MetaValue::Text(ref text) if entry.key == "payee" => Some(text.as_str()),
            _ => None,
        })
    }

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

    /// The motivating example from the design: metadata at both levels, and a
    /// key repeated across two legs.
    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn metadata_reaches_both_the_transaction_and_its_legs() {
        let input = "2026-01-15 * \"Some transaction\"\n\
                     \x20 note: hello!\n\
                     \x20 invoice: 1502\n\
                     \x20 Expenses:Health   100.00 AUD\n\
                     \x20   note: appointment\n\
                     \x20 Expenses:Health    50.00 AUD\n\
                     \x20   note: medication\n\
                     \x20 Assets:Bank      -150.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config("metadata_reaches_both_levels", input))
            .expect("import");
        let tx = txs.first().expect("one transaction");

        assert_eq!(
            tx.metadata,
            vec![
                MetaEntry::text("note", "hello!"),
                MetaEntry::number("invoice", dec!(1502)),
            ],
            "a file with no payee states no payee entry"
        );
        assert_eq!(
            tx.postings[0].metadata,
            vec![MetaEntry::text("note", "appointment")]
        );
        assert_eq!(
            tx.postings[1].metadata,
            vec![MetaEntry::text("note", "medication")]
        );
        assert_eq!(tx.postings[2].metadata, vec![]);
    }

    /// The payee leads, so the file's own entries follow it.
    #[test]
    fn the_payee_precedes_the_file_s_own_entries() {
        let input = "2026-01-15 * \"Generic Grocer\" \"Weekly shop\"\n\
                     \x20 settled: 2026-01-17\n\
                     \x20 Assets:Bank    -50.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config("payee_precedes_own_entries", input))
            .expect("import");

        assert_eq!(
            txs.first().expect("one transaction").metadata,
            vec![
                MetaEntry::text("payee", "Generic Grocer"),
                MetaEntry::date("settled", Date::new(2026, 1, 17)),
            ]
        );
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
        assert_eq!(payee_of(tx), Some("Acme"));
        assert_eq!(tx.description, "Salary");
        assert_eq!(tx.date, Date::new(2025, 1, 15));
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "Assets:Bank:Checking");
        assert_eq!(
            tx.postings[0].amount,
            Some(Amount::new(dec!(4321.00), "AUD"))
        );
        assert_eq!(tx.postings[1].account, "Income:Salary:Acme");
        assert_eq!(
            tx.postings[1].amount,
            Some(Amount::new(dec!(-4321.00), "AUD"))
        );
    }

    #[test]
    fn imports_narration_only() {
        let input = "2025-01-15 * \"Transfer\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(test_config("imports_narration_only", input))
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(payee_of(tx), None);
        assert_eq!(tx.description, "Transfer");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "A:B");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(dec!(1.00), "AUD")));
        assert_eq!(tx.postings[1].account, "A:C");
        assert_eq!(tx.postings[1].amount, Some(Amount::new(dec!(-1.00), "AUD")));
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
        assert_eq!(
            tx.postings[0].amount,
            Some(Amount::new(dec!(100.00), "USD"))
        );
        assert_eq!(tx.postings[1].account, "Assets:AUD");
        assert_eq!(
            tx.postings[1].amount,
            Some(Amount::new(dec!(-150.00), "AUD"))
        );
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
        assert_eq!(tx.postings[0].amount, Some(Amount::new(dec!(50.00), "AUD")));
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
    fn root_of_includes_imports_the_included_transactions() {
        // This is #401: a root file carrying only options and includes used to
        // import zero transactions and report success.
        let dir = std::env::temp_dir().join("bc-beancount-include-root");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("main.bean"),
            "option \"title\" \"Household\"\n\ninclude \"2025-01.bean\"\n",
        )
        .expect("write root");
        std::fs::write(
            dir.join("2025-01.bean"),
            "2025-01-15 * \"Generic Store\" \"Groceries\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank   -50.00 AUD\n",
        )
        .expect("write included");

        let source_file = dir.join("main.bean").to_str().expect("utf8").to_owned();
        let config = ImportConfig::from_json_string(
            serde_json::json!({ "source_file": source_file }).to_string(),
        );

        let txs = BeancountImporter.import(config).expect("import");
        assert_eq!(txs.len(), 1);
        let tx = txs.first().expect("one transaction");
        assert_eq!(tx.description, "Groceries");
        let location = tx
            .source_location
            .as_ref()
            .expect("transactions carry a source location");
        assert!(
            location.display.contains("2025-01.bean:1"),
            "the location names the file the transaction actually came from, \
             not the root that included it: {}",
            location.display
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
