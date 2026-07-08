//! CSV importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for delimited text (CSV) files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for CsvImporter` block
//! to generate the required WASM export glue.

mod config;
mod glob;
mod preamble;

use std::collections::HashMap;

use bc_sdk::Amount;
use bc_sdk::Date;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use rust_decimal::Decimal;

use crate::config::AmountColumns;
use crate::config::Config;
use crate::preamble::find_csv_start;

/// Imports transactions from delimited text (CSV) files.
///
/// Implements [`bc_sdk::Importer`] and is registered under the name `"csv"`.
/// Configuration is provided via a [`Config`] JSON blob.
#[derive(Debug, Default)]
pub struct CsvImporter;

#[bc_sdk::importer]
impl bc_sdk::Importer for CsvImporter {
    #[inline]
    fn name(&self) -> &str {
        "csv"
    }

    #[inline]
    fn import(&self, config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        let cfg: Config = config.as_typed()?;

        let files = crate::glob::matching_files(&cfg.source_dir, &cfg.source_glob)?;
        if files.is_empty() {
            return Err(ImportError::BadValue {
                field: "source_glob".to_owned(),
                detail: format!(
                    "no files under {:?} match {:?}",
                    cfg.source_dir, cfg.source_glob
                ),
            });
        }

        let mut all = Vec::new();
        for path in files {
            let display = path.display().to_string();
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) => {
                    bc_sdk::error!("failed to read import file"; path = display, reason = e.to_string());
                    continue;
                }
            };
            match self.parse_bytes(&bytes, &cfg) {
                Ok(mut txs) => all.append(&mut txs),
                Err(e) => {
                    bc_sdk::error!("failed to parse import file"; path = display, reason = e.to_string());
                }
            }
        }
        Ok(all)
    }
}

impl CsvImporter {
    /// Parses one file's `bytes` into raw transactions using `cfg`.
    ///
    /// Contains the delimiter validation, preamble skipping, header mapping,
    /// and row loop.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError`] on delimiter, header, or row parse failures.
    fn parse_bytes(&self, bytes: &[u8], cfg: &Config) -> Result<Vec<RawTransaction>, ImportError> {
        if !cfg.delimiter.is_ascii() {
            return Err(ImportError::BadValue {
                field: "delimiter".to_owned(),
                detail: format!(
                    "delimiter must be a single printable ASCII character, got {:?}",
                    cfg.delimiter
                ),
            });
        }

        let required = cfg.required_column_names();
        let csv_bytes = find_csv_start(bytes, &cfg.preamble, cfg.delimiter, &required)?;

        // SAFETY: non-ASCII delimiters are rejected above, so this truncation
        // is always lossless for printable ASCII characters.
        #[expect(
            clippy::as_conversions,
            reason = "delimiter is guaranteed ASCII by the is_ascii() guard above"
        )]
        let delimiter_byte = cfg.delimiter as u8;

        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter_byte)
            .trim(csv::Trim::All)
            .from_reader(csv_bytes);

        // Build a case-insensitive column-name → zero-based index map.
        let headers = reader
            .headers()
            .map_err(|e| ImportError::Parse(e.to_string()))?
            .clone();

        let col_index: HashMap<String, usize> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| (h.to_ascii_lowercase(), i))
            .collect();

        let lookup = |name: &str| -> Result<usize, ImportError> {
            col_index
                .get(&name.to_ascii_lowercase())
                .copied()
                .ok_or_else(|| ImportError::MissingField(name.to_owned()))
        };

        let date_idx = lookup(&cfg.date_column)?;

        let commodity = cfg
            .commodity
            .as_deref()
            .ok_or_else(|| ImportError::BadValue {
                field: "commodity".to_owned(),
                detail: "commodity must be set in config when the file does not contain a currency column".to_owned(),
            })?;

        let mut transactions = Vec::new();

        for result in reader.records() {
            let record = result.map_err(|e| ImportError::Parse(e.to_string()))?;

            let date_str = record_field(&record, date_idx, &cfg.date_column)?;
            let parsed = jiff::civil::Date::strptime(&cfg.date_format, &date_str).map_err(|e| {
                ImportError::BadValue {
                    field: cfg.date_column.clone(),
                    detail: e.to_string(),
                }
            })?;
            let date = Date::new(
                parsed.year() as i32,
                parsed.month() as u8,
                parsed.day() as u8,
            );

            let amount_value = parse_amount(&cfg, &record, &col_index)?;
            let amount = decimal_to_amount(amount_value, commodity, "amount")?;

            let balance = if let Some(col) = &cfg.balance_column {
                let &idx = col_index
                    .get(&col.to_ascii_lowercase())
                    .ok_or_else(|| ImportError::MissingField(col.clone()))?;
                let raw_owned = record_field(&record, idx, col)?;
                let raw = raw_owned.trim();
                if raw.is_empty() {
                    None
                } else {
                    let val = parse_number(raw, cfg.decimal_separator, cfg.thousands_separator)
                        .map_err(|e| ImportError::BadValue {
                            field: col.clone(),
                            detail: e,
                        })?;
                    Some(decimal_to_amount(val, commodity, col.as_str())?)
                }
            } else {
                None
            };

            let payee = cfg.payee_column.as_ref().and_then(|col| {
                col_index
                    .get(&col.to_ascii_lowercase())
                    .and_then(|&idx| record.get(idx))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            });

            let description = cfg
                .description_column
                .as_ref()
                .and_then(|col| {
                    col_index
                        .get(&col.to_ascii_lowercase())
                        .and_then(|&idx| record.get(idx))
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                })
                .unwrap_or_default();

            let reference = cfg.reference_column.as_ref().and_then(|col| {
                col_index
                    .get(&col.to_ascii_lowercase())
                    .and_then(|&idx| record.get(idx))
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
            });

            transactions.push(
                RawTransaction::builder()
                    .date(date)
                    .maybe_payee(payee)
                    .description(description)
                    .maybe_reference(reference)
                    .postings(vec![
                        RawPosting::builder()
                            .account(cfg.account.clone())
                            .amount(amount)
                            .maybe_balance(balance)
                            .build(),
                    ])
                    .build(),
            );
        }

        Ok(transactions)
    }
}

