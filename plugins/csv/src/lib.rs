//! CSV importer plugin for BorrowChecker.
//!
//! Implements the [`bc_sdk::Importer`] trait for delimited text (CSV) files.
//! Apply `#[bc_sdk::importer]` to the `impl Importer for CsvImporter` block
//! to generate the required WASM export glue.

mod config;
mod glob;
mod header;
mod preamble;

use bc_sdk::Amount;
use bc_sdk::Date;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::MetaEntry;
use bc_sdk::MetaValue;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use bc_sdk::SourceLocation;
use rust_decimal::Decimal;

use crate::config::AmountColumns;
use crate::config::ColumnRef;
use crate::config::CommoditySource;
use crate::config::Config;
use crate::config::Header;
use crate::config::MetaColumnType;
use crate::header::HeaderMap;
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
            .flexible(true)
            .trim(csv::Trim::All)
            .from_reader(csv_bytes);

        // The header row's own record, cloned because `Reader::headers`
        // borrows the reader mutably and the row loop needs it afterwards.
        let header_record = if has_header_row {
            Some(
                reader
                    .headers()
                    .map_err(|e| ImportError::Parse(e.to_string()))?
                    .clone(),
            )
        } else {
            None
        };

        let (columns, warnings) = HeaderMap::build(header_record.as_ref(), &cfg.column_refs())?;
        for detail in warnings {
            bc_sdk::warn!("duplicate column name in header"; path = file, detail = detail);
        }

        let date_idx = columns.resolve(&cfg.date_column)?;

        let commodity_source = cfg
            .commodity
            .as_ref()
            .ok_or_else(|| ImportError::BadValue {
                field: "commodity".to_owned(),
                detail: "commodity is not set; give a code or a column".to_owned(),
            })?;

        let mut transactions = Vec::new();

        // The file's data-row width, learned from its first data row rather
        // than from the header: a header carrying trailing prose is wider than
        // every row, and trusting it would flag the whole file instead of the
        // one malformed line.
        let mut expected_width: Option<usize> = None;

        // The widest data row in the file, which is the only exact evidence
        // that a column index is beyond *every* row.
        let mut widest_row = 0_usize;

        for (row_idx, result) in reader.records().enumerate() {
            let row = row_idx.saturating_add(1);
            let record = result.map_err(|e| ImportError::Parse(e.to_string()))?;
            widest_row = widest_row.max(record.len());

            let expected = match expected_width {
                None => {
                    expected_width = Some(record.len());
                    if let Some(detail) =
                        crate::header::header_width_warning(columns.width(), record.len())
                    {
                        bc_sdk::warn!("csv header width differs from data rows"; path = file, detail = detail);
                    }
                    record.len()
                }
                Some(expected) => {
                    if let Some(detail) =
                        crate::header::row_width_warning(expected, record.len(), row)
                    {
                        bc_sdk::warn!("ragged csv row"; path = file, detail = detail);
                    }
                    expected
                }
            };

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

            let commodity = row_commodity(commodity_source, &record, &columns, expected)?;

            let amount_value = parse_amount(&cfg.amount_columns, cfg, &record, &columns, expected)?
                .ok_or_else(|| match cfg.amount_columns {
                    AmountColumns::SplitDebitCredit {
                        ref debit_column,
                        ref credit_column,
                    } => ImportError::MissingField(format!(
                        "{} or {}",
                        debit_column.describe(),
                        credit_column.describe()
                    )),
                    AmountColumns::Single { ref column } => {
                        ImportError::MissingField(column.describe())
                    }
                })?;

            let balance = if let Some(column) = cfg.balance_column.as_ref() {
                match optional_text(&record, &columns, Some(column), expected)? {
                    None => None,
                    Some(raw) => {
                        let val =
                            parse_number(&raw, cfg.decimal_separator, cfg.thousands_separator)
                                .map_err(|e| ImportError::BadValue {
                                    field: column.describe(),
                                    detail: e,
                                })?;
                        Some(Amount::new(val, commodity.clone()))
                    }
                }
            } else {
                None
            };

            let metadata = row_metadata(cfg, &record, &columns, expected, &commodity, row)?;
            let description =
                optional_text(&record, &columns, cfg.description_column.as_ref(), expected)?
                    .unwrap_or_default();
            let reference =
                optional_text(&record, &columns, cfg.reference_column.as_ref(), expected)?;

            let mut postings = vec![
                RawPosting::builder()
                    .account(cfg.account.clone())
                    .amount(Amount::new(amount_value, commodity.clone()))
                    .maybe_balance(balance)
                    .build(),
            ];

            for leg in &cfg.extra_legs {
                let Some(value) =
                    parse_amount(&leg.amount_columns, cfg, &record, &columns, expected)?
                else {
                    // A blank cell, or a row too short to reach the column,
                    // means this leg is absent for this row, which is the
                    // normal shape of a fee column.
                    continue;
                };
                let code = match row_commodity(&leg.commodity, &record, &columns, expected) {
                    Ok(code) => code,
                    // A blank or short-row cell means this row does not name
                    // the leg's commodity, which is a per-row omission. An
                    // unresolvable column reference, or one beyond every row,
                    // is a profile that does not match the file, and would drop
                    // the leg from every row silently.
                    Err(e @ ImportError::BadValue { .. }) => {
                        bc_sdk::warn!(
                            "extra leg has an amount but no commodity; dropping the leg";
                            account = leg.account.clone(),
                            row = row.to_string(),
                            reason = e.to_string()
                        );
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let value = if leg.negate { -value } else { value };
                postings.push(
                    RawPosting::builder()
                        .account(leg.account.clone())
                        .amount(Amount::new(value, code))
                        .build(),
                );
            }

            transactions.push(
                RawTransaction::builder()
                    .date(date)
                    .description(description)
                    .metadata(metadata)
                    .maybe_reference(reference)
                    .source_location(
                        SourceLocation::builder()
                            .display(format!("{file} data row {row}"))
                            .build(),
                    )
                    .postings(postings)
                    .build(),
            );
        }

        // A file with no data rows says nothing about which columns it carries,
        // so there is nothing to hold the profile against.
        if expected_width.is_some() {
            unreachable_names(cfg, &columns, widest_row)?;
        }

        Ok(transactions)
    }
}

