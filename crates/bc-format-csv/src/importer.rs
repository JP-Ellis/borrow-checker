//! [`CsvImporter`] — the main entry point for CSV-format imports.

use std::collections::HashMap;

use bc_models::Amount;
use bc_models::CommodityCode;
use rust_decimal::Decimal;

use crate::config::AmountColumns;
use crate::config::CsvConfig;
use crate::preamble::find_csv_start;

/// Imports transactions from delimited text (CSV) files.
///
/// Implements [`bc_core::Importer`] and is registered under the name `"csv"`.
/// Configuration is provided via a [`CsvConfig`] JSON blob.
#[non_exhaustive]
#[derive(Debug, Default)]
#[expect(
    clippy::module_name_repetitions,
    reason = "CsvImporter is the canonical public name; the module is implementation-private"
)]
pub struct CsvImporter;

impl CsvImporter {
    /// Creates a new [`CsvImporter`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl bc_core::Importer for CsvImporter {
    #[inline]
    fn name(&self) -> &'static str {
        "csv"
    }

    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        // Must be valid UTF-8 and the first non-empty line must contain a
        // delimiter character.
        // NOTE: delimiter detection is heuristic — detect() has no access to
        // the configured delimiter, so we accept any common delimiter character
        let Ok(text) = core::str::from_utf8(bytes) else {
            return false;
        };
        text.lines()
            .find(|l| !l.trim().is_empty())
            .is_some_and(|first| first.contains([',', '\t', ';', '|']))
    }

    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "the import function coordinates several sequential parsing steps; splitting would harm readability"
    )]
    fn import(
        &self,
        bytes: &[u8],
        config: &bc_core::ImportConfig,
    ) -> Result<Vec<bc_core::RawTransaction>, bc_core::ImportError> {
        let cfg: CsvConfig = config.as_typed()?;

        if !cfg.delimiter.is_ascii() {
            return Err(bc_core::ImportError::BadValue {
                field: "delimiter".to_owned(),
                detail: format!(
                    "delimiter must be a single printable ASCII character, got {:?}",
                    cfg.delimiter
                ),
            });
        }

        let required = cfg.required_column_names();
        let csv_bytes = find_csv_start(bytes, &cfg.preamble, cfg.delimiter, &required)?;

        // The `as` cast is safe: delimiter is ASCII, confirmed by the guard above.
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
            .map_err(|e| bc_core::ImportError::Parse(e.to_string()))?
            .clone();

        let col_index: HashMap<String, usize> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| (h.to_ascii_lowercase(), i))
            .collect();

        let lookup = |name: &str| -> Result<usize, bc_core::ImportError> {
            col_index
                .get(&name.to_ascii_lowercase())
                .copied()
                .ok_or_else(|| bc_core::ImportError::MissingField(name.to_owned()))
        };

        let date_idx = lookup(&cfg.date_column)?;

        let commodity = cfg
            .commodity
            .as_deref()
            .ok_or_else(|| bc_core::ImportError::BadValue {
                field: "commodity".to_owned(),
                detail: "commodity must be set in config when the file does not contain a currency column".to_owned(),
            })?;

        let mut transactions = Vec::new();

        for result in reader.records() {
            let record = result.map_err(|e| bc_core::ImportError::Parse(e.to_string()))?;

            let date_str = record_field(&record, date_idx, &cfg.date_column)?;
            let date = jiff::civil::Date::strptime(&cfg.date_format, date_str).map_err(|e| {
                bc_core::ImportError::BadValue {
                    field: cfg.date_column.clone(),
                    detail: e.to_string(),
                }
            })?;

            let amount_value = parse_amount(&cfg, &record, &col_index)?;

            let amount = Amount::new(amount_value, CommodityCode::new(commodity));

            let balance = if let Some(col) = &cfg.balance_column {
                let &idx = col_index
                    .get(&col.to_ascii_lowercase())
                    .ok_or_else(|| bc_core::ImportError::MissingField(col.clone()))?;
                let raw_owned = record_field(&record, idx, col)?;
                let raw = raw_owned.trim();
                if raw.is_empty() {
                    None
                } else {
                    let val = parse_number(raw, cfg.decimal_separator, cfg.thousands_separator)
                        .map_err(|e| bc_core::ImportError::BadValue {
                            field: col.clone(),
                            detail: e,
                        })?;
                    Some(Amount::new(val, CommodityCode::new(commodity)))
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

            transactions.push(bc_core::RawTransaction::new(
                date,
                amount,
                balance,
                payee,
                description,
                reference,
            ));
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
/// A string slice of the field value.
///
/// # Errors
///
/// Returns [`bc_core::ImportError::MissingField`] if `idx` is out of range.
#[inline]
fn record_field(
    record: &csv::StringRecord,
    idx: usize,
    column_name: &str,
) -> Result<String, bc_core::ImportError> {
    record
        .get(idx)
        .map(str::to_owned)
        .ok_or_else(|| bc_core::ImportError::MissingField(column_name.to_owned()))
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
/// Returns [`bc_core::ImportError`] if the column is missing or the value
/// cannot be parsed.
fn parse_amount(
    cfg: &CsvConfig,
    record: &csv::StringRecord,
    col_index: &HashMap<String, usize>,
) -> Result<Decimal, bc_core::ImportError> {
    match &cfg.amount_columns {
        AmountColumns::Single { column } => {
            let idx = col_index
                .get(&column.to_ascii_lowercase())
                .copied()
                .ok_or_else(|| bc_core::ImportError::MissingField(column.clone()))?;
            let raw = record_field(record, idx, column)?;
            parse_number(&raw, cfg.decimal_separator, cfg.thousands_separator).map_err(|e| {
                bc_core::ImportError::BadValue {
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
                        .map_err(|e| bc_core::ImportError::BadValue {
                            field: debit_column.clone(),
                            detail: e,
                        })?;
                    // Negate: a positive debit figure means money going out.
                    #[expect(
                        clippy::arithmetic_side_effects,
                        reason = "Decimal negation cannot overflow"
                    )]
                    Ok(-val)
                }
                (None, Some(c)) => parse_number(c, cfg.decimal_separator, cfg.thousands_separator)
                    .map_err(|e| bc_core::ImportError::BadValue {
                        field: credit_column.clone(),
                        detail: e,
                    }),
                (Some(_), Some(_)) => Err(bc_core::ImportError::Parse(format!(
                    "both '{debit_column}' and '{credit_column}' are populated in the same row"
                ))),
                (None, None) => Err(bc_core::ImportError::MissingField(format!(
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
fn parse_number(
    raw: &str,
    decimal_sep: char,
    thousands_sep: Option<char>,
) -> Result<Decimal, String> {
    // Strip leading/trailing whitespace, then separate any leading minus sign
    // before trimming currency prefixes/suffixes.  `trim_matches` scans both
    // ends simultaneously, so a leading `−` stops it from ever reaching a `$`.
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
    use bc_core::Importer as _;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    #[test]
    fn detect_returns_true_for_csv_bytes() {
        let importer = CsvImporter::new();
        assert!(importer.detect(b"Date,Amount,Description\n"));
    }

    #[test]
    fn detect_returns_false_for_non_csv() {
        let importer = CsvImporter::new();
        assert!(!importer.detect(b"\x89PNG\r\n"));
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
}