/// Converts a [`Decimal`] value and currency string into a [`bc_sdk::Amount`].
///
/// # Arguments
///
/// * `value` - The decimal value to convert.
/// * `currency` - The ISO 4217 currency code (e.g. `"AUD"`).
/// * `field_name` - The CSV field name, used in error messages.
///
/// # Returns
///
/// A [`bc_sdk::Amount`] with `minor_units`, `currency`, and `scale` set
/// from the decimal's mantissa and exponent.
///
/// # Errors
///
/// Returns [`ImportError::BadValue`] if the value overflows `i64` minor units.
#[inline]
fn decimal_to_amount(
    value: Decimal,
    currency: impl Into<String>,
    field_name: &str,
) -> Result<Amount, ImportError> {
    let scale = value.scale();
    // `Decimal::mantissa()` gives the integer backing the decimal: e.g.
    // `50.00` has mantissa=5000 and scale=2, which is exactly the minor-units
    // value we want.  No further multiplication is needed.
    let minor_units = i64::try_from(value.mantissa()).map_err(|_| ImportError::BadValue {
        field: field_name.to_owned(),
        detail: format!("amount {value} overflows i64 minor units"),
    })?;
    // scale is the number of decimal digits; monetary values have at most
    // 28 digits of scale so saturating to u8::MAX is safe in practice.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "scale.min(255) clamps to u8 range before the cast; monetary decimals have at most 28 digits"
    )]
    let scale_u8 = scale.min(255) as u8;
    Ok(Amount::new(minor_units, currency, scale_u8))
}

/// Returns the value of a record field at the given index, or an error if the
/// index is out of range.
///
/// # Arguments
///
/// * `record` - The CSV record to index into.
/// * `idx` - Zero-based column index.
/// * `column_name` - Human-readable column name used in the error message.
///
/// # Returns
///
/// The field value as an owned `String`.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] if `idx` is out of range.
#[inline]
fn record_field(
    record: &csv::StringRecord,
    idx: usize,
    column_name: &str,
) -> Result<String, ImportError> {
    record
        .get(idx)
        .map(str::to_owned)
        .ok_or_else(|| ImportError::MissingField(column_name.to_owned()))
}