/// Rejects name-addressed columns that no data row in the file reaches.
///
/// A name resolves against the header, which a trailing-prose line makes wider
/// than every data row, so a name can denote a column the data never carries.
/// [`cell`] cannot catch that per row — a short row is a per-row omission — and
/// the width it works from is learned from one possibly-unrepresentative row.
/// The widest row in the file is exact, so the check [`cell`] applies eagerly to
/// a positional reference lands here for a name.
///
/// # Arguments
///
/// * `cfg` - The CSV import configuration, for its column references.
/// * `columns` - The file's header map.
/// * `widest_row` - The field count of the widest data row in the file.
///
/// # Returns
///
/// `Ok(())` when every name-addressed column is within the widest row.
///
/// # Errors
///
/// Returns [`ImportError::BadValue`] naming the first configuration field whose
/// column no row reaches, because the profile does not match the file.
fn unreachable_names(
    cfg: &Config,
    columns: &HeaderMap,
    widest_row: usize,
) -> Result<(), ImportError> {
    for (field, column) in cfg.column_refs() {
        if column.as_name().is_none() {
            continue;
        }
        let idx = columns.resolve(column)?;
        if idx >= widest_row {
            return Err(ImportError::BadValue {
                field: field.into_owned(),
                detail: format!(
                    "the header names {} at column {idx}, but no data row is that \
                     wide; the profile does not match the file",
                    column.describe()
                ),
            });
        }
    }
    Ok(())
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

/// Returns the trimmed text of a configured column, or `None` when it is absent
/// from this row or blank.
///
/// This is the single short-row policy every column whose absence is tolerable
/// obeys, so that "absent from this row" and "absent from every row" mean the
/// same thing wherever such a cell is read. The date column reads through
/// [`record_field`] instead, because a row that names no date cannot become a
/// transaction at all.
///
/// A column absent from *this* row because the row is short is a per-row
/// omission and yields `None`, indistinguishable from a blank cell. A
/// positional reference whose index lies beyond the file's data-row width is
/// absent from *every* row, which means the profile does not match the file,
/// and is an error. A name reference is exempt from that check, because the
/// width comes from one possibly-unrepresentative row and a name that resolved
/// against the header is evidence against overruling it; the exact form of the
/// same check runs over the whole file in [`unreachable_names`].
///
/// # Arguments
///
/// * `record` - The CSV record to read from.
/// * `columns` - The file's header map.
/// * `column` - The column reference to read.
/// * `expected` - The file's data-row width.
///
/// # Returns
///
/// The trimmed cell text, or `None` when the cell is absent from this row or
/// blank.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] when the reference does not resolve,
/// or when a positional reference resolves beyond the file's data-row width.
#[inline]
fn cell<'record>(
    record: &'record csv::StringRecord,
    columns: &HeaderMap,
    column: &ColumnRef,
    expected: usize,
) -> Result<Option<&'record str>, ImportError> {
    let idx = columns.resolve(column)?;
    // A name that resolved came from the header, which is itself proof the
    // profile matches the file. Only a positional reference needs the width
    // guard.
    if column.as_name().is_none() && idx >= expected {
        return Err(ImportError::MissingField(column.describe()));
    }
    let Some(raw) = record.get(idx) else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    Ok((!trimmed.is_empty()).then_some(trimmed))
}

/// Returns the trimmed text of an optional column, or `None` when it is absent
/// or blank.
///
/// A thin wrapper over [`cell`] that also accepts an unconfigured field.
///
/// # Arguments
///
/// * `record` - The CSV record to read from.
/// * `columns` - The file's header map.
/// * `column` - The column reference, or `None` when the field is unconfigured.
/// * `expected` - The file's data-row width.
///
/// # Returns
///
/// The trimmed cell text, or `None` when unconfigured, absent, or blank.
///
/// # Errors
///
/// Returns whatever [`cell`] returns for a configured column.
#[inline]
fn optional_text(
    record: &csv::StringRecord,
    columns: &HeaderMap,
    column: Option<&ColumnRef>,
    expected: usize,
) -> Result<Option<String>, ImportError> {
    let Some(column) = column else {
        return Ok(None);
    };
    Ok(cell(record, columns, column, expected)?.map(str::to_owned))
}

