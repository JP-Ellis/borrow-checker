//! Beancount importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for Beancount plain-text accounting files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for BeancountImporter` block
//! to generate the required WASM export glue.

#![cfg_attr(coverage_nightly, feature(coverage_attribute))]

mod ast;
mod config;
mod parser;
mod source;

pub use ast::BudgetPeriod;
use bc_sdk::AccountClose;
use bc_sdk::AccountOpen;
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
/// [`RawTransaction`] values. Every other directive is skipped; an
/// unrecognised keyword or a malformed `custom "budget"` line is skipped with
/// a warning.
#[derive(Debug, Default)]
pub struct BeancountImporter;

#[bc_sdk::importer]
impl bc_sdk::Importer for BeancountImporter {
    /// Returns the stable identifier for this importer.
    #[inline]
    fn name(&self) -> &str {
        "beancount"
    }

    /// Parses `cfg.source_file` as a Beancount file and returns its directives.
    ///
    /// # Arguments
    ///
    /// * `config` - Importer configuration; must supply `source_file`.
    ///
    /// # Returns
    ///
    /// One directive per `open`, `close` and transaction with postings, in
    /// source order. A transaction carries one [`RawPosting`] per source
    /// posting leg. A leg keeps its explicit amount when the source specifies
    /// one; a leg the source leaves elided (Beancount lets the tool derive it
    /// so the transaction balances) maps to a `None` amount.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::BadValue`] if `source_file` or any file it
    /// includes cannot be read — naming the include that referred to it — if
    /// the includes form a cycle, or if they nest deeper than the loader's
    /// limit. Returns [`ImportError::Parse`] if a file is not valid UTF-8, a
    /// parse error is encountered, or a posting annotation the parser rejects
    /// (a lot selection, a negative figure) appears.
    ///
    /// A transaction directive with no postings is skipped with a warning
    /// naming its line: Beancount accepts one, and it carries nothing an
    /// import could post.
    #[inline]
    fn import(&self, config: ImportConfig) -> Result<Vec<bc_sdk::Directive>, ImportError> {
        let cfg: Config = config.as_typed()?;
        let loaded = source::load(&cfg.source_file)?;
        for warning in &loaded.warnings {
            bc_sdk::warn!("beancount ledger warning"; detail = warning);
        }
        let mut directives = Vec::new();

        for Sourced { file, directive } in loaded.directives {
            match directive {
                Directive::Transaction(tx) => {
                    if tx.postings.is_empty() {
                        bc_sdk::warn!(
                            "transaction has no postings, skipping it";
                            location = format!("{file}:{}", tx.line)
                        );
                        continue;
                    }

                    let mut postings = Vec::with_capacity(tx.postings.len());
                    for posting in tx.postings {
                        let amount = posting.amount.map(|PostingAmount { value, currency }| {
                            Amount::new(value, alias(&cfg, &currency, &tx.date))
                        });
                        let price = posting
                            .price
                            .map(|quote| alias_quote(&cfg, quote, &tx.date));
                        let cost = posting.cost.map(|mut cost| {
                            cost.basis = alias_quote(&cfg, cost.basis, &tx.date);
                            cost
                        });
                        postings.push(
                            RawPosting::builder()
                                .account(posting.account)
                                .maybe_amount(amount)
                                .maybe_price(price)
                                .maybe_cost(cost)
                                .metadata(meta_entries(&cfg, posting.metadata, &tx.date))
                                .build(),
                        );
                    }

                    // The payee leads, then the file's own metadata lines in
                    // source order.
                    let mut metadata: Vec<MetaEntry> = tx
                        .payee
                        .into_iter()
                        .map(|name| MetaEntry::text("payee", name))
                        .collect();
                    metadata.extend(meta_entries(&cfg, tx.metadata, &tx.date));

                    directives.push(bc_sdk::Directive::from(
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
                    ));
                }
                Directive::Open {
                    date,
                    account,
                    currencies,
                    line,
                } => {
                    let commodities = currencies
                        .iter()
                        .map(|code| alias(&cfg, code, &date).to_owned())
                        .collect();
                    directives.push(bc_sdk::Directive::Open(
                        AccountOpen::builder()
                            .date(date)
                            .account(account)
                            .commodities(commodities)
                            .source_location(
                                SourceLocation::builder()
                                    .display(format!("{file}:{line}"))
                                    .build(),
                            )
                            .build(),
                    ));
                }
                Directive::Close {
                    date,
                    account,
                    line,
                } => {
                    directives.push(bc_sdk::Directive::Close(
                        AccountClose::builder()
                            .date(date)
                            .account(account)
                            .source_location(
                                SourceLocation::builder()
                                    .display(format!("{file}:{line}"))
                                    .build(),
                            )
                            .build(),
                    ));
                }
                _ => {}
            }
        }

        Ok(directives)
    }