/// Parses the monetary amount from a record using the configured amount
/// column strategy.
///
/// # Arguments
///
/// * `cfg` - The CSV import configuration.
/// * `record` - The CSV record being processed.
/// * `col_index` - Case-insensitive column name to index mapping.
///
/// # Returns
///
/// The parsed [`Decimal`] value, with debits negated.
///
/// # Errors
///
/// Returns [`ImportError`] if the column is missing or the value cannot be parsed.
#[inline]
fn parse_amount(
    cfg: &Config,
    record: &csv::StringRecord,
    col_index: &HashMap<String, usize>,
) -> Result<Decimal, ImportError> {
    match &cfg.amount_columns {
        AmountColumns::Single { column } => {
            let idx = col_index
                .get(&column.to_ascii_lowercase())
                .copied()
                .ok_or_else(|| ImportError::MissingField(column.clone()))?;
            let raw = record_field(record, idx, column)?;
            parse_number(&raw, cfg.decimal_separator, cfg.thousands_separator).map_err(|e| {
                ImportError::BadValue {
                    field: column.clone(),
                    detail: e,
                }
            })
        }
        AmountColumns::SplitDebitCredit {
            debit_column,
            credit_column,
        } => {
            let debit_idx = col_index.get(&debit_column.to_ascii_lowercase()).copied();
            let credit_idx = col_index.get(&credit_column.to_ascii_lowercase()).copied();

            let debit_raw = debit_idx
                .and_then(|i| record.get(i))
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let credit_raw = credit_idx
                .and_then(|i| record.get(i))
                .map(str::trim)
                .filter(|s| !s.is_empty());

            match (debit_raw, credit_raw) {
                (Some(d), None) => {
                    let val = parse_number(d, cfg.decimal_separator, cfg.thousands_separator)
                        .map_err(|e| ImportError::BadValue {
                            field: debit_column.clone(),
                            detail: e,
                        })?;
                    // Negate: a positive debit figure means money going out.
                    Ok(-val)
                }
                (None, Some(c)) => parse_number(c, cfg.decimal_separator, cfg.thousands_separator)
                    .map_err(|e| ImportError::BadValue {
                        field: credit_column.clone(),
                        detail: e,
                    }),
                (Some(_), Some(_)) => Err(ImportError::Parse(format!(
                    "both '{debit_column}' and '{credit_column}' are populated in the same row"
                ))),
                (None, None) => Err(ImportError::MissingField(format!(
                    "{debit_column} or {credit_column}"
                ))),
            }
        }
    }
}