/// Builds one row's metadata from the profile's column mappings.
///
/// A blank or absent cell states nothing, so it yields no entry. A populated
/// cell that will not carry the type the profile claims for it is filed as
/// text and warned about: an annotation is worth less than the row, so a
/// malformed one must not cost the row its amount. The host then reads that
/// text against the key's registered type and flags it for repair.
///
/// # Arguments
///
/// * `cfg` - The profile, for its mappings and numeric and date formats.
/// * `record` - The CSV record being read.
/// * `columns` - The file's header map.
/// * `expected` - The file's data-row width.
/// * `commodity` - The row's commodity, which an `amount` column posts in.
/// * `row` - The 1-based data-row number, for diagnostics.
///
/// # Returns
///
/// One entry per populated mapped column, in the order the profile states
/// them.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] when a mapped column does not resolve
/// or lies beyond every row. That is a profile that does not match the file,
/// not one row's omission.
fn row_metadata(
    cfg: &Config,
    record: &csv::StringRecord,
    columns: &HeaderMap,
    expected: usize,
    commodity: &str,
    row: usize,
) -> Result<Vec<MetaEntry>, ImportError> {
    let mut entries = Vec::with_capacity(cfg.metadata_columns.len());
    for mapping in &cfg.metadata_columns {
        let Some(raw) = cell(record, columns, &mapping.column, expected)? else {
            continue;
        };
        let value = match meta_value(mapping.ty, raw, cfg, commodity) {
            Ok(value) => value,
            Err(detail) => {
                bc_sdk::warn!(
                    "metadata cell does not carry the type the profile states; filing it as text";
                    key = mapping.key.clone(),
                    row = row.to_string(),
                    detail = detail
                );
                MetaValue::Text(raw.to_owned())
            }
        };
        entries.push(MetaEntry::new(mapping.key.clone(), value));
    }
    Ok(entries)
}

/// Reads one cell as the type a profile states for its column.
///
/// # Arguments
///
/// * `ty` - The stated type.
/// * `raw` - The trimmed cell text, known to be non-empty.
/// * `cfg` - The profile, for its numeric separators and date format.
/// * `commodity` - The row's commodity, which an `amount` cell posts in.
///
/// # Returns
///
/// The typed value.
///
/// # Errors
///
/// Returns the reason the cell does not carry `ty`, for the caller to log.
fn meta_value(
    ty: MetaColumnType,
    raw: &str,
    cfg: &Config,
    commodity: &str,
) -> Result<MetaValue, String> {
    match ty {
        MetaColumnType::Text => Ok(MetaValue::Text(raw.to_owned())),
        MetaColumnType::Account => Ok(MetaValue::Account(raw.to_owned())),
        MetaColumnType::Number => {
            parse_number(raw, cfg.decimal_separator, cfg.thousands_separator).map(MetaValue::Number)
        }
        MetaColumnType::Amount => parse_number(raw, cfg.decimal_separator, cfg.thousands_separator)
            .map(|value| MetaValue::Amount(Amount::new(value, commodity.to_owned()))),
        MetaColumnType::Boolean => match raw.to_ascii_lowercase().as_str() {
            "true" => Ok(MetaValue::Boolean(true)),
            "false" => Ok(MetaValue::Boolean(false)),
            _other => Err(format!("'{raw}' is neither 'true' nor 'false'")),
        },
        MetaColumnType::Date => jiff::civil::Date::strptime(&cfg.date_format, raw)
            .map(|parsed| {
                MetaValue::Date(Date::new(
                    i32::from(parsed.year()),
                    parsed.month().unsigned_abs(),
                    parsed.day().unsigned_abs(),
                ))
            })
            .map_err(|e| e.to_string()),
        // The host takes an unparsable timestamp as the plugin's own defect
        // and fails the whole file, so a cell that is not RFC 3339 has to be
        // caught here, where it costs one annotation instead of the import.
        MetaColumnType::Timestamp => raw
            .parse::<jiff::Timestamp>()
            .map(|_parsed| MetaValue::Timestamp(raw.to_owned()))
            .map_err(|e| e.to_string()),
    }
}

/// Resolves the commodity code that applies to one row.
///
/// # Arguments
///
/// * `source` - Where the code comes from.
/// * `record` - The CSV record being processed.
/// * `columns` - The header map, for resolving column references.
/// * `expected` - The file's data-row width, for [`cell`]'s short-row policy.
///
/// # Returns
///
/// The commodity code for this row.
///
/// # Errors
///
/// Returns [`ImportError::MissingField`] when a column reference does not
/// resolve or lies beyond every row, or [`ImportError::BadValue`] when a
/// column-sourced cell is blank or absent from this row — a `Column` source
/// asserts that every row names its commodity, and callers that tolerate a
/// per-row omission match on `BadValue`.
#[inline]
fn row_commodity(
    source: &CommoditySource,
    record: &csv::StringRecord,
    columns: &HeaderMap,
    expected: usize,
) -> Result<String, ImportError> {
    match *source {
        CommoditySource::Fixed { ref code } => Ok(code.clone()),
        CommoditySource::Column { ref column } => cell(record, columns, column, expected)?
            .map(str::to_owned)
            .ok_or_else(|| ImportError::BadValue {
                field: column.describe(),
                detail: "the commodity cell is blank or missing from this row; every row \
                         must name its commodity"
                    .to_owned(),
            }),
    }
}