    /// Rejects a `commodity_aliases` entry whose `since` is not a date.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::InvalidConfig`] naming the
    /// `commodity_aliases[i].since` entry that does not parse, or the
    /// underlying error if `config` does not deserialise.
    #[inline]
    fn validate(&self, config: ImportConfig) -> Result<(), ImportError> {
        let cfg: Config = config.as_typed()?;
        for (index, entry) in cfg.commodity_aliases.iter().enumerate() {
            entry.since_date().map_err(|detail| {
                ImportError::InvalidConfig(format!(
                    "commodity_aliases[{index}].since is not a YYYY-MM-DD date: {detail}"
                ))
            })?;
        }
        Ok(())
    }
}

/// Applies the configured aliases to one code on one date.
///
/// # Arguments
///
/// * `cfg` - The importer configuration, for its aliases.
/// * `code` - The commodity code as the ledger writes it.
/// * `on` - The date of the directive carrying the code.
///
/// # Returns
///
/// The aliased code, or `code` itself when no entry applies.
fn alias<'a>(cfg: &'a Config, code: &'a str, on: &bc_sdk::Date) -> &'a str {
    bc_sdk::alias::resolve(&cfg.commodity_aliases, code, on)
}

/// Applies the configured aliases to the commodity of a price or cost figure.
///
/// # Arguments
///
/// * `cfg` - The importer configuration, for its aliases.
/// * `quote` - The per-unit or total figure.
/// * `on` - The date of the transaction carrying the figure.
///
/// # Returns
///
/// The same figure with its commodity aliased.
fn alias_quote(cfg: &Config, quote: bc_sdk::Quote, on: &bc_sdk::Date) -> bc_sdk::Quote {
    match quote {
        bc_sdk::Quote::PerUnit(amount) => {
            bc_sdk::Quote::PerUnit(Amount::new(amount.value, alias(cfg, &amount.commodity, on)))
        }
        bc_sdk::Quote::Total(amount) => {
            bc_sdk::Quote::Total(Amount::new(amount.value, alias(cfg, &amount.commodity, on)))
        }
        other => other,
    }
}

/// Converts parsed metadata lines into the SDK's entries.
///
/// A beancount date and amount each become the matching typed value; a
/// timestamp has no beancount spelling, so no entry ever produces one here.
/// An amount's commodity is aliased as a posting's is.
///
/// # Arguments
///
/// * `cfg` - The importer configuration, for its aliases.
/// * `entries` - The parsed entries, in source order.
/// * `on` - The date of the transaction carrying the entries.
///
/// # Returns
///
/// The same entries in the same order.
fn meta_entries(cfg: &Config, entries: Vec<ast::MetaEntry>, on: &bc_sdk::Date) -> Vec<MetaEntry> {
    entries
        .into_iter()
        .map(|entry| {
            let value = match entry.value {
                ast::MetaValue::Text(text) => bc_sdk::MetaValue::Text(text),
                ast::MetaValue::Number(number) => bc_sdk::MetaValue::Number(number),
                ast::MetaValue::Boolean(flag) => bc_sdk::MetaValue::Boolean(flag),
                ast::MetaValue::Date(date) => bc_sdk::MetaValue::Date(date),
                ast::MetaValue::Amount(PostingAmount { value, currency }) => {
                    bc_sdk::MetaValue::Amount(Amount::new(value, alias(cfg, &currency, on)))
                }
                ast::MetaValue::Account(path) => bc_sdk::MetaValue::Account(path),
            };
            MetaEntry::new(entry.key, value)
        })
        .collect()
}

