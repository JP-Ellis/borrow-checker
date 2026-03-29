//! Configuration types for the CSV importer.

/// Describes how many lines of preamble metadata precede the CSV header row.
///
/// Many bank CSV exports include metadata rows before the actual column headers.
/// This enum lets callers declare the strategy for skipping those lines.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum Preamble {
    /// First line is the header row — no preamble.
    None,
    /// Skip exactly `lines` lines before the header.
    SkipLines {
        /// The number of lines to skip.
        lines: u32,
    },
    /// Scan from the top; first line whose CSV fields all match the configured
    /// column names (case-insensitive) is the header. Lines before it are discarded.
    AutoDetect {
        /// Maximum number of lines to scan before giving up.
        max_scan_lines: u32,
    },
}

impl Default for Preamble {
    #[inline]
    fn default() -> Self {
        Self::None
    }
}

/// Describes how monetary amounts are represented in the CSV.
///
/// Some files use a single signed column; others use separate debit and credit
/// columns where both values are always positive.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "style", rename_all = "snake_case")]
pub enum AmountColumns {
    /// Single signed column: positive = credit (in), negative = debit (out).
    Single {
        /// The name of the amount column.
        column: String,
    },
    /// Separate columns; both always positive; exactly one populated per row.
    SplitDebitCredit {
        /// The name of the debit (money out) column.
        debit_column: String,
        /// The name of the credit (money in) column.
        credit_column: String,
    },
}

impl Default for AmountColumns {
    #[inline]
    fn default() -> Self {
        Self::Single {
            column: "Amount".into(),
        }
    }
}

/// Full configuration for the CSV importer.
///
/// Supports configurable column names, delimiters, date formats, amount
/// representations, and preamble handling for bank-style CSV exports.
///
/// Construct using [`Config::builder()`] or [`Config::default()`].
#[non_exhaustive]
#[derive(bon::Builder, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// How to skip over metadata lines before the header row.
    #[serde(default)]
    #[builder(default)]
    pub preamble: Preamble,
    /// The field delimiter character.
    #[serde(default = "default_delimiter")]
    #[builder(default = default_delimiter())]
    pub delimiter: char,
    /// The name of the column containing the transaction date.
    #[serde(default = "default_date_column")]
    #[builder(default = default_date_column())]
    pub date_column: String,
    /// The date format string (jiff `strptime` syntax, e.g. `"%Y-%m-%d"`).
    #[serde(default = "default_date_format")]
    #[builder(default = default_date_format())]
    pub date_format: String,
    /// Which column(s) hold the monetary amount.
    #[serde(default)]
    #[builder(default)]
    pub amount_columns: AmountColumns,
    /// Optional column name for the payee or merchant.
    pub payee_column: Option<String>,
    /// Optional column name for the free-text description.
    pub description_column: Option<String>,
    /// Optional column name for an institution-supplied reference number.
    pub reference_column: Option<String>,
    /// Optional column name for the running balance after each transaction.
    pub balance_column: Option<String>,
    /// Commodity code (e.g. `"AUD"`). Required when the file does not contain one.
    pub commodity: Option<String>,
    /// The character used as the decimal separator in numeric fields.
    #[serde(default = "default_decimal_separator")]
    #[builder(default = default_decimal_separator())]
    pub decimal_separator: char,
    /// Optional thousands-separator character to strip from numeric fields.
    pub thousands_separator: Option<char>,
}

/// Returns the default field delimiter.
#[inline]
fn default_delimiter() -> char {
    ','
}

/// Returns the default date column name.
#[inline]
fn default_date_column() -> String {
    "Date".into()
}

/// Returns the default date format string.
#[inline]
fn default_date_format() -> String {
    "%Y-%m-%d".into()
}

/// Returns the default decimal separator character.
#[inline]
fn default_decimal_separator() -> char {
    '.'
}

impl Default for Config {
    #[inline]
    fn default() -> Self {
        Self {
            preamble: Preamble::default(),
            delimiter: default_delimiter(),
            date_column: default_date_column(),
            date_format: default_date_format(),
            amount_columns: AmountColumns::default(),
            payee_column: None,
            description_column: None,
            reference_column: None,
            balance_column: None,
            commodity: None,
            decimal_separator: default_decimal_separator(),
            thousands_separator: None,
        }
    }
}

impl Config {
    /// Returns the column names that are required to identify the CSV header row.
    ///
    /// Used by the [`Preamble::AutoDetect`] strategy to locate the header line.
    /// Always includes the date column and at least one amount column.
    ///
    /// # Returns
    ///
    /// A `Vec` of column name string slices.
    #[must_use]
    #[inline]
    pub fn required_column_names(&self) -> Vec<&str> {
        let mut cols = vec![self.date_column.as_str()];
        match &self.amount_columns {
            AmountColumns::Single { column } => cols.push(column.as_str()),
            AmountColumns::SplitDebitCredit {
                debit_column,
                credit_column,
            } => {
                cols.push(debit_column.as_str());
                cols.push(credit_column.as_str());
            }
        }
        cols
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn default_config_has_expected_values() {
        let cfg = Config::default();
        assert_eq!(cfg.delimiter, ',');
        assert_eq!(cfg.date_column, "Date");
        assert_eq!(cfg.date_format, "%Y-%m-%d");
        assert_eq!(cfg.decimal_separator, '.');
        assert!(cfg.thousands_separator.is_none());
        assert!(cfg.commodity.is_none());
    }

    #[test]
    fn required_column_names_single() {
        let cfg = Config::default();
        let cols = cfg.required_column_names();
        assert_eq!(cols, vec!["Date", "Amount"]);
    }

    #[test]
    fn required_column_names_split() {
        let cfg = Config {
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: "Debit".into(),
                credit_column: "Credit".into(),
            },
            ..Config::default()
        };
        let cols = cfg.required_column_names();
        assert_eq!(cols, vec!["Date", "Debit", "Credit"]);
    }

    #[test]
    fn preamble_default_is_none() {
        assert_eq!(Preamble::default(), Preamble::None);
    }

    #[test]
    fn amount_columns_default_is_single_amount() {
        assert_eq!(
            AmountColumns::default(),
            AmountColumns::Single {
                column: "Amount".into()
            }
        );
    }
}
