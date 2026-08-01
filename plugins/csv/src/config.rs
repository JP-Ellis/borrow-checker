//! Configuration types for the CSV importer.

/// How a column is addressed within a CSV row.
///
/// Deserializes from either a bare JSON string (matched case-insensitively
/// against the header row) or a bare non-negative JSON integer (a zero-based
/// physical position). Both forms are accepted wherever a column is named:
///
/// ```json
/// { "date_column": "Date" }
/// { "date_column": 0 }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnRef {
    /// Matched case-insensitively against the header row.
    Name(String),
    /// Zero-based physical position within the row.
    Index(usize),
}

impl ColumnRef {
    /// Returns the column name when this reference is name-based.
    ///
    /// # Returns
    ///
    /// `Some(name)` for [`ColumnRef::Name`], `None` for [`ColumnRef::Index`].
    #[must_use]
    #[inline]
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "additive only; wired into Config by a later task in this effort"
        )
    )]
    pub fn as_name(&self) -> Option<&str> {
        match *self {
            Self::Name(ref name) => Some(name.as_str()),
            Self::Index(_) => None,
        }
    }

    /// Returns a human-readable description for use in error messages.
    ///
    /// # Returns
    ///
    /// `'Date'` for a name reference, `column 3` for an index reference.
    #[must_use]
    #[inline]
    #[expect(
        dead_code,
        reason = "additive only; used for error messages by a later task in this effort"
    )]
    pub fn describe(&self) -> String {
        match *self {
            Self::Name(ref name) => format!("'{name}'"),
            Self::Index(index) => format!("column {index}"),
        }
    }
}

impl serde::Serialize for ColumnRef {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match *self {
            Self::Name(ref name) => serializer.serialize_str(name),
            Self::Index(index) => {
                let as_u64 = u64::try_from(index).map_err(|_| {
                    serde::ser::Error::custom(format!("column index {index} exceeds u64"))
                })?;
                serializer.serialize_u64(as_u64)
            }
        }
    }
}

/// Serde visitor accepting either a column name or a zero-based column index.
struct ColumnRefVisitor;

impl serde::de::Visitor<'_> for ColumnRefVisitor {
    type Value = ColumnRef;

    fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("a column name string, or a non-negative zero-based column index")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(ColumnRef::Name(value.to_owned()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        usize::try_from(value)
            .map(ColumnRef::Index)
            .map_err(|_| E::custom(format!("column index {value} is too large")))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value < 0 {
            return Err(E::custom(format!(
                "column index {value} must not be negative; indices are zero-based"
            )));
        }
        usize::try_from(value)
            .map(ColumnRef::Index)
            .map_err(|_| E::custom(format!("column index {value} is too large")))
    }
}

impl<'de> serde::Deserialize<'de> for ColumnRef {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(ColumnRefVisitor)
    }
}

/// Describes how many lines of leading metadata precede the CSV data.
///
/// Many bank exports include banner or metadata rows before anything useful.
/// This enum declares only what to *discard*; whether a header row follows is
/// described separately by [`Header`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum Preamble {
    /// No preamble — the data (or its header) starts at the first line.
    None,
    /// Discard exactly `lines` lines.
    SkipLines {
        /// The number of lines to skip.
        lines: u32,
    },
}

impl Default for Preamble {
    #[inline]
    fn default() -> Self {
        Self::None
    }
}

/// Describes whether the file has a header row, and how to find it.
///
/// This is independent of [`Preamble`]: a file may have metadata lines to
/// discard *and* no header row, or neither, or both.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Header {
    /// The first line after the preamble is the header row.
    Present,
    /// Scan from the top; the first line whose fields all match the configured
    /// column names (case-insensitive) is the header. Earlier lines are
    /// discarded.
    AutoDetect {
        /// Maximum number of lines to scan before giving up.
        max_scan_lines: u32,
    },
    /// There is no header row; columns are addressed by index.
    Absent,
}