/// Parses the monetary amount for one leg from a record, using the given
/// amount-column strategy.
///
/// # Arguments
///
/// * `amount_columns` - The amount-column strategy to apply — the primary
///   leg's `cfg.amount_columns` or an extra leg's `LegSpec::amount_columns`.
/// * `cfg` - The CSV import configuration, for number-formatting settings.
/// * `record` - The CSV record being processed.
/// * `columns` - The header map, for resolving column references.
/// * `expected` - The file's data-row width, for [`cell`]'s short-row policy.
///
/// # Returns
///
/// `Ok(Some(value))` with the parsed [`Decimal`] value, debits negated, when
/// at least one configured cell is populated. `Ok(None)` when every
/// configured cell for these columns is blank or absent from this row — the
/// normal shape of a leg that is absent for this row (e.g. an unfee'd trade,
/// or a ragged export that drops its trailing fee column). A required leg
/// turns that `None` into an error at its call site; an extra leg skips.
///
/// # Errors
///
/// Returns [`ImportError`] if a configured column does not resolve, lies
/// beyond every row, or a populated cell cannot be parsed as a number.
#[inline]
fn parse_amount(
    amount_columns: &AmountColumns,
    cfg: &Config,
    record: &csv::StringRecord,
    columns: &HeaderMap,
    expected: usize,
) -> Result<Option<Decimal>, ImportError> {
    match *amount_columns {
        AmountColumns::Single { ref column } => {
            let Some(trimmed) = cell(record, columns, column, expected)? else {
                return Ok(None);
            };
            parse_number(trimmed, cfg.decimal_separator, cfg.thousands_separator)
                .map(Some)
                .map_err(|e| ImportError::BadValue {
                    field: column.describe(),
                    detail: e,
                })
        }
        AmountColumns::SplitDebitCredit {
            ref debit_column,
            ref credit_column,
        } => {
            // Both references must resolve and lie within the file; only the
            // *cells* may be empty, since exactly one of the pair is populated
            // per row.
            let debit_raw = cell(record, columns, debit_column, expected)?;
            let credit_raw = cell(record, columns, credit_column, expected)?;

            match (debit_raw, credit_raw) {
                (Some(d), None) => {
                    let val = parse_number(d, cfg.decimal_separator, cfg.thousands_separator)
                        .map_err(|e| ImportError::BadValue {
                            field: debit_column.describe(),
                            detail: e,
                        })?;
                    // Negate: a positive debit figure means money going out.
                    Ok(Some(-val))
                }
                (None, Some(c)) => parse_number(c, cfg.decimal_separator, cfg.thousands_separator)
                    .map(Some)
                    .map_err(|e| ImportError::BadValue {
                        field: credit_column.describe(),
                        detail: e,
                    }),
                (Some(_), Some(_)) => Err(ImportError::Parse(format!(
                    "both {} and {} are populated in the same row",
                    debit_column.describe(),
                    credit_column.describe()
                ))),
                (None, None) => Ok(None),
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
    use bc_sdk::MetaValue;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::config::LegSpec;
    use crate::config::MetaColumnType;
    use crate::config::MetadataColumn;

    /// Reads the first `payee` metadata entry, when the row states one.
    fn payee_of(tx: &RawTransaction) -> Option<&str> {
        tx.metadata.iter().find_map(|entry| match entry.value {
            MetaValue::Text(ref text) if entry.key == "payee" => Some(text.as_str()),
            _ => None,
        })
    }

    /// Maps `column` onto the `payee` key, as most of these tests once did
    /// through the retired `payee_column` field.
    fn payee_map(column: ColumnRef) -> MetadataColumn {
        MetadataColumn {
            key: "payee".to_owned(),
            column,
            ty: MetaColumnType::Text,
        }
    }

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
            "metadata_columns": [{"key": "payee", "column": "Payee"}]
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
        assert_eq!(payee_of(t0), Some("Java Hut"));

        let t1 = &txns[1];
        assert_eq!(t1.date, Date::new(2025, 3, 16));
        assert_eq!(t1.postings.len(), 1);
        assert_eq!(t1.postings[0].account, "Assets:Bank:Checking");
        assert_eq!(
            t1.postings[0].amount,
            Some(Amount::new(dec!(-120.00), "AUD"))
        );
        assert_eq!(t1.description, "Groceries");
        assert_eq!(payee_of(t1), None);
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
            "metadata_columns": [{"key": "payee", "column": "Payee"}]
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

    /// A three-column headerless profile whose third column carries `ty`
    /// under the key `thing`.
    fn typed_third_column_config(ty: crate::config::MetaColumnType) -> Config {
        Config {
            metadata_columns: vec![crate::config::MetadataColumn {
                key: "thing".to_owned(),
                column: ColumnRef::Index(2),
                ty,
            }],
            ..headerless_two_column_config()
        }
    }

    /// Imports one row and returns the value filed under `thing`.
    fn typed_cell(ty: crate::config::MetaColumnType, cell: &str) -> bc_sdk::MetaValue {
        let cfg = typed_third_column_config(ty);
        let row = format!("01/02/2025,120.00,{cell}\n");
        let txs = CsvImporter
            .parse_bytes(row.as_bytes(), &cfg, "statement.csv")
            .expect("the row should import");
        let tx = txs.first().expect("one transaction");
        tx.metadata
            .first()
            .expect("one metadata entry")
            .value
            .clone()
    }

    #[test]
    fn a_column_is_read_as_the_type_the_profile_states() {
        use crate::config::MetaColumnType as T;

        assert_eq!(
            typed_cell(T::Text, "Java Hut"),
            bc_sdk::MetaValue::Text("Java Hut".to_owned())
        );
        assert_eq!(
            typed_cell(T::Number, "1502.50"),
            bc_sdk::MetaValue::Number(dec!(1502.50))
        );
        assert_eq!(
            typed_cell(T::Boolean, "TRUE"),
            bc_sdk::MetaValue::Boolean(true)
        );
        assert_eq!(
            typed_cell(T::Date, "15/01/2026"),
            bc_sdk::MetaValue::Date(Date::new(2026, 1, 15))
        );
        assert_eq!(
            typed_cell(T::Timestamp, "2026-01-15T09:30:00Z"),
            bc_sdk::MetaValue::Timestamp("2026-01-15T09:30:00Z".to_owned())
        );
        assert_eq!(
            typed_cell(T::Amount, "42.00"),
            bc_sdk::MetaValue::Amount(Amount::new(dec!(42.00), "AUD"))
        );
        assert_eq!(
            typed_cell(T::Account, "Assets:Bank:Savings"),
            bc_sdk::MetaValue::Account("Assets:Bank:Savings".to_owned())
        );
    }

    /// An annotation is worth less than the row it annotates, so a cell that
    /// will not carry its stated type is filed as text. The host then reads
    /// that text against the key's registered type and flags it for repair.
    #[test]
    fn a_cell_that_will_not_carry_its_stated_type_is_filed_as_text() {
        assert_eq!(
            typed_cell(crate::config::MetaColumnType::Number, "not-a-number"),
            bc_sdk::MetaValue::Text("not-a-number".to_owned())
        );
    }

    /// The host reads a stated timestamp strictly and takes an unparsable one
    /// as the plugin's own defect, failing the whole file. The cell has to be
    /// read here so that one bad annotation costs only itself.
    #[test]
    fn a_timestamp_cell_that_is_not_rfc_3339_is_filed_as_text() {
        assert_eq!(
            typed_cell(crate::config::MetaColumnType::Timestamp, "15/01/2026"),
            bc_sdk::MetaValue::Text("15/01/2026".to_owned())
        );
    }

    #[test]
    fn a_blank_mapped_cell_states_nothing() {
        let cfg = typed_third_column_config(crate::config::MetaColumnType::Text);
        let txs = CsvImporter
            .parse_bytes(b"01/02/2025,120.00,\n", &cfg, "statement.csv")
            .expect("the row should import");
        assert_eq!(txs.first().expect("one transaction").metadata, vec![]);
    }

    #[test]
    fn mapped_columns_keep_the_order_the_profile_states() {
        let cfg = Config {
            metadata_columns: vec![
                payee_map(ColumnRef::Index(2)),
                crate::config::MetadataColumn {
                    key: "note".to_owned(),
                    column: ColumnRef::Index(3),
                    ty: crate::config::MetaColumnType::Text,
                },
            ],
            ..headerless_two_column_config()
        };
        let txs = CsvImporter
            .parse_bytes(
                b"01/02/2025,120.00,Java Hut,paid by card\n",
                &cfg,
                "statement.csv",
            )
            .expect("the row should import");
        let keys: Vec<&str> = txs
            .first()
            .expect("one transaction")
            .metadata
            .iter()
            .map(|e| e.key.as_str())
            .collect();
        assert_eq!(keys, vec!["payee", "note"]);
    }

    /// One cell can be filed under two keys: a mapping annotates rather than
    /// claiming a column, so the second mapping is not redundant.
    #[test]
    fn one_column_can_be_filed_under_two_keys() {
        let cfg = Config {
            metadata_columns: vec![
                payee_map(ColumnRef::Index(2)),
                crate::config::MetadataColumn {
                    key: "merchant".to_owned(),
                    column: ColumnRef::Index(2),
                    ty: crate::config::MetaColumnType::Text,
                },
            ],
            ..headerless_two_column_config()
        };
        let txs = CsvImporter
            .parse_bytes(b"01/02/2025,120.00,Java Hut\n", &cfg, "statement.csv")
            .expect("the row should import");
        let entries: Vec<(&str, &str)> = txs
            .first()
            .expect("one transaction")
            .metadata
            .iter()
            .filter_map(|entry| match entry.value {
                MetaValue::Text(ref text) => Some((entry.key.as_str(), text.as_str())),
                _ => None,
            })
            .collect();
        assert_eq!(
            entries,
            vec![("payee", "Java Hut"), ("merchant", "Java Hut")]
        );
    }

    #[test]
    fn parse_bytes_errors_on_out_of_range_payee_index() {
        let cfg = Config {
            metadata_columns: vec![payee_map(ColumnRef::Index(9))],
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
            metadata_columns: vec![payee_map(ColumnRef::Index(2))],
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            ..Config::default()
        };

        let txs = importer
            .parse_bytes(b"01/02/2025,120.00,\n", &cfg, "statement.csv")
            .expect("an empty payee cell is not an error");
        assert_eq!(txs.len(), 1);
        assert_eq!(payee_of(&txs[0]), None);
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

    /// A trade row: base quantity in, quote total out, fee in the quote asset.
    #[test]
    fn import_emits_a_posting_per_configured_leg() {
        let csv = "Date,Base,Quote,Quantity,Total,Fees\n\
                   2025-01-02,BTC,AUD,0.50000000,32000.00,25.00\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![
                LegSpec {
                    account: "Assets:Crypto:Exchange".to_owned(),
                    amount_columns: AmountColumns::Single {
                        column: ColumnRef::Name("Total".to_owned()),
                    },
                    commodity: CommoditySource::Column {
                        column: ColumnRef::Name("Quote".to_owned()),
                    },
                    negate: true,
                },
                LegSpec {
                    account: "Expenses:Fees".to_owned(),
                    amount_columns: AmountColumns::Single {
                        column: ColumnRef::Name("Fees".to_owned()),
                    },
                    commodity: CommoditySource::Column {
                        column: ColumnRef::Name("Quote".to_owned()),
                    },
                    negate: false,
                },
            ],
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        assert_eq!(txs[0].postings.len(), 3);
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(dec!(0.50000000), "BTC"))
        );
        assert_eq!(
            txs[0].postings[1].amount,
            Some(Amount::new(dec!(-32000.00), "AUD"))
        );
        assert_eq!(txs[0].postings[1].account, "Assets:Crypto:Exchange");
        assert_eq!(
            txs[0].postings[2].amount,
            Some(Amount::new(dec!(25.00), "AUD"))
        );
        assert_eq!(txs[0].postings[2].account, "Expenses:Fees");
    }

    /// A blank fee cell is the normal shape of a fee column, not an error.
    #[test]
    fn import_omits_an_extra_leg_whose_amount_cell_is_blank() {
        let csv = "Date,Base,Quote,Quantity,Fees\n\
                   2025-01-02,BTC,AUD,0.50000000,\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("Fees".to_owned()),
                },
                commodity: CommoditySource::Column {
                    column: ColumnRef::Name("Quote".to_owned()),
                },
                negate: false,
            }],
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        assert_eq!(txs[0].postings.len(), 1);
    }

    #[test]
    fn import_negates_an_extra_leg_when_configured() {
        let csv = "Date,Base,Quote,Quantity,Total\n\
                   2025-01-02,BTC,AUD,0.50000000,32000.00\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Assets:Crypto:Exchange".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("Total".to_owned()),
                },
                commodity: CommoditySource::Column {
                    column: ColumnRef::Name("Quote".to_owned()),
                },
                negate: true,
            }],
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("parses");
        assert_eq!(
            txs[0].postings[1].amount,
            Some(Amount::new(dec!(-32000.00), "AUD"))
        );
    }

    /// An extra leg with an amount but no commodity is malformed; it costs the
    /// leg, not the row.
    #[test]
    fn import_drops_an_extra_leg_with_a_blank_commodity_cell() {
        let csv = "Date,Base,Quote,Quantity,Fees\n\
                   2025-01-02,BTC,,0.50000000,25.00\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("Fees".to_owned()),
                },
                commodity: CommoditySource::Column {
                    column: ColumnRef::Name("Quote".to_owned()),
                },
                negate: false,
            }],
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("the row still parses");
        assert_eq!(txs[0].postings.len(), 1);
    }

    #[test]
    fn parses_a_header_carrying_trailing_prose() {
        // Broker A's export glues explanatory prose onto the last column name,
        // and the prose contains an unquoted comma: the header parses as 5
        // fields where every data row has 4. The last real column name is
        // corrupted too ("Balance <as at settlement date"), so the balance must
        // be addressed by index — mixed addressing, legal under Header::Present.
        let csv =
            b"Date,Description,Amount,Balance <as at settlement date, includes pending items>\n\
                    2025-03-15,Alpha Holdings purchase,-500.00,1234.56\n\
                    2025-03-16,Delta Corp dividend,25.00,1259.56\n";

        let cfg = Config {
            account: "Assets:BrokerA:123456789".to_owned(),
            date_column: ColumnRef::Name("Date".to_owned()),
            description_column: Some(ColumnRef::Name("Description".to_owned())),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Amount".to_owned()),
            },
            balance_column: Some(ColumnRef::Index(3)),
            ..Config::default()
        };

        let txns = CsvImporter
            .parse_bytes(csv, &cfg, "brokera.csv")
            .expect("a ragged header must not fail the file");

        assert_eq!(txns.len(), 2);
        assert_eq!(txns[0].date, Date::new(2025, 3, 15));
        assert_eq!(
            txns[0].postings[0].amount,
            Some(Amount::new(dec!(-500.00), "AUD"))
        );
        assert_eq!(
            txns[0].postings[0].balance,
            Some(Amount::new(dec!(1234.56), "AUD"))
        );
        assert_eq!(txns[1].description, "Delta Corp dividend");
        assert_eq!(
            txns[1].postings[0].balance,
            Some(Amount::new(dec!(1259.56), "AUD"))
        );
    }

    #[test]
    fn an_optional_column_absent_from_a_short_row_is_none() {
        // Under strict parsing a row could never be short, so this is new
        // territory rather than a regression. Warn, don't block.
        let csv = b"Date,Amount,Payee\n\
                    2025-03-15,50.00,Java Hut\n\
                    2025-03-16,-20.00\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            metadata_columns: vec![payee_map(ColumnRef::Name("Payee".to_owned()))],
            ..Config::default()
        };

        let txns = CsvImporter
            .parse_bytes(csv, &cfg, "short.csv")
            .expect("a short row must not fail the file");

        assert_eq!(txns.len(), 2);
        assert_eq!(payee_of(&txns[0]), Some("Java Hut"));
        assert_eq!(payee_of(&txns[1]), None);
    }

    #[test]
    fn a_ragged_first_row_sets_the_expected_width_wrongly() {
        // Stated limitation: the expected width is learned from the first data
        // row, so if that row is the outlier the rest of the file warns. The
        // per-row warnings make this loud rather than silent, and no known
        // export has this shape. Rows still parse; only the diagnostics suffer.
        let csv = b"Date,Amount,Payee\n\
                    2025-03-15,50.00\n\
                    2025-03-16,-20.00,Java Hut\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            metadata_columns: vec![payee_map(ColumnRef::Name("Payee".to_owned()))],
            ..Config::default()
        };

        let txns = CsvImporter
            .parse_bytes(csv, &cfg, "outlier.csv")
            .expect("rows still parse");

        assert_eq!(txns.len(), 2);
        assert_eq!(payee_of(&txns[0]), None);
        assert_eq!(payee_of(&txns[1]), Some("Java Hut"));
    }

    #[test]
    fn a_ragged_first_row_fails_a_positional_optional_column() {
        // The positional half of `a_ragged_first_row_sets_the_expected_width_wrongly`
        // on the identical file. A name resolved from the header proves the
        // profile matches the file, so the width guard is skipped and the rows
        // parse; a bare index has no such proof, so the too-narrow width learned
        // from the outlier first row makes column 2 look absent from every row.
        // The asymmetry is deliberate: exempting indices too would resurrect the
        // silent-wrong-data failure `an_optional_column_beyond_every_row_is_an_error`
        // pins.
        let csv = b"Date,Amount,Payee\n\
                    2025-03-15,50.00\n\
                    2025-03-16,-20.00,Java Hut\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            metadata_columns: vec![payee_map(ColumnRef::Index(2))],
            ..Config::default()
        };

        let err = CsvImporter
            .parse_bytes(csv, &cfg, "outlier.csv")
            .expect_err("a positional column beyond the learned width must fail");

        assert_eq!(err.to_string(), "missing required field: column 2");
    }

    #[test]
    fn an_optional_column_beyond_every_row_is_an_error() {
        // Absent from *one* row is a per-row omission; absent from *every* row
        // means the profile does not match the file. Degrading that to None
        // would resurrect the silent-wrong-data failure #397 is about.
        let csv = b"Date,Amount\n\
                    2025-03-15,50.00\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            metadata_columns: vec![payee_map(ColumnRef::Index(5))],
            ..Config::default()
        };

        let err = CsvImporter
            .parse_bytes(csv, &cfg, "narrow.csv")
            .expect_err("a column present in no row must fail");

        assert_eq!(err.to_string(), "missing required field: column 5");
    }

    #[test]
    fn a_named_column_beyond_every_row_is_an_error() {
        // The positional half of this is `an_optional_column_beyond_every_row_is_an_error`.
        // A name resolves against the header, which trailing prose makes wider
        // than the data, so a name can denote a column no row carries. Reading
        // it as a per-row omission would leave `payee` silently None for the
        // whole file — the silent-wrong-data failure #397 is about.
        let csv = b"Date,Amount,Notes prose, more prose\n\
                    2025-03-15,50.00,ok\n\
                    2025-03-16,-20.00,ok\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            metadata_columns: vec![payee_map(ColumnRef::Name("more prose".to_owned()))],
            ..Config::default()
        };

        let err = CsvImporter
            .parse_bytes(csv, &cfg, "prose.csv")
            .expect_err("a named column present in no row must fail");

        assert_eq!(
            err.to_string(),
            "bad value for field 'metadata_columns[0].column': the header names 'more prose' at \
             column 3, but no data row is that wide; the profile does not match the file"
        );
    }

    #[test]
    fn a_named_extra_leg_column_beyond_every_row_is_an_error() {
        // The same guard reaching an extra leg, where a silent None is worse
        // still: `parse_amount` returning None drops the leg from every row
        // without a diagnostic. Mirrors
        // `import_fails_when_extra_leg_split_columns_are_beyond_every_row` for
        // names.
        let csv = b"Date,Quantity,Fees prose, more prose\n\
                    2025-01-02,0.50000000,ok\n";

        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("more prose".to_owned()),
                },
                commodity: CommoditySource::Fixed {
                    code: "AUD".to_owned(),
                },
                negate: false,
            }],
            ..Config::default()
        };

        let err = CsvImporter
            .parse_bytes(csv, &cfg, "prose.csv")
            .expect_err("a named leg column present in no row must fail");

        assert!(
            err.to_string()
                .contains("extra_legs[0].amount_columns.column"),
            "the error must name the leg's field, got: {err}"
        );
    }

    #[test]
    fn a_duplicate_header_name_a_field_addresses_fails_the_import() {
        // #397's shape, end to end: the unit tests pin HeaderMap::build, this
        // pins the wiring that carries its error out of parse_bytes.
        let csv = b"Date,Balance,Income,Value,Income\n\
                    2025-03-15,1000.00,50.00,10.00,25.00\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Income".to_owned()),
            },
            ..Config::default()
        };

        let err = CsvImporter
            .parse_bytes(csv, &cfg, "duplicate.csv")
            .expect_err("an addressed duplicate must fail the file");

        assert_eq!(
            err.to_string(),
            "invalid import configuration: amount_columns.column names 'Income', which \
             appears at columns 2 and 4; address it by zero-based index instead"
        );
    }

    #[test]
    fn a_duplicate_header_name_no_field_addresses_still_imports() {
        // The other half of the wiring: an unaddressed duplicate only warns, so
        // a header carrying unnamed empty columns cannot reject a file.
        let csv = b"Date,,Amount,\n\
                    2025-03-15,,50.00,\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Amount".to_owned()),
            },
            ..Config::default()
        };

        let txns = CsvImporter
            .parse_bytes(csv, &cfg, "duplicate.csv")
            .expect("an unaddressed duplicate must not fail the file");

        assert_eq!(txns.len(), 1);
        assert_eq!(
            txns[0].postings[0].amount,
            Some(Amount::new(dec!(50.00), "AUD"))
        );
    }

    #[test]
    fn a_required_column_absent_from_a_short_row_is_an_error() {
        // A row that cannot yield an amount cannot become a posting.
        let csv = b"Date,Amount\n\
                    2025-03-15,50.00\n\
                    2025-03-16\n";

        let cfg = Config {
            account: "Assets:Bank:Checking".to_owned(),
            ..Config::default()
        };

        CsvImporter
            .parse_bytes(csv, &cfg, "short.csv")
            .expect_err("a row with no amount must fail");
    }

    /// A short row is morally identical to a blank cell for an extra leg: the
    /// leg is absent for this row only, which is the normal shape of a ragged
    /// export that drops its trailing fee column.
    #[test]
    fn import_omits_an_extra_leg_whose_amount_column_is_off_a_short_row() {
        let csv = "Date,Base,Quote,Quantity,Fees\n\
                   2025-01-02,BTC,AUD,0.50000000,25.00\n\
                   2025-01-03,BTC,AUD,0.25000000\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("Fees".to_owned()),
                },
                commodity: CommoditySource::Column {
                    column: ColumnRef::Name("Quote".to_owned()),
                },
                negate: false,
            }],
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("a short row must not fail the file");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].postings.len(), 2);
        assert_eq!(txs[1].postings.len(), 1, "the fee leg is absent, not fatal");
    }

    /// A commodity column off the end of *one* row is a per-row omission, and
    /// costs the leg rather than the file — the same treatment a blank cell
    /// gets.
    #[test]
    fn import_drops_an_extra_leg_whose_commodity_is_off_a_short_row() {
        let csv = "Date,Base,Quantity,Fees,Quote\n\
                   2025-01-02,BTC,0.50000000,25.00,AUD\n\
                   2025-01-03,BTC,0.25000000,25.00\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("Fees".to_owned()),
                },
                commodity: CommoditySource::Column {
                    column: ColumnRef::Name("Quote".to_owned()),
                },
                negate: false,
            }],
            ..Config::default()
        };
        let txs = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect("a short row must not fail the file");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].postings.len(), 2);
        assert_eq!(
            txs[1].postings.len(),
            1,
            "the fee leg is dropped, not fatal"
        );
    }

    /// The inverse of `import_omits_an_extra_leg_whose_amount_column_is_off_a_short_row`:
    /// split columns beyond *every* row would otherwise drop the leg from every
    /// row with no diagnostic, which is the failure #397 exists to kill.
    #[test]
    fn import_fails_when_extra_leg_split_columns_are_beyond_every_row() {
        let csv = "Date,Base,Quantity\n\
                   2025-01-02,BTC,0.50000000\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::SplitDebitCredit {
                    debit_column: ColumnRef::Index(5),
                    credit_column: ColumnRef::Index(6),
                },
                commodity: CommoditySource::Fixed {
                    code: "AUD".to_owned(),
                },
                negate: false,
            }],
            ..Config::default()
        };
        let err = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect_err("split columns present in no row must fail");
        assert_eq!(err.to_string(), "missing required field: column 5");
    }

    /// A blank cell omits one leg on one row, but a commodity column the file
    /// does not have is a profile that does not match the file: dropping the
    /// leg would silently discard it from every row.
    #[test]
    fn import_fails_when_an_extra_leg_commodity_column_is_absent() {
        let csv = "Date,Base,Quantity,Fees\n\
                   2025-01-02,BTC,0.50000000,25.00\n";
        let cfg = Config {
            account: "Assets:Crypto:Exchange".to_owned(),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Quantity".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Base".to_owned()),
            }),
            extra_legs: vec![LegSpec {
                account: "Expenses:Fees".to_owned(),
                amount_columns: AmountColumns::Single {
                    column: ColumnRef::Name("Fees".to_owned()),
                },
                commodity: CommoditySource::Column {
                    column: ColumnRef::Name("Quote".to_owned()),
                },
                negate: false,
            }],
            ..Config::default()
        };
        let err = CsvImporter
            .parse_bytes(csv.as_bytes(), &cfg, "test.csv")
            .expect_err("an unresolvable commodity column aborts the import");
        assert!(
            matches!(err, ImportError::MissingField(ref field) if field.contains("Quote")),
            "expected a MissingField naming Quote, got {err:?}"
        );
    }
}