/// One Fava budget directive with its source location.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Budget {
    /// The directive date.
    pub date: bc_sdk::Date,
    /// The colon-separated account path.
    pub account: String,
    /// The period word, in Fava's vocabulary.
    pub period: BudgetPeriod,
    /// The evaluated amount.
    pub amount: rust_decimal::Decimal,
    /// The raw amount text, when it is an expression rather than a literal.
    pub expression: Option<String>,
    /// The commodity code.
    pub currency: String,
    /// `file:line`, as the importer reports transactions.
    pub location: String,
}

/// Reads every Fava budget directive reachable from `root`, in document order.
///
/// A native-only companion to [`BeancountImporter`]: the importer ABI carries
/// transactions, opens and closes alone, so a tool that wants the ledger's budgets calls this
/// directly. Includes are followed the same way.
///
/// # Arguments
///
/// * `root` - Path to the ledger's root file.
///
/// # Returns
///
/// The budget directives, in document order across the include graph.
///
/// # Errors
///
/// Returns [`ImportError`] if a file cannot be read or fails to parse, or
/// [`ImportError::Parse`] naming the first `custom "budget"` line that does
/// not read as a budget. The importer skips such a line with a warning; a
/// caller asking for budgets gets the whole set or nothing.
#[inline]
pub fn budgets(root: &str) -> Result<Vec<Budget>, ImportError> {
    let loaded = source::load(root)?;
    let malformed = loaded
        .directives
        .iter()
        .find_map(|Sourced { file, directive }| {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "only the malformed-budget carrier is wanted"
            )]
            match directive {
                Directive::MalformedBudget { line, reason } => {
                    Some(format!("{file}:{line}: {reason}"))
                }
                _ => None,
            }
        });
    if let Some(message) = malformed {
        return Err(ImportError::Parse(message));
    }
    for warning in &loaded.warnings {
        bc_sdk::warn!("beancount ledger warning"; detail = warning);
    }
    Ok(loaded
        .directives
        .into_iter()
        .filter_map(|Sourced { file, directive }| {
            #[expect(
                clippy::wildcard_enum_match_arm,
                reason = "only the budget variant is wanted"
            )]
            match directive {
                Directive::Budget(budget) => Some(Budget {
                    date: budget.date,
                    account: budget.account,
                    period: budget.period,
                    amount: budget.amount,
                    expression: budget.expression,
                    currency: budget.currency,
                    location: format!("{file}:{}", budget.line),
                }),
                _ => None,
            }
        })
        .collect())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::io::Write as _;

    use bc_sdk::Amount;
    use bc_sdk::Cost;
    use bc_sdk::Date;
    use bc_sdk::ImportConfig;
    use bc_sdk::Importer as _;
    use bc_sdk::MetaValue;
    use bc_sdk::Quote;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    /// Keeps the transactions among an importer's directives, in order.
    fn transactions(directives: Vec<bc_sdk::Directive>) -> Vec<RawTransaction> {
        directives
            .into_iter()
            .filter_map(bc_sdk::Directive::into_transaction)
            .collect()
    }

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
        let txs = transactions(
            BeancountImporter
                .import(test_config("metadata_reaches_both_levels", input))
                .expect("import"),
        );
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
        let txs = transactions(
            BeancountImporter
                .import(test_config("payee_precedes_own_entries", input))
                .expect("import"),
        );

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
        let txs = transactions(
            BeancountImporter
                .import(test_config(
                    "imports_transaction_payee_and_narration",
                    input,
                ))
                .expect("import"),
        );
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
        let txs = transactions(
            BeancountImporter
                .import(test_config("imports_narration_only", input))
                .expect("import"),
        );
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(payee_of(tx), None);
        assert_eq!(tx.description, "Transfer");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].account, "A:B");
        assert_eq!(tx.postings[0].amount, Some(Amount::new(dec!(1.00), "AUD")));
        assert_eq!(tx.postings[1].account, "A:C");
        assert_eq!(tx.postings[1].amount, Some(Amount::new(dec!(-1.00), "AUD")));
    }

    /// Writes `text` as a ledger like [`test_config`], with `aliases` as the
    /// `commodity_aliases` config value.
    fn aliased_config(test_name: &str, text: &str, aliases: &serde_json::Value) -> ImportConfig {
        let base = test_config(test_name, text);
        let mut value: serde_json::Value = base.as_typed().expect("config json");
        value["commodity_aliases"] = aliases.clone();
        ImportConfig::from_json_string(value.to_string())
    }

    #[test]
    fn emits_open_and_close_in_source_order() {
        let input = "2025-01-01 open Assets:Bank:Checking AUD\n\
                     2025-01-15 * \"X\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n\
                     2025-02-01 close Assets:Bank:Checking\n";
        let directives = BeancountImporter
            .import(test_config("emits_open_and_close_in_source_order", input))
            .expect("import");
        let dir = std::env::temp_dir().join("bc-beancount-emits_open_and_close_in_source_order");
        let at = |line: u32| {
            Some(
                SourceLocation::builder()
                    .display(format!("{}:{line}", dir.join("ledger.bean").display()))
                    .build(),
            )
        };

        let [
            bc_sdk::Directive::Open(open),
            bc_sdk::Directive::Transaction(tx),
            bc_sdk::Directive::Close(close),
        ] = directives.as_slice()
        else {
            panic!("expected [Open, Transaction, Close], got {directives:?}");
        };
        assert_eq!(open.date, Date::new(2025, 1, 1));
        assert_eq!(open.account, "Assets:Bank:Checking");
        assert_eq!(open.commodities, vec!["AUD".to_owned()]);
        assert_eq!(open.source_location, at(1));
        assert_eq!(tx.source_location, at(2));
        assert_eq!(close.date, Date::new(2025, 2, 1));
        assert_eq!(close.account, "Assets:Bank:Checking");
        assert_eq!(close.source_location, at(5));
    }

    #[test]
    fn include_keeps_source_order() {
        let dir = std::env::temp_dir().join("bc-beancount-include-keeps-order");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("main.bean"),
            "2025-01-01 open Assets:Bank:Checking AUD\n\
             include \"part.bean\"\n\
             2025-03-01 close Assets:Bank:Checking\n",
        )
        .expect("write root");
        std::fs::write(
            dir.join("part.bean"),
            "2025-02-01 * \"Generic Store\"\n  Expenses:Food   5.00 AUD\n  Assets:Bank:Checking\n\
             2025-02-02 open Expenses:Food AUD,XTS\n",
        )
        .expect("write included");
        let source_file = dir.join("main.bean").to_str().expect("utf8").to_owned();
        let config = ImportConfig::from_json_string(
            serde_json::json!({ "source_file": source_file }).to_string(),
        );

        let directives = BeancountImporter.import(config).expect("import");

        insta::with_settings!({
            // Debug escapes a Windows separator as `\\`.
            filters => vec![(r#"[^\s"]*bc-beancount-include-keeps-order(?:/|\\\\)"#, "[DIR]/")],
        }, {
            insta::assert_debug_snapshot!(directives);
        });
    }

    #[test]
    fn aliases_apply_to_every_code() {
        let input = "2023-06-01 open Assets:Broker AUD,XTS\n\
                     2024-02-01 * \"Buy\"\n  \
                     Assets:Broker  2 XTS {10 XTS} @ 12 XTS\n  \
                     Assets:Broker  1 XTS {{10 XTS}} @@ 12 XTS\n  \
                     Assets:Bank  -50 AUD\n";
        let aliases = serde_json::json!([
            { "from": "XTS", "to": "XTS.CRYPTO", "since": "2024-01-01" }
        ]);
        let directives = BeancountImporter
            .import(aliased_config(
                "aliases_apply_to_every_code",
                input,
                &aliases,
            ))
            .expect("import");

        let [
            bc_sdk::Directive::Open(open),
            bc_sdk::Directive::Transaction(tx),
        ] = directives.as_slice()
        else {
            panic!("expected [Open, Transaction], got {directives:?}");
        };
        assert_eq!(
            open.commodities,
            vec!["AUD".to_owned(), "XTS".to_owned()],
            "the open predates the alias"
        );
        let per_unit = tx.postings.first().expect("first leg");
        assert_eq!(per_unit.amount, Some(Amount::new(dec!(2), "XTS.CRYPTO")));
        assert_eq!(
            per_unit.price,
            Some(Quote::PerUnit(Amount::new(dec!(12), "XTS.CRYPTO")))
        );
        assert_eq!(
            per_unit.cost,
            Some(
                Cost::builder()
                    .basis(Quote::PerUnit(Amount::new(dec!(10), "XTS.CRYPTO")))
                    .build()
            )
        );
        let total = tx.postings.get(1).expect("second leg");
        assert_eq!(total.amount, Some(Amount::new(dec!(1), "XTS.CRYPTO")));
        assert_eq!(
            total.price,
            Some(Quote::Total(Amount::new(dec!(12), "XTS.CRYPTO")))
        );
        assert_eq!(
            total.cost,
            Some(
                Cost::builder()
                    .basis(Quote::Total(Amount::new(dec!(10), "XTS.CRYPTO")))
                    .build()
            )
        );
        assert_eq!(
            tx.postings.get(2).expect("third leg").amount,
            Some(Amount::new(dec!(-50), "AUD")),
            "a code with no alias crosses unchanged"
        );
    }

    /// A metadata amount is aliased at its transaction's date, at both
    /// levels, as a posting amount is.
    #[test]
    fn metadata_amounts_are_aliased() {
        let input = "2023-06-01 * \"Before\"\n\
                     \x20 fee: 1 XTS\n\
                     \x20 Assets:Bank  -1 AUD\n\
                     2024-02-01 * \"After\"\n\
                     \x20 fee: 2 XTS\n\
                     \x20 Assets:Bank  -2 AUD\n\
                     \x20   fee: 3 XTS\n";
        let aliases = serde_json::json!([
            { "from": "XTS", "to": "XTS.CRYPTO", "since": "2024-01-01" }
        ]);
        let txs = transactions(
            BeancountImporter
                .import(aliased_config(
                    "metadata_amounts_are_aliased",
                    input,
                    &aliases,
                ))
                .expect("import"),
        );
        let [before, after] = txs.as_slice() else {
            panic!("expected two transactions, got {txs:?}");
        };
        let fee = |value| {
            vec![MetaEntry::new(
                "fee",
                bc_sdk::MetaValue::Amount(Amount::new(value, "XTS.CRYPTO")),
            )]
        };

        assert_eq!(
            before.metadata,
            vec![MetaEntry::new(
                "fee",
                bc_sdk::MetaValue::Amount(Amount::new(dec!(1), "XTS")),
            )],
            "a transaction before the alias keeps the code"
        );
        assert_eq!(after.metadata, fee(dec!(2)));
        assert_eq!(
            after.postings.first().expect("one leg").metadata,
            fee(dec!(3))
        );
    }

    #[test]
    fn an_open_on_or_after_the_since_date_is_aliased() {
        let aliases = serde_json::json!([
            { "from": "XTS", "to": "XTS.CRYPTO", "since": "2024-01-01" }
        ]);
        let directives = BeancountImporter
            .import(aliased_config(
                "open_on_since_date",
                "2024-01-01 open Assets:Broker XTS\n",
                &aliases,
            ))
            .expect("import");
        let [bc_sdk::Directive::Open(open)] = directives.as_slice() else {
            panic!("expected one open, got {directives:?}");
        };
        assert_eq!(open.commodities, vec!["XTS.CRYPTO".to_owned()]);
    }

    #[test]
    fn validate_rejects_bad_alias_since() {
        let aliases = serde_json::json!([
            { "from": "XTS", "to": "XTS.CRYPTO", "since": "2024-01-01" },
            { "from": "AAA", "to": "BBB", "since": "last tuesday" }
        ]);
        let config = aliased_config("validate_bad_since", "", &aliases);
        let err = BeancountImporter
            .validate(config)
            .expect_err("a bad since date is rejected");
        let ImportError::InvalidConfig(message) = err else {
            panic!("expected InvalidConfig, got {err:?}");
        };
        assert!(message.contains("commodity_aliases[1].since"), "{message}");
    }

    #[test]
    fn commodity_and_balance_still_skipped() {
        let input = "2025-01-01 commodity AUD\n\
                     2025-01-02 balance Assets:Bank 0 AUD\n\
                     2025-01-15 * \"X\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let directives = BeancountImporter
            .import(test_config("commodity_and_balance_still_skipped", input))
            .expect("import");
        assert!(
            matches!(directives.as_slice(), [bc_sdk::Directive::Transaction(_)]),
            "{directives:?}"
        );
    }

    #[test]
    fn import_multi_currency_transaction_emits_all_postings() {
        let input =
            "2025-01-15 * \"FX Purchase\"\n  Assets:USD   100.00 USD\n  Assets:AUD  -150.00 AUD\n";
        let txs = transactions(
            BeancountImporter
                .import(test_config(
                    "import_multi_currency_transaction_emits_all_postings",
                    input,
                ))
                .expect("import should succeed even for multi-currency"),
        );
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
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn a_priced_and_a_costed_leg_reach_the_raw_posting() {
        let input = "2026-06-01 * \"Sell 2 AAPL\"\n  \
            Assets:Shares  -2 AAPL {105 AUD, 2024-03-01, \"lot-a\"} @ 150 AUD\n  \
            Assets:Bank  290 AUD\n  \
            Income:Gains\n";
        let txs = transactions(
            BeancountImporter
                .import(test_config(
                    "a_priced_and_a_costed_leg_reach_the_raw_posting",
                    input,
                ))
                .expect("import"),
        );
        let tx = txs.first().expect("one transaction");
        assert_eq!(tx.postings.len(), 3);
        let shares = &tx.postings[0];
        assert_eq!(shares.amount, Some(Amount::new(dec!(-2), "AAPL")));
        assert_eq!(
            shares.cost,
            Some(
                Cost::builder()
                    .basis(Quote::PerUnit(Amount::new(dec!(105), "AUD")))
                    .date(Date::new(2024, 3, 1))
                    .label("lot-a")
                    .build()
            )
        );
        assert_eq!(
            shares.price,
            Some(Quote::PerUnit(Amount::new(dec!(150), "AUD")))
        );
        assert_eq!(tx.postings[1].price, None);
        assert_eq!(tx.postings[1].cost, None);
        assert_eq!(tx.postings[2].amount, None);
    }

    #[test]
    fn import_elided_posting_maps_to_none_amount() {
        let input =
            "2025-01-15 * \"Payee\" \"Elided leg\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank\n";
        let txs = transactions(
            BeancountImporter
                .import(test_config(
                    "import_elided_posting_maps_to_none_amount",
                    input,
                ))
                .expect("import"),
        );
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
        let txs = transactions(
            BeancountImporter
                .import(test_config(
                    "import_transaction_header_tags_carry_into_raw_transaction",
                    input,
                ))
                .expect("import"),
        );
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.tags, vec!["josh".to_owned(), "groceries".to_owned()]);
    }

    #[test]
    fn import_transaction_with_no_postings_is_skipped() {
        // Beancount accepts a transaction directive with zero postings, and a
        // ledger extracted from statements can hold hundreds of them. One
        // must not stop the import of everything around it.
        let input = "2025-01-15 * \"Payee\" \"No postings\"\n\
                     2025-01-16 * \"Generic Store\" \"Groceries\"\n  \
                     Expenses:Food   50.00 AUD\n  \
                     Assets:Bank   -50.00 AUD\n";
        let txs = transactions(
            BeancountImporter
                .import(test_config(
                    "import_transaction_with_no_postings_is_skipped",
                    input,
                ))
                .expect("the empty directive is skipped, not fatal"),
        );
        assert_eq!(txs.len(), 1);
        assert_eq!(
            txs.first().expect("one transaction").description,
            "Groceries"
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

        let txs = transactions(BeancountImporter.import(config).expect("import"));
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

        let txs = transactions(BeancountImporter.import(config).expect("import"));
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

    #[test]
    fn budgets_follows_includes_and_reports_locations() {
        let dir = std::env::temp_dir().join("bc-beancount-budgets");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("main.bean"),
            "option \"title\" \"Household\"\n\ninclude \"budgets.bean\"\n",
        )
        .expect("write root");
        std::fs::write(
            dir.join("budgets.bean"),
            "2026-01-01 custom \"budget\" Expenses:Widgets \"monthly\" 500.00 AUD\n\
             2026-02-01 * \"Generic Store\" \"Widgets\"\n  Expenses:Widgets   50.00 AUD\n  Assets:Bank\n\
             2026-03-01 custom \"budget\" Expenses:Widgets \"yearly\" (6 * 100) AUD\n",
        )
        .expect("write included");

        let root = dir.join("main.bean").to_str().expect("utf8").to_owned();
        let found = budgets(&root).expect("budgets");
        assert_eq!(found.len(), 2);
        let first = found.first().expect("first");
        assert_eq!(first.account, "Expenses:Widgets");
        assert_eq!(first.period, BudgetPeriod::Monthly);
        assert_eq!(first.amount, rust_decimal_macros::dec!(500.00));
        assert_eq!(first.currency, "AUD");
        assert!(
            first.location.ends_with("budgets.bean:1"),
            "{}",
            first.location
        );
        // the literal budget
        assert_eq!(first.expression, None);
        let second = found.get(1).expect("second");
        assert_eq!(second.amount, rust_decimal_macros::dec!(600));
        assert!(
            second.location.ends_with("budgets.bean:5"),
            "{}",
            second.location
        );
        // the `(6 * 100)` budget
        assert_eq!(second.expression.as_deref(), Some("(6 * 100)"));
    }

    #[test]
    fn budgets_fails_on_a_malformed_budget_line() {
        let config = test_config(
            "budgets_fails_on_malformed",
            "2026-01-01 custom \"budget\" Expenses:Widgets \"monthly\" 500.00 AUD\n\
             2026-02-01 custom \"budget\" Expenses:Widgets \"fortnightly\" 5.00 AUD\n",
        );
        let cfg: Config = config.as_typed().expect("config");
        let err = budgets(&cfg.source_file).expect_err("a malformed budget fails budgets()");
        let ImportError::Parse(message) = err else {
            panic!("expected a parse error, got {err:?}");
        };
        assert!(
            message.contains("ledger.bean:2") && message.contains("fortnightly"),
            "{message}"
        );
    }

    #[test]
    fn import_skips_a_malformed_budget_line() {
        let config = test_config(
            "import_skips_malformed_budget",
            "2026-01-01 custom \"budget\" Expenses:Widgets \"monthly\"\n\
             2026-01-02 * \"Generic Store\" \"Widgets\"\n  Expenses:Widgets  5.00 AUD\n  Assets:Bank\n",
        );
        let txs = transactions(
            BeancountImporter
                .import(config)
                .expect("the transaction still imports"),
        );
        assert_eq!(txs.len(), 1);
    }
}