impl Default for Header {
    #[inline]
    fn default() -> Self {
        Self::Present
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
/// Construct using [`Config::default()`] and then set individual fields,
/// or deserialize from JSON.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// Account path this statement's rows post to, e.g. `"Assets:Bank:Checking"`.
    /// Stamped onto every emitted posting.
    pub account: String,
    /// Directory holding this account's statement files, relative to the
    /// host-preopened documents root (e.g. `"Assets/Bank/Checking"`).
    pub source_dir: String,
    /// Filename pattern selecting statement files in `source_dir`, supporting a
    /// single `*` wildcard (e.g. `"*.csv"`). Defaults to `"*"` (all files).
    #[serde(default = "default_source_glob")]
    pub source_glob: String,
    /// How to skip over metadata lines before the data begins.
    #[serde(default)]
    pub preamble: Preamble,
    /// Whether the file has a header row, and how to locate it.
    #[serde(default)]
    pub header: Header,
    /// The field delimiter character.
    #[serde(default = "default_delimiter")]
    pub delimiter: char,
    /// The name of the column containing the transaction date.
    #[serde(default = "default_date_column")]
    pub date_column: String,
    /// The date format string (jiff `strptime` syntax, e.g. `"%Y-%m-%d"`).
    #[serde(default = "default_date_format")]
    pub date_format: String,
    /// Which column(s) hold the monetary amount.
    #[serde(default)]
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
    pub decimal_separator: char,
    /// Optional thousands-separator character to strip from numeric fields.
    pub thousands_separator: Option<char>,
}

/// Returns the default field delimiter.
#[inline]
fn default_delimiter() -> char {
    ','
}

/// Default source glob: match every file in the source directory.
#[inline]
fn default_source_glob() -> String {
    "*".to_owned()
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
            account: String::new(),
            source_dir: String::new(),
            source_glob: default_source_glob(),
            preamble: Preamble::default(),
            header: Header::default(),
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
    /// Used by the [`Header::AutoDetect`] strategy to locate the header line.
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
    fn column_ref_deserializes_string_as_name() {
        let r: ColumnRef = serde_json::from_str(r#""Date""#).expect("string is a valid ColumnRef");
        assert_eq!(r, ColumnRef::Name("Date".to_owned()));
    }

    #[test]
    fn column_ref_deserializes_integer_as_index() {
        let r: ColumnRef = serde_json::from_str("3").expect("integer is a valid ColumnRef");
        assert_eq!(r, ColumnRef::Index(3));
    }

    #[test]
    fn column_ref_deserializes_zero_as_index() {
        let r: ColumnRef = serde_json::from_str("0").expect("zero is a valid column index");
        assert_eq!(r, ColumnRef::Index(0));
    }

    #[test]
    fn column_ref_rejects_negative_index() {
        let err = serde_json::from_str::<ColumnRef>("-1").expect_err("negative index is invalid");
        assert!(
            err.to_string().contains("negative"),
            "error should explain the sign problem, got: {err}"
        );
    }

    #[test]
    fn column_ref_rejects_fractional_index() {
        serde_json::from_str::<ColumnRef>("1.5").expect_err("fractional index is invalid");
    }

    #[test]
    fn column_ref_rejects_object() {
        serde_json::from_str::<ColumnRef>(r#"{"name": "Date"}"#).expect_err("object is invalid");
    }

    #[test]
    fn column_ref_round_trips_through_json() {
        let name = ColumnRef::Name("Amount".to_owned());
        assert_eq!(
            serde_json::to_string(&name).expect("serialize"),
            r#""Amount""#
        );
        let index = ColumnRef::Index(2);
        assert_eq!(serde_json::to_string(&index).expect("serialize"), "2");
    }

    #[test]
    fn column_ref_as_name_distinguishes_variants() {
        assert_eq!(ColumnRef::Name("Date".to_owned()).as_name(), Some("Date"));
        assert_eq!(ColumnRef::Index(0).as_name(), None);
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
    fn header_default_is_present() {
        assert_eq!(Header::default(), Header::Present);
    }

    #[test]
    fn header_deserializes_absent() {
        let h: Header = serde_json::from_str(r#"{"kind": "absent"}"#).expect("valid header");
        assert_eq!(h, Header::Absent);
    }

    #[test]
    fn header_deserializes_auto_detect_with_scan_limit() {
        let h: Header = serde_json::from_str(r#"{"kind": "auto_detect", "max_scan_lines": 5}"#)
            .expect("valid header");
        assert_eq!(h, Header::AutoDetect { max_scan_lines: 5 });
    }

    #[test]
    fn config_defaults_to_a_present_header() {
        assert_eq!(Config::default().header, Header::Present);
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