/// Parses a numeric string, stripping currency symbols, thousands separators,
/// and normalising the decimal separator to `'.'`.
///
/// # Arguments
///
/// * `raw` - The raw string to parse.
/// * `decimal_sep` - The decimal separator character in use.
/// * `thousands_sep` - An optional thousands separator to strip.
///
/// # Returns
///
/// The parsed [`Decimal`] value.
///
/// # Errors
///
/// Returns a [`String`] describing the parse error.
#[inline]
fn parse_number(
    raw: &str,
    decimal_sep: char,
    thousands_sep: Option<char>,
) -> Result<Decimal, String> {
    // Strip leading/trailing whitespace, then separate any leading minus sign
    // before trimming currency prefixes/suffixes.
    let trimmed = raw.trim();

    // Detect accounting notation: (50.00) means negative.
    let (sign, magnitude_str) =
        if let Some(inner) = trimmed.strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
            ("-", inner)
        } else if let Some(rest) = trimmed.strip_prefix('-') {
            ("-", rest)
        } else {
            ("", trimmed)
        };

    let stripped_magnitude = magnitude_str
        .trim_matches(|c| matches!(c, '$' | '£' | '€' | '+'))
        .trim();
    let stripped = if sign.is_empty() {
        stripped_magnitude.to_owned()
    } else {
        format!("-{stripped_magnitude}")
    };

    // Remove thousands separator when configured.
    let without_thousands: String;
    let after_thousands = if let Some(ts) = thousands_sep {
        without_thousands = stripped.chars().filter(|&c| c != ts).collect();
        without_thousands.as_str()
    } else {
        stripped.as_str()
    };

    // Normalise decimal separator to '.'.
    let normalised: String;
    let normalised_str = if decimal_sep == '.' {
        after_thousands
    } else {
        normalised = after_thousands
            .chars()
            .map(|c| if c == decimal_sep { '.' } else { c })
            .collect();
        normalised.as_str()
    };

    normalised_str
        .parse::<Decimal>()
        .map_err(|e| format!("cannot parse '{raw}' as a decimal: {e}"))
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;

    use bc_sdk::Importer as _;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn parse_number_strips_currency_symbols() {
        assert_eq!(parse_number("$50.00", '.', None), Ok(dec!(50.00)));
        assert_eq!(parse_number("£100.50", '.', None), Ok(dec!(100.50)));
        assert_eq!(parse_number("€9.99", '.', None), Ok(dec!(9.99)));
    }

    #[test]
    fn parse_number_strips_thousands_separator() {
        assert_eq!(parse_number("1,234.56", '.', Some(',')), Ok(dec!(1234.56)));
    }

    #[test]
    fn parse_number_normalises_decimal_separator() {
        assert_eq!(parse_number("1234,56", ',', None), Ok(dec!(1234.56)));
    }

    #[test]
    fn parse_number_negative() {
        assert_eq!(parse_number("-50.00", '.', None), Ok(dec!(-50.00)));
    }

    #[test]
    fn parse_number_parenthesised_accounting_notation() {
        // Many Australian bank exports use (50.00) to represent a debit.
        assert_eq!(parse_number("(50.00)", '.', None), Ok(dec!(-50.00)));
        assert_eq!(
            parse_number("(1,234.56)", '.', Some(',')),
            Ok(dec!(-1234.56))
        );
        assert_eq!(parse_number("($99.95)", '.', None), Ok(dec!(-99.95)));
    }

    #[test]
    fn decimal_to_amount_round_trips_typical_bank_values() {
        assert_eq!(
            decimal_to_amount(dec!(50.00), "AUD", "amount").expect("50.00 should convert"),
            Amount::new(5000, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(-1234.56), "AUD", "amount").expect("-1234.56 should convert"),
            Amount::new(-123_456, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(0.00), "AUD", "amount").expect("0.00 should convert"),
            Amount::new(0, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(1.0), "AUD", "amount").expect("1.0 should convert"),
            Amount::new(10, "AUD", 1)
        );
    }

    #[test]
    fn import_simple_csv() {
        let dir = std::env::temp_dir().join("bc-csv-import-simple-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = b"Date,Amount,Description,Payee\n\
                    2025-03-15,50.00,Coffee shop,Java Hut\n\
                    2025-03-16,-120.00,Groceries,\n";
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(csv).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Assets:Bank:Checking",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "date_column": "Date",
            "date_format": "%Y-%m-%d",
            "amount_columns": {"style": "single", "column": "Amount"},
            "description_column": "Description",
            "payee_column": "Payee"
        });
        let config = ImportConfig::from_json_string(config_json.to_string());

        let importer = CsvImporter;
        let txns = importer.import(config).expect("import should succeed");

        assert_eq!(txns.len(), 2);

        let t0 = &txns[0];
        assert_eq!(t0.date, Date::new(2025, 3, 15));
        assert_eq!(t0.postings.len(), 1);
        assert_eq!(t0.postings[0].account, "Assets:Bank:Checking");
        // 50.00 AUD → minor_units=5000, scale=2
        assert_eq!(t0.postings[0].amount, Some(Amount::new(5000, "AUD", 2)));
        assert_eq!(t0.postings[0].balance, None);
        assert_eq!(t0.description, "Coffee shop");
        assert_eq!(t0.payee.as_deref(), Some("Java Hut"));

        let t1 = &txns[1];
        assert_eq!(t1.date, Date::new(2025, 3, 16));
        assert_eq!(t1.postings.len(), 1);
        assert_eq!(t1.postings[0].account, "Assets:Bank:Checking");
        // -120.00 AUD → minor_units=-12000, scale=2
        assert_eq!(t1.postings[0].amount, Some(Amount::new(-12000, "AUD", 2)));
        assert_eq!(t1.description, "Groceries");
        assert_eq!(t1.payee, None);
    }

    #[test]
    fn imports_all_matching_files_in_sorted_order() {
        // Exercises the multi-file loop in `import`: two files match the glob
        // and both parse, so their rows are unioned across files.
        let dir = std::env::temp_dir().join("bc-csv-import-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let header = "Date,Amount,Account Number,,Transaction Type,Transaction Details,Balance,Category,Merchant Name\n";
        let june = format!(
            "{header}27 Jun 25,-4321.00,123456789, ,TRANSFER DEBIT,ACME,0.00,Transfers out,\n"
        );
        let july =
            format!("{header}05 Jul 25,120.00,123456789, ,TRANSFER CREDIT,SALARY,120.00,Income,\n");
        let mut f = std::fs::File::create(dir.join("2025-06.csv")).expect("create june");
        f.write_all(june.as_bytes()).expect("write june");
        let mut g = std::fs::File::create(dir.join("2025-07.csv")).expect("create july");
        g.write_all(july.as_bytes()).expect("write july");

        let cfg = serde_json::json!({
            "account": "Assets:Bank:Checking",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "date_column": "Date",
            "date_format": "%d %b %y",
            "amount_columns": { "style": "single", "column": "Amount" },
            "description_column": "Transaction Details",
            "balance_column": "Balance",
            "commodity": "AUD"
        });
        let importer = CsvImporter;
        let txs = importer
            .import(ImportConfig::from_json_string(cfg.to_string()))
            .expect("import");
        assert_eq!(txs.len(), 2, "one row from each matching file");
        assert_eq!(txs[0].date, Date::new(2025, 6, 27));
        assert_eq!(txs[1].date, Date::new(2025, 7, 5));
    }

    #[test]
    fn imports_all_csvs_and_skips_a_bad_file() {
        // One good file and one that matches the glob but is not parseable for
        // this config. `import` logs the bad file via `bc_sdk::error!` (whose
        // native, off-wasm `__emit` is a no-op that drops the entry, so the
        // skip is silent here) and skips it, still returning the good file's
        // rows.
        let dir = std::env::temp_dir().join("bc-csv-import-skip-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let header = "Date,Amount,Account Number,,Transaction Type,Transaction Details,Balance,Category,Merchant Name\n";
        let good = format!(
            "{header}27 Jun 25,-4321.00,123456789, ,TRANSFER DEBIT,ACME,0.00,Transfers out,\n"
        );
        let mut f = std::fs::File::create(dir.join("2025-06.csv")).expect("create good");
        f.write_all(good.as_bytes()).expect("write good");
        let mut b = std::fs::File::create(dir.join("2025-07.csv")).expect("create bad");
        b.write_all(b"\x00\x00 not csv").expect("write bad");

        let cfg = serde_json::json!({
            "account": "Assets:Bank:Checking",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "date_column": "Date",
            "date_format": "%d %b %y",
            "amount_columns": { "style": "single", "column": "Amount" },
            "description_column": "Transaction Details",
            "balance_column": "Balance",
            "commodity": "AUD"
        });
        let importer = CsvImporter;
        let txs = importer
            .import(ImportConfig::from_json_string(cfg.to_string()))
            .expect("import succeeds despite the bad file");
        assert_eq!(txs.len(), 1, "one row from the good file; bad file skipped");
        assert_eq!(txs[0].date, Date::new(2025, 6, 27));
    }

    #[test]
    fn parse_bytes_errors_on_unparsable_content() {
        // `import`'s skip-and-continue path depends on `parse_bytes` returning
        // `Err` for a file that matches the glob but is not valid CSV for the
        // configured columns. This checks that underlying failure directly;
        // `imports_all_csvs_and_skips_a_bad_file` covers the end-to-end skip.
        let importer = CsvImporter;
        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            date_column: "Date".to_owned(),
            commodity: Some("AUD".to_owned()),
            ..Config::default()
        };
        let result = importer.parse_bytes(b"\x00\x00 not csv", &cfg);
        assert!(result.is_err());
    }

    #[test]
    fn import_errors_when_no_files_match_glob() {
        let dir = std::env::temp_dir().join("bc-csv-import-empty-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let cfg = serde_json::json!({
            "account": "Assets:Bank:Checking",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "commodity": "AUD"
        });
        let importer = CsvImporter;
        let result = importer.import(ImportConfig::from_json_string(cfg.to_string()));
        assert!(result.is_err());
    }
}
