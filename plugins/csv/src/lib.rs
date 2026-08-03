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
use bc_sdk::SourceLocation;
use rust_decimal::Decimal;

use crate::config::AmountColumns;
use crate::config::ColumnRef;
use crate::config::CommoditySource;
use crate::config::Config;
use crate::config::Header;
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
        // Validate before touching the filesystem. The host also validates
        // before calling in, but `import` is a public trait method: reached
        // directly, an incoherent config would otherwise fail per-file in the
        // loop below, which logs and skips, silently yielding zero
        // transactions.
        let cfg: Config = config.as_typed()?;
        cfg.validate()?;

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
            match self.parse_bytes(&bytes, &cfg, &display) {
                Ok(mut txs) => all.append(&mut txs),
                Err(e) => {
                    bc_sdk::error!("failed to parse import file"; path = display, reason = e.to_string());
                }
            }
        }
        Ok(all)
    }

    #[inline]
    fn validate(&self, config: ImportConfig) -> Result<(), ImportError> {
        let cfg: Config = config.as_typed()?;
        cfg.validate()
    }
}

impl CsvImporter {
    /// Parses one file's `bytes` into raw transactions using `cfg`.
    ///
    /// Contains the delimiter validation, preamble skipping, header mapping,
    /// and row loop.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The raw file content.
    /// * `cfg` - The CSV import configuration.
    /// * `file` - The file's display path, used to build each transaction's
    ///   [`SourceLocation`].
    ///
    /// # Errors
    ///
    /// Returns [`ImportError`] on delimiter, header, or row parse failures.
    fn parse_bytes(
        &self,
        bytes: &[u8],
        cfg: &Config,
        file: &str,
    ) -> Result<Vec<RawTransaction>, ImportError> {
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
        let csv_bytes =
            find_csv_start(bytes, &cfg.preamble, &cfg.header, cfg.delimiter, &required)?;

        // SAFETY: non-ASCII delimiters are rejected above, so this truncation
        // is always lossless for printable ASCII characters.
        #[expect(
            clippy::as_conversions,
            reason = "delimiter is guaranteed ASCII by the is_ascii() guard above"
        )]
        let delimiter_byte = cfg.delimiter as u8;

        let has_header_row = !matches!(cfg.header, Header::Absent);

        let mut reader = csv::ReaderBuilder::new()
            .delimiter(delimiter_byte)
            .has_headers(has_header_row)
            .trim(csv::Trim::All)
            .from_reader(csv_bytes);

        // Case-insensitive column-name → zero-based index map. Empty when the
        // file has no header row, in which case every reference is positional.
        let col_index: HashMap<String, usize> = if has_header_row {
            reader
                .headers()
                .map_err(|e| ImportError::Parse(e.to_string()))?
                .iter()
                .enumerate()
                .map(|(i, h)| (h.to_ascii_lowercase(), i))
                .collect()
        } else {
            HashMap::new()
        };

        let date_idx = resolve(&col_index, &cfg.date_column)?;

        let commodity_source = cfg.commodity.as_ref().ok_or_else(|| ImportError::BadValue {
            field: "commodity".to_owned(),
            detail: "commodity is not set; give a code or a column".to_owned(),
        })?;

        let mut transactions = Vec::new();

        for (row_idx, result) in reader.records().enumerate() {
            let row = row_idx.saturating_add(1);
            let record = result.map_err(|e| ImportError::Parse(e.to_string()))?;

            let date_str = record_field(&record, date_idx, &cfg.date_column.describe())?;
            let parsed = jiff::civil::Date::strptime(&cfg.date_format, &date_str).map_err(|e| {
                ImportError::BadValue {
                    field: cfg.date_column.describe(),
                    detail: e.to_string(),
                }
            })?;
            let date = Date::new(
                parsed.year() as i32,
                parsed.month() as u8,
                parsed.day() as u8,
            );

            let commodity = row_commodity(commodity_source, &record, &col_index)?;

            let amount_value = parse_amount(cfg, &record, &col_index)?;
            let amount = Amount::new(amount_value, commodity.clone());

            let balance = if let Some(column) = cfg.balance_column.as_ref() {
                let idx = resolve(&col_index, column)?;
                let raw_owned = record_field(&record, idx, &column.describe())?;
                let raw = raw_owned.trim();
                if raw.is_empty() {
                    None
                } else {
                    let val = parse_number(raw, cfg.decimal_separator, cfg.thousands_separator)
                        .map_err(|e| ImportError::BadValue {
                            field: column.describe(),
                            detail: e,
                        })?;
                    Some(Amount::new(val, commodity.clone()))
                }
            } else {
                None
            };

            let payee = optional_text(&record, &col_index, cfg.payee_column.as_ref())?;
            let description = optional_text(&record, &col_index, cfg.description_column.as_ref())?
                .unwrap_or_default();
            let reference = optional_text(&record, &col_index, cfg.reference_column.as_ref())?;

            transactions.push(
                RawTransaction::builder()
                    .date(date)
                    .maybe_payee(payee)
                    .description(description)
                    .maybe_reference(reference)
                    .source_location(
                        SourceLocation::builder()
                            .display(format!("{file} data row {row}"))
                            .build(),
                    )
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

/// Reads an optional free-text column from a record.
///
/// A column that is not configured yields `None`. A column that *is*
/// configured must resolve and must be present in the row: an unresolvable
/// name or an index past the end of the row is a configuration error, not an
/// empty value. This matches `balance_column` and the required columns, so
/// that a mistyped column reference cannot import as silently blank text.
///
/// # Arguments
///
/// * `record` - The CSV record being processed.
/// * `col_index` - Case-insensitive header-name to index map.
/// * `column` - The configured reference, if any.
///
/// # Returns
///
/// The trimmed field value, or `None` when the column is unconfigured or the
/// cell is empty.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] when a configured column cannot be
/// resolved or the row is too short to contain it.
#[inline]
fn optional_text(
    record: &csv::StringRecord,
    col_index: &HashMap<String, usize>,
    column: Option<&ColumnRef>,
) -> Result<Option<String>, ImportError> {
    let Some(column) = column else {
        return Ok(None);
    };
    let idx = resolve(col_index, column)?;
    let raw = record_field(record, idx, &column.describe())?;
    let trimmed = raw.trim();
    Ok((!trimmed.is_empty()).then(|| trimmed.to_owned()))
}

/// Resolves a column reference to a zero-based index within a row.
///
/// # Arguments
///
/// * `col_index` - Case-insensitive header-name to index map. Empty when the
///   file has no header row.
/// * `column` - The reference to resolve.
///
/// # Returns
///
/// The zero-based column index.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] when a name-based reference does not
/// appear in the header.
#[inline]
fn resolve(col_index: &HashMap<String, usize>, column: &ColumnRef) -> Result<usize, ImportError> {
    match *column {
        ColumnRef::Name(ref name) => col_index
            .get(&name.to_ascii_lowercase())
            .copied()
            .ok_or_else(|| ImportError::MissingField(column.describe())),
        ColumnRef::Index(index) => Ok(index),
    }
}

/// Resolves the commodity code that applies to one row.
///
/// # Arguments
///
/// * `source` - Where the code comes from.
/// * `record` - The CSV record being processed.
/// * `col_index` - Case-insensitive header-name to index map.
///
/// # Returns
///
/// The commodity code for this row.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] when a column reference does not
/// resolve, or [`ImportError::BadValue`] when a column-sourced cell is blank —
/// a `Column` source asserts that every row names its commodity.
#[inline]
fn row_commodity(
    source: &CommoditySource,
    record: &csv::StringRecord,
    col_index: &HashMap<String, usize>,
) -> Result<String, ImportError> {
    match *source {
        CommoditySource::Fixed { ref code } => Ok(code.clone()),
        CommoditySource::Column { ref column } => {
            let idx = resolve(col_index, column)?;
            let raw = record_field(record, idx, &column.describe())?;
            let trimmed = raw.trim();
            if trimmed.is_empty() {
                return Err(ImportError::BadValue {
                    field: column.describe(),
                    detail: "the commodity cell is blank; every row must name its commodity"
                        .to_owned(),
                });
            }
            Ok(trimmed.to_owned())
        }
    }
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
    match cfg.amount_columns {
        AmountColumns::Single { ref column } => {
            let idx = resolve(col_index, column)?;
            let raw = record_field(record, idx, &column.describe())?;
            parse_number(&raw, cfg.decimal_separator, cfg.thousands_separator).map_err(|e| {
                ImportError::BadValue {
                    field: column.describe(),
                    detail: e,
                }
            })
        }
        AmountColumns::SplitDebitCredit {
            ref debit_column,
            ref credit_column,
        } => {
            // Both references must resolve; only the *cells* may be empty,
            // since exactly one of the pair is populated per row.
            let debit_idx = resolve(col_index, debit_column)?;
            let credit_idx = resolve(col_index, credit_column)?;

            let debit_raw = record
                .get(debit_idx)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let credit_raw = record
                .get(credit_idx)
                .map(str::trim)
                .filter(|s| !s.is_empty());

            match (debit_raw, credit_raw) {
                (Some(d), None) => {
                    let val = parse_number(d, cfg.decimal_separator, cfg.thousands_separator)
                        .map_err(|e| ImportError::BadValue {
                            field: debit_column.describe(),
                            detail: e,
                        })?;
                    // Negate: a positive debit figure means money going out.
                    Ok(-val)
                }
                (None, Some(c)) => parse_number(c, cfg.decimal_separator, cfg.thousands_separator)
                    .map_err(|e| ImportError::BadValue {
                        field: credit_column.describe(),
                        detail: e,
                    }),
                (Some(_), Some(_)) => Err(ImportError::Parse(format!(
                    "both {} and {} are populated in the same row",
                    debit_column.describe(),
                    credit_column.describe()
                ))),
                (None, None) => Err(ImportError::MissingField(format!(
                    "{} or {}",
                    debit_column.describe(),
                    credit_column.describe()
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
        assert_eq!(t0.postings[0].amount, Some(Amount::new(dec!(50.00), "AUD")));
        assert_eq!(t0.postings[0].balance, None);
        assert_eq!(t0.description, "Coffee shop");
        assert_eq!(t0.payee.as_deref(), Some("Java Hut"));

        let t1 = &txns[1];
        assert_eq!(t1.date, Date::new(2025, 3, 16));
        assert_eq!(t1.postings.len(), 1);
        assert_eq!(t1.postings[0].account, "Assets:Bank:Checking");
        assert_eq!(
            t1.postings[0].amount,
            Some(Amount::new(dec!(-120.00), "AUD"))
        );
        assert_eq!(t1.description, "Groceries");
        assert_eq!(t1.payee, None);
    }

    #[test]
    fn source_location_names_file_and_data_row_after_preamble() {
        // Two metadata lines precede the header, skipped via `SkipLines`. The
        // `display` string must count data rows (post-preamble, post-header),
        // not physical file lines — this pins that documented scheme end to
        // end rather than trusting the preamble skip to be a no-op.
        let dir = std::env::temp_dir().join("bc-csv-import-source-location-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = b"Statement export\nGenerated 2025-03-01\n\
                    Date,Amount,Description,Payee\n\
                    2025-03-15,50.00,Coffee shop,COFFEE\n\
                    2025-03-16,-120.00,Groceries,ACME\n";
        let path = dir.join("statement.csv");
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(csv).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Assets:Bank:Checking",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "preamble": {"strategy": "skip_lines", "lines": 2},
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
        let display_path = path.display().to_string();

        let location0 = txns[0]
            .source_location
            .as_ref()
            .expect("source location should be set");
        assert_eq!(location0.display, format!("{display_path} data row 1"));

        let location1 = txns[1]
            .source_location
            .as_ref()
            .expect("source location should be set");
        assert_eq!(location1.display, format!("{display_path} data row 2"));
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
            date_column: ColumnRef::Name("Date".to_owned()),
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };
        let result = importer.parse_bytes(b"\x00\x00 not csv", &cfg, "statement.csv");
        assert!(result.is_err());
    }

    #[test]
    fn import_headerless_csv_by_index() {
        let dir = std::env::temp_dir().join("bc-csv-import-headerless-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = b"01/02/2025,120.00,GENERIC GROCER\n\
                    02/02/2025,-45.00,ACME HARDWARE\n";
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(csv).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "header": {"kind": "absent"},
            "date_column": 0,
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1},
            "description_column": 2
        });

        let importer = CsvImporter;
        let txns = importer
            .import(ImportConfig::from_json_string(config_json.to_string()))
            .expect("import should succeed");

        assert_eq!(txns.len(), 2, "no row should be consumed as a header");
        assert_eq!(txns[0].date, Date::new(2025, 2, 1));
        assert_eq!(
            txns[0].postings[0].amount,
            Some(Amount::new(dec!(120.00), "AUD"))
        );
        assert_eq!(txns[0].description, "GENERIC GROCER");
        assert_eq!(txns[1].date, Date::new(2025, 2, 2));
        assert_eq!(
            txns[1].postings[0].amount,
            Some(Amount::new(dec!(-45.00), "AUD"))
        );
    }

    #[test]
    fn import_rejects_named_columns_on_a_headerless_file() {
        // The failure must arrive from config validation, not from the per-file
        // loop — which logs and skips, silently yielding zero transactions.
        let dir = std::env::temp_dir().join("bc-csv-import-invalid-config-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(b"01/02/2025,120.00,GENERIC GROCER\n")
            .expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "header": {"kind": "absent"},
            "date_column": "Date",
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1}
        });

        let importer = CsvImporter;
        let err = importer
            .import(ImportConfig::from_json_string(config_json.to_string()))
            .expect_err("a named column on a headerless file is not importable");
        assert!(
            err.to_string().contains("date_column"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn validate_rejects_named_columns_on_a_headerless_file() {
        // Same incoherent combination as `import_rejects_named_columns_on_a_headerless_file`,
        // but exercised through `validate` directly: no file is read, so this
        // must fail from config coherence alone.
        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": "unused",
            "source_glob": "*.csv",
            "header": {"kind": "absent"},
            "date_column": "Date",
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1}
        });

        let importer = CsvImporter;
        let err = importer
            .validate(ImportConfig::from_json_string(config_json.to_string()))
            .expect_err("a named column on a headerless file is not importable");
        assert!(
            err.to_string().contains("date_column"),
            "error should name the offending field, got: {err}"
        );
    }

    #[test]
    fn validate_accepts_a_coherent_headerless_config() {
        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": "unused",
            "source_glob": "*.csv",
            "header": {"kind": "absent"},
            "date_column": 0,
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1},
            "description_column": 2
        });

        let importer = CsvImporter;
        importer
            .validate(ImportConfig::from_json_string(config_json.to_string()))
            .expect("coherent headerless config should validate");
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

    #[test]
    fn import_headerless_four_column_with_running_balance() {
        // Shape: date, signed amount, description, running balance. dd/mm/yyyy,
        // quoted fields, a leading '+' on credits.
        let dir = std::env::temp_dir().join("bc-csv-import-headerless-balance-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = "01/02/2025,\"+120.00\",\"Repayment/Payment\",\"-1234.56\"\n\
                   02/02/2025,\"-45.00\",\"GENERIC GROCER\",\"-1279.56\"\n";
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(csv.as_bytes()).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "header": {"kind": "absent"},
            "date_column": 0,
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1},
            "description_column": 2,
            "balance_column": 3
        });

        let importer = CsvImporter;
        let txns = importer
            .import(ImportConfig::from_json_string(config_json.to_string()))
            .expect("import should succeed");

        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].date, Date::new(2025, 2, 1));
        assert_eq!(
            txns[0].postings[0].amount,
            Some(Amount::new(dec!(120.00), "AUD"))
        );
        assert_eq!(
            txns[0].postings[0].balance,
            Some(Amount::new(dec!(-1234.56), "AUD"))
        );
        assert_eq!(txns[0].description, "Repayment/Payment");
        assert_eq!(
            txns[1].postings[0].amount,
            Some(Amount::new(dec!(-45.00), "AUD"))
        );
    }

    #[test]
    fn import_headerless_three_column_without_balance() {
        // Shape: date, signed amount, description. Unquoted description with
        // runs of internal whitespace, which `Trim::All` must not collapse.
        let dir = std::env::temp_dir().join("bc-csv-import-headerless-nobalance-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = "03/02/2025,\"-33.03\",GENERIC GROCER - SUBURB    SUBURB NORTH\n";
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(csv.as_bytes()).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "header": {"kind": "absent"},
            "date_column": 0,
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1},
            "description_column": 2
        });

        let importer = CsvImporter;
        let txns = importer
            .import(ImportConfig::from_json_string(config_json.to_string()))
            .expect("import should succeed");

        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].date, Date::new(2025, 2, 3));
        assert_eq!(
            txns[0].postings[0].amount,
            Some(Amount::new(dec!(-33.03), "AUD"))
        );
        assert_eq!(
            txns[0].description,
            "GENERIC GROCER - SUBURB    SUBURB NORTH"
        );
        assert_eq!(txns[0].postings[0].balance, None);
    }

    #[test]
    fn import_headerless_with_a_preamble_to_skip() {
        // The two axes are independent: discard a banner *and* have no header.
        let dir = std::env::temp_dir().join("bc-csv-import-headerless-preamble-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = "Statement export\n\
                   01/02/2025,120.00,GENERIC GROCER\n";
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(csv.as_bytes()).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Liabilities:Bank:Card",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "preamble": {"strategy": "skip_lines", "lines": 1},
            "header": {"kind": "absent"},
            "date_column": 0,
            "date_format": "%d/%m/%Y",
            "amount_columns": {"style": "single", "column": 1},
            "description_column": 2
        });

        let importer = CsvImporter;
        let txns = importer
            .import(ImportConfig::from_json_string(config_json.to_string()))
            .expect("import should succeed");

        assert_eq!(
            txns.len(),
            1,
            "the banner is discarded, the data row is kept"
        );
        assert_eq!(txns[0].date, Date::new(2025, 2, 1));
    }

    #[test]
    fn import_mixes_named_and_positional_columns_with_a_header() {
        // The header has a blank name, which cannot be addressed by name at all.
        let dir = std::env::temp_dir().join("bc-csv-import-mixed-addressing-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");

        let csv = "Date,Amount,,Details\n\
                   2025-02-01,120.00,IGNORED,GENERIC GROCER\n";
        let mut f = std::fs::File::create(dir.join("statement.csv")).expect("create");
        f.write_all(csv.as_bytes()).expect("write");

        let config_json = serde_json::json!({
            "commodity": "AUD",
            "account": "Assets:Bank:Checking",
            "source_dir": dir.to_str().expect("utf8"),
            "source_glob": "*.csv",
            "date_column": "Date",
            "date_format": "%Y-%m-%d",
            "amount_columns": {"style": "single", "column": "Amount"},
            "description_column": 3
        });

        let importer = CsvImporter;
        let txns = importer
            .import(ImportConfig::from_json_string(config_json.to_string()))
            .expect("import should succeed");

        assert_eq!(txns.len(), 1);
        assert_eq!(txns[0].description, "GENERIC GROCER");
        assert_eq!(
            txns[0].postings[0].amount,
            Some(Amount::new(dec!(120.00), "AUD"))
        );
    }

    #[test]
    fn parse_bytes_errors_on_out_of_range_index() {
        // Out-of-range is a parse-time error by design: it cannot be known from
        // the config alone, so `validate` does not and cannot catch it.
        let importer = CsvImporter;
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(9),
            },
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };
        let result = importer.parse_bytes(b"01/02/2025,120.00\n", &cfg, "statement.csv");
        let err = result.expect_err("column 9 does not exist in a two-column row");
        assert_eq!(err.to_string(), "missing required field: column 9");
    }

    /// An out-of-range index on an *optional* column must fail like a required
    /// one, rather than importing a silently blank value.
    ///
    /// Hand-counting indices off a headerless file makes an off-by-one the
    /// likeliest mistake this addressing mode introduces.
    fn headerless_two_column_config() -> Config {
        Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        }
    }

    #[test]
    fn parse_bytes_errors_on_out_of_range_payee_index() {
        let cfg = Config {
            payee_column: Some(ColumnRef::Index(9)),
            ..headerless_two_column_config()
        };
        let result = CsvImporter.parse_bytes(b"01/02/2025,120.00\n", &cfg, "statement.csv");
        let err = result.expect_err("column 9 does not exist in a two-column row");
        assert_eq!(err.to_string(), "missing required field: column 9");
    }

    #[test]
    fn parse_bytes_errors_on_out_of_range_description_index() {
        let cfg = Config {
            description_column: Some(ColumnRef::Index(9)),
            ..headerless_two_column_config()
        };
        let result = CsvImporter.parse_bytes(b"01/02/2025,120.00\n", &cfg, "statement.csv");
        let err = result.expect_err("column 9 does not exist in a two-column row");
        assert_eq!(err.to_string(), "missing required field: column 9");
    }

    #[test]
    fn parse_bytes_errors_on_out_of_range_reference_index() {
        let cfg = Config {
            reference_column: Some(ColumnRef::Index(9)),
            ..headerless_two_column_config()
        };
        let result = CsvImporter.parse_bytes(b"01/02/2025,120.00\n", &cfg, "statement.csv");
        let err = result.expect_err("column 9 does not exist in a two-column row");
        assert_eq!(err.to_string(), "missing required field: column 9");
    }

    /// A name-addressed optional column absent from the header is likewise a
    /// configuration error rather than a silently blank value.
    #[test]
    fn parse_bytes_errors_on_an_absent_optional_column_name() {
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Present,
            date_column: ColumnRef::Name("Date".to_owned()),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Amount".to_owned()),
            },
            description_column: Some(ColumnRef::Name("Narrative".to_owned())),
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };
        let result =
            CsvImporter.parse_bytes(b"Date,Amount\n01/02/2025,120.00\n", &cfg, "statement.csv");
        let err = result.expect_err("'Narrative' is not in the header");
        assert_eq!(err.to_string(), "missing required field: 'Narrative'");
    }

    /// An optional column that resolves but holds an empty cell is still
    /// absent, not an error — only unresolvable references fail.
    #[test]
    fn parse_bytes_treats_an_empty_optional_cell_as_absent() {
        let importer = CsvImporter;
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            payee_column: Some(ColumnRef::Index(2)),
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };

        let txs = importer
            .parse_bytes(b"01/02/2025,120.00,\n", &cfg, "statement.csv")
            .expect("an empty payee cell is not an error");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].payee, None);
    }

    /// Split debit/credit columns must work under positional addressing.
    #[test]
    fn parse_bytes_handles_headerless_split_debit_credit() {
        let importer = CsvImporter;
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Index(1),
                credit_column: ColumnRef::Index(2),
            },
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };

        let data = b"01/02/2025,33.03,\n02/02/2025,,369.00\n";
        let txs = importer
            .parse_bytes(data, &cfg, "statement.csv")
            .expect("split debit/credit by index should parse");

        assert_eq!(txs.len(), 2);
        // A positive debit figure is money out, so it is negated.
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(dec!(-33.03), "AUD"))
        );
        assert_eq!(
            txs[1].postings[0].amount,
            Some(Amount::new(dec!(369.00), "AUD"))
        );
    }

    /// Both split columns populated in one row is a per-row parse error, and
    /// the message must name the columns by their positional description.
    #[test]
    fn parse_bytes_rejects_both_split_columns_populated_by_index() {
        let importer = CsvImporter;
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Index(1),
                credit_column: ColumnRef::Index(2),
            },
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };

        let result = importer.parse_bytes(b"01/02/2025,33.03,369.00\n", &cfg, "statement.csv");
        let err = result.expect_err("a row cannot be both a debit and a credit");
        assert_eq!(
            err.to_string(),
            "parse error: both column 1 and column 2 are populated in the same row"
        );
    }

    /// Neither split column populated names both columns in the error.
    #[test]
    fn parse_bytes_rejects_neither_split_column_populated_by_index() {
        let importer = CsvImporter;
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Index(1),
                credit_column: ColumnRef::Index(2),
            },
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };

        let result = importer.parse_bytes(b"01/02/2025,,\n", &cfg, "statement.csv");
        let err = result.expect_err("a row must be either a debit or a credit");
        assert_eq!(
            err.to_string(),
            "missing required field: column 1 or column 2"
        );
    }

    #[test]
    fn import_takes_the_commodity_from_a_named_column() {
        let csv = "Date,Coin,Change\n\
                   2025-01-02,BTC,0.25000000\n\
                   2025-01-03,ETH,-1.50000000\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Change".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Coin".to_owned()),
            }),
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(dec!(0.25000000), "BTC"))
        );
        assert_eq!(
            txs[1].postings[0].amount,
            Some(Amount::new(dec!(-1.50000000), "ETH"))
        );
    }

    #[test]
    fn import_takes_the_commodity_from_an_indexed_column() {
        let csv = "2025-01-02,BTC,0.25\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(2),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Index(1),
            }),
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(dec!(0.25), "BTC"))
        );
    }

    /// `Column` asserts every row names a commodity, so a blank cell is
    /// malformed input rather than a default.
    #[test]
    fn import_rejects_a_blank_commodity_cell() {
        let csv = "Date,Coin,Change\n2025-01-02,,0.25\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Change".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Coin".to_owned()),
            }),
            ..Config::default()
        };
        let msg = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect_err("a blank commodity cell is malformed")
            .to_string();
        assert!(msg.contains("Coin"), "should name the column: {msg}");
    }

    #[test]
    fn import_still_applies_a_fixed_commodity_to_every_row() {
        let csv = "Date,Amount\n2025-01-02,50.00\n2025-01-03,-20.00\n";
        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(dec!(50.00), "AUD"))
        );
        assert_eq!(
            txs[1].postings[0].amount,
            Some(Amount::new(dec!(-20.00), "AUD"))
        );
    }

    /// The old i64 minor-units wire format could not carry this; real exports do.
    #[test]
    fn import_carries_an_eighteen_decimal_amount_above_the_old_ceiling() {
        let csv = "Date,Coin,Change\n2025-01-02,ETH,123.456789012345678\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Change".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Coin".to_owned()),
            }),
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        let amount = txs[0].postings[0].amount.as_ref().expect("has an amount");
        assert_eq!(amount.value.to_string(), "123.456789012345678");
    }

    /// A split column addressed by a name absent from the header is a
    /// configuration error, not an empty cell that collapses to "neither".
    #[test]
    fn parse_bytes_errors_when_a_split_column_name_is_absent() {
        let importer = CsvImporter;
        let cfg = Config {
            account: "Liabilities:Bank:Card".to_owned(),
            header: Header::Present,
            date_column: ColumnRef::Name("Date".to_owned()),
            date_format: "%d/%m/%Y".to_owned(),
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Name("Debit".to_owned()),
                credit_column: ColumnRef::Name("Missing".to_owned()),
            },
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };

        let result = importer.parse_bytes(b"Date,Debit\n01/02/2025,33.03\n", &cfg, "statement.csv");
        let err = result.expect_err("the credit column is not in the header");
        assert_eq!(err.to_string(), "missing required field: 'Missing'");
    }
}
