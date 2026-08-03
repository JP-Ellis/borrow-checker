//! Configuration types for the CSV importer.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::Deserialize as _;

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
    pub fn describe(&self) -> String {
        match *self {
            Self::Name(ref name) => format!("'{name}'"),
            Self::Index(index) => format!("column {index}"),
        }
    }

    /// Returns the key under which two references address the same column.
    ///
    /// This mirrors resolution: names are folded case-insensitively, indices
    /// compare by value. A name and an index never share a key, because which
    /// physical column a name denotes is unknown until the header is read.
    /// Note this is a finer relation than the derived [`PartialEq`], which
    /// treats `"Amount"` and `"amount"` as distinct.
    fn dedup_key(&self) -> String {
        match *self {
            Self::Name(ref name) => format!("name:{}", name.to_ascii_lowercase()),
            Self::Index(index) => format!("index:{index}"),
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

/// Where a row's commodity code comes from.
///
/// Bank statements carry one currency for the whole file; exchange exports name
/// the asset per row. Deserializes from a bare JSON string as [`Self::Fixed`],
/// so profiles written before per-row commodities load unchanged:
///
/// ```json
/// { "commodity": "AUD" }
/// { "commodity": { "source": "fixed",  "code": "AUD" } }
/// { "commodity": { "source": "column", "column": "Coin" } }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum CommoditySource {
    /// One code applies to every row in the file.
    Fixed {
        /// The commodity code, e.g. `"AUD"`.
        code: String,
    },
    /// Each row names its own code in a column.
    Column {
        /// The column holding the code.
        column: ColumnRef,
    },
}

impl<'de> serde::Deserialize<'de> for CommoditySource {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        /// The tagged form, mirroring `CommoditySource`'s own shape.
        #[derive(serde::Deserialize)]
        #[serde(tag = "source", rename_all = "snake_case")]
        enum Tagged {
            Fixed { code: String },
            Column { column: ColumnRef },
        }

        impl From<Tagged> for CommoditySource {
            fn from(tagged: Tagged) -> Self {
                match tagged {
                    Tagged::Fixed { code } => Self::Fixed { code },
                    Tagged::Column { column } => Self::Column { column },
                }
            }
        }

        /// Accepts a bare string (the pre-per-row-commodity spelling of
        /// `Fixed`) or the tagged map form, deserializing the latter through
        /// `Tagged` so a malformed map keeps `Tagged`'s own diagnostic (e.g.
        /// the unknown-variant or missing-field message) instead of being
        /// flattened into a generic "did not match any variant" error.
        struct CommoditySourceVisitor;

        impl<'de> serde::de::Visitor<'de> for CommoditySourceVisitor {
            type Value = CommoditySource;

            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str(
                    "a bare commodity code string, or a map with a \"source\" of \"fixed\" or \"column\"",
                )
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                Ok(CommoditySource::Fixed {
                    code: value.to_owned(),
                })
            }

            fn visit_map<A>(self, map: A) -> Result<Self::Value, A::Error>
            where
                A: serde::de::MapAccess<'de>,
            {
                Tagged::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(Into::into)
            }
        }

        deserializer.deserialize_any(CommoditySourceVisitor)
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
    /// Scan forward from the first line *after* the preamble; the first line
    /// whose fields all match the configured column names (case-insensitive)
    /// is the header. Lines scanned before it are discarded.
    AutoDetect {
        /// Maximum number of lines to scan before giving up.
        ///
        /// Counted from the end of the preamble, not from the start of the
        /// file, so lines already discarded by [`Preamble`] do not consume the
        /// budget. Must be non-zero.
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
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "style", rename_all = "snake_case")]
pub enum AmountColumns {
    /// Single signed column: positive = credit (in), negative = debit (out).
    Single {
        /// The amount column.
        column: ColumnRef,
    },
    /// Separate columns; both always positive; exactly one populated per row.
    SplitDebitCredit {
        /// The debit (money out) column.
        debit_column: ColumnRef,
        /// The credit (money in) column.
        credit_column: ColumnRef,
    },
}

impl Default for AmountColumns {
    #[inline]
    fn default() -> Self {
        Self::Single {
            column: ColumnRef::Name("Amount".to_owned()),
        }
    }
}

/// Full configuration for the CSV importer.
///
/// Supports configurable delimiters, date formats, amount representations,
/// preamble handling, and columns addressed either by header name or by
/// zero-based position (see [`ColumnRef`]) for bank-style CSV exports.
///
/// Not every syntactically valid combination is meaningful — addressing a
/// column by name in a file with no header row, for instance. [`Config::validate`]
/// rejects those without reading a file.
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
    /// The column containing the transaction date.
    #[serde(default = "default_date_column")]
    pub date_column: ColumnRef,
    /// The date format string (jiff `strptime` syntax, e.g. `"%Y-%m-%d"`).
    #[serde(default = "default_date_format")]
    pub date_format: String,
    /// Which column(s) hold the monetary amount.
    #[serde(default)]
    pub amount_columns: AmountColumns,
    /// Optional column for the payee or merchant.
    pub payee_column: Option<ColumnRef>,
    /// Optional column for the free-text description.
    pub description_column: Option<ColumnRef>,
    /// Optional column for an institution-supplied reference number.
    pub reference_column: Option<ColumnRef>,
    /// Optional column for the running balance after each transaction.
    pub balance_column: Option<ColumnRef>,
    /// Where each row's commodity code comes from. Required — [`Config::validate`]
    /// rejects a profile without one.
    #[serde(default)]
    pub commodity: Option<CommoditySource>,
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

/// Returns the default date column reference.
#[inline]
fn default_date_column() -> ColumnRef {
    ColumnRef::Name("Date".to_owned())
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
            commodity: Some(CommoditySource::Fixed {
                code: "AUD".to_owned(),
            }),
            decimal_separator: default_decimal_separator(),
            thousands_separator: None,
        }
    }
}

impl Config {
    /// Returns the amount column(s) and, when the commodity is
    /// column-sourced, the commodity column — the references shared by
    /// [`Self::required_column_refs`] and [`Self::leg_column_refs`], which
    /// each prepend or append their own extras around this shared core.
    ///
    /// # Returns
    ///
    /// A `Vec` of `(field name, reference)` pairs.
    #[inline]
    fn amount_and_commodity_refs(&self) -> Vec<(Cow<'static, str>, &ColumnRef)> {
        let mut refs: Vec<(Cow<'static, str>, &ColumnRef)> = Vec::new();
        match self.amount_columns {
            AmountColumns::Single { ref column } => {
                refs.push((Cow::Borrowed("amount_columns.column"), column));
            }
            AmountColumns::SplitDebitCredit {
                ref debit_column,
                ref credit_column,
            } => {
                refs.push((Cow::Borrowed("amount_columns.debit_column"), debit_column));
                refs.push((Cow::Borrowed("amount_columns.credit_column"), credit_column));
            }
        }
        if let Some(CommoditySource::Column { ref column }) = self.commodity {
            refs.push((Cow::Borrowed("commodity.column"), column));
        }
        refs
    }

    /// Returns the column references that must resolve for any import, paired
    /// with their config field names.
    ///
    /// These are the date column, the amount column(s), and — when the
    /// commodity is column-sourced — the commodity column: the ones without
    /// which a row cannot become a posting.
    ///
    /// # Returns
    ///
    /// A `Vec` of `(field name, reference)` pairs.
    #[must_use]
    #[inline]
    pub fn required_column_refs(&self) -> Vec<(Cow<'static, str>, &ColumnRef)> {
        let mut refs: Vec<(Cow<'static, str>, &ColumnRef)> =
            vec![(Cow::Borrowed("date_column"), &self.date_column)];
        refs.extend(self.amount_and_commodity_refs());
        refs
    }

    /// Returns the column references describing the transaction as a whole,
    /// paired with their config field names.
    ///
    /// These name one value per row however many postings the row produces, so
    /// they form their own distinctness group.
    ///
    /// # Returns
    ///
    /// A `Vec` of `(field name, reference)` pairs for whichever are configured.
    #[must_use]
    #[inline]
    pub fn transaction_column_refs(&self) -> Vec<(Cow<'static, str>, &ColumnRef)> {
        let mut refs: Vec<(Cow<'static, str>, &ColumnRef)> =
            vec![(Cow::Borrowed("date_column"), &self.date_column)];
        for (field, maybe_ref) in [
            ("payee_column", &self.payee_column),
            ("description_column", &self.description_column),
            ("reference_column", &self.reference_column),
        ] {
            if let Some(column) = maybe_ref.as_ref() {
                refs.push((Cow::Borrowed(field), column));
            }
        }
        refs
    }

    /// Returns the primary leg's column references, paired with their config
    /// field names.
    ///
    /// A leg's references must be distinct from each other and from the
    /// transaction-level group, but two *different* legs may share a column —
    /// a fee leg and a total leg legitimately read one quote-currency column.
    ///
    /// # Returns
    ///
    /// A `Vec` of `(field name, reference)` pairs.
    #[must_use]
    #[inline]
    pub fn leg_column_refs(&self) -> Vec<(Cow<'static, str>, &ColumnRef)> {
        let mut refs = self.amount_and_commodity_refs();
        if let Some(column) = self.balance_column.as_ref() {
            refs.push((Cow::Borrowed("balance_column"), column));
        }
        refs
    }

    /// Returns every configured column reference, paired with its config field
    /// name.
    ///
    /// The union of [`Self::transaction_column_refs`] and
    /// [`Self::leg_column_refs`], used where the grouping does not matter —
    /// the headerless check, for instance, which rejects a name wherever it
    /// appears.
    ///
    /// # Returns
    ///
    /// A `Vec` of `(field name, reference)` pairs.
    #[must_use]
    #[inline]
    pub fn column_refs(&self) -> Vec<(Cow<'static, str>, &ColumnRef)> {
        let mut refs = self.transaction_column_refs();
        refs.extend(self.leg_column_refs());
        refs
    }

    /// Returns the column names required to identify the CSV header row.
    ///
    /// Used by [`Header::AutoDetect`] to locate the header line. Index-based
    /// references contribute no name and are omitted; [`Config::validate`]
    /// rejects the configurations where that would leave nothing to match on.
    ///
    /// # Returns
    ///
    /// A `Vec` of column name string slices.
    #[must_use]
    #[inline]
    pub fn required_column_names(&self) -> Vec<&str> {
        self.required_column_refs()
            .into_iter()
            .filter_map(|(_, column)| column.as_name())
            .collect()
    }

    /// Checks the configuration for internal coherence.
    ///
    /// This inspects the configuration only; it reads no files. Problems that
    /// depend on file contents — an index past the end of a row, for instance —
    /// remain parse-time errors.
    ///
    /// All violations are reported together rather than one per call, because
    /// a headerless profile converted from a name-based one typically has
    /// several at once.
    ///
    /// # Returns
    ///
    /// `Ok(())` when the configuration is coherent.
    ///
    /// # Errors
    ///
    /// Returns [`bc_sdk::ImportError::InvalidConfig`] listing every violation:
    /// - no commodity configured, so rows would have no commodity to post in;
    /// - a name-based column reference when the file has no header row;
    /// - an index-based required column (including a column-sourced commodity)
    ///   under [`Header::AutoDetect`], which matches the header line against
    ///   column names;
    /// - [`Header::AutoDetect`] with `max_scan_lines` of zero, which can never
    ///   find a header;
    /// - two configuration fields within the same distinctness group
    ///   addressing the same column.
    #[inline]
    pub fn validate(&self) -> Result<(), bc_sdk::ImportError> {
        let mut problems: Vec<String> = Vec::new();

        if self.commodity.is_none() {
            problems.push(
                "commodity is not set, so rows have no commodity to post in. Set it to a \
                 code (\"AUD\"), or to {\"source\":\"column\",\"column\":\"Coin\"} when each \
                 row names its own."
                    .to_owned(),
            );
        }

        /// Records the fields sharing a column within one distinctness group.
        fn collect_duplicates(
            group: &[(Cow<'static, str>, &ColumnRef)],
            problems: &mut Vec<String>,
        ) {
            let mut by_column: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for &(ref field, column) in group {
                by_column
                    .entry(column.dedup_key())
                    .or_default()
                    .push(field.clone().into_owned());
            }
            for (_, fields) in by_column.iter().filter(|&(_, g)| g.len() > 1) {
                problems.push(format!(
                    "these fields all address the same column: {}. \
                     Each column reference must denote a distinct column.",
                    fields.join(", ")
                ));
            }
        }

        let mut transaction_and_primary = self.transaction_column_refs();
        transaction_and_primary.extend(self.leg_column_refs());
        collect_duplicates(&transaction_and_primary, &mut problems);

        match self.header {
            Header::Absent => {
                let named: Vec<String> = self
                    .column_refs()
                    .into_iter()
                    .filter(|&(_, column)| column.as_name().is_some())
                    .map(|(field, _)| field.into_owned())
                    .collect();
                if !named.is_empty() {
                    problems.push(format!(
                        "header.kind is \"absent\", so there are no column names to match, \
                         but these fields are addressed by name: {}. \
                         Use zero-based integer indices instead.",
                        named.join(", ")
                    ));
                }
            }
            Header::AutoDetect { max_scan_lines } => {
                if max_scan_lines == 0 {
                    problems.push(
                        "header.kind is \"auto_detect\" with max_scan_lines of 0, so no line \
                         is ever examined and the header can never be found. Raise it, or \
                         use a different header.kind."
                            .to_owned(),
                    );
                }

                let positional: Vec<String> = self
                    .required_column_refs()
                    .into_iter()
                    .filter(|&(_, column)| column.as_name().is_none())
                    .map(|(field, _)| field.into_owned())
                    .collect();
                if !positional.is_empty() {
                    problems.push(format!(
                        "header.kind is \"auto_detect\", which finds the header by matching \
                         column names, but these required fields are addressed by index: {}. \
                         Name them, or use a different header.kind.",
                        positional.join(", ")
                    ));
                }
            }
            Header::Present => {}
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(bc_sdk::ImportError::InvalidConfig(problems.join("; ")))
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn validate_accepts_a_default_config() {
        Config::default()
            .validate()
            .expect("the default config is coherent");
    }

    #[test]
    fn validate_accepts_positional_columns_on_a_headerless_file() {
        let cfg = Config {
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            ..Config::default()
        };
        cfg.validate()
            .expect("all-positional on a headerless file is coherent");
    }

    #[test]
    fn validate_accepts_mixed_addressing_when_a_header_is_present() {
        // Headers exist but one is blank or duplicated, so it is addressed by index.
        let cfg = Config {
            header: Header::Present,
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(3),
            },
            ..Config::default()
        };
        cfg.validate()
            .expect("mixed addressing is legitimate with a header");
    }

    #[test]
    fn validate_rejects_named_columns_on_a_headerless_file() {
        let cfg = Config {
            header: Header::Absent,
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            payee_column: Some(ColumnRef::Name("Merchant".to_owned())),
            ..Config::default()
        };
        let err = cfg.validate().expect_err("named refs need a header row");
        let msg = err.to_string();
        assert!(
            msg.contains("date_column"),
            "should name date_column: {msg}"
        );
        assert!(
            msg.contains("payee_column"),
            "should name payee_column: {msg}"
        );
        assert!(
            !msg.contains("amount_columns"),
            "should not name the positional amount column: {msg}"
        );
    }

    #[test]
    fn validate_reports_every_named_column_at_once() {
        // A headerless profile typically has several; fixing them one run at a
        // time would be miserable, so all are reported together.
        let cfg = Config {
            header: Header::Absent,
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Name("Debit".to_owned()),
                credit_column: ColumnRef::Name("Credit".to_owned()),
            },
            description_column: Some(ColumnRef::Name("Details".to_owned())),
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("named refs need a header row")
            .to_string();
        for field in [
            "date_column",
            "amount_columns.debit_column",
            "amount_columns.credit_column",
            "description_column",
        ] {
            assert!(msg.contains(field), "should name {field}: {msg}");
        }
    }

    #[test]
    fn validate_rejects_auto_detect_with_positional_required_columns() {
        // Auto-detection matches a line against column *names*. With none, the
        // match set is empty, `.all()` is vacuously true, and the first data row
        // is silently eaten as the header.
        let cfg = Config {
            header: Header::AutoDetect { max_scan_lines: 10 },
            date_column: ColumnRef::Index(0),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("auto-detect needs names to match on")
            .to_string();
        assert!(
            msg.contains("date_column"),
            "should name date_column: {msg}"
        );
        assert!(
            msg.contains("amount_columns.column"),
            "should name the amount column: {msg}"
        );
    }

    /// R2 rejects *any* positional required column, not only the all-positional
    /// case.
    ///
    /// A weaker rule that fired only when every required ref was positional
    /// would pass the two neighbouring tests, so this pins the intermediate
    /// case: one name to match the header on, one index that cannot be checked.
    #[test]
    fn validate_rejects_auto_detect_with_one_positional_required_column() {
        let cfg = Config {
            header: Header::AutoDetect { max_scan_lines: 10 },
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("a single positional required column is enough to fail")
            .to_string();
        assert!(
            msg.contains("amount_columns.column"),
            "should name the positional amount column: {msg}"
        );
        assert!(
            !msg.contains("date_column"),
            "should not name the correctly-addressed date column: {msg}"
        );
    }

    #[test]
    fn validate_rejects_auto_detect_with_zero_scan_lines() {
        let cfg = Config {
            header: Header::AutoDetect { max_scan_lines: 0 },
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("scanning zero lines can never find a header")
            .to_string();
        assert!(
            msg.contains("max_scan_lines"),
            "should name max_scan_lines: {msg}"
        );
    }

    #[test]
    fn validate_rejects_two_fields_addressing_the_same_index() {
        let cfg = Config {
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Index(1),
                credit_column: ColumnRef::Index(1),
            },
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("debit and credit cannot be the same column")
            .to_string();
        assert!(
            msg.contains("amount_columns.debit_column")
                && msg.contains("amount_columns.credit_column"),
            "should name both colliding fields: {msg}"
        );
    }

    /// Duplicate detection folds case, because resolution does.
    ///
    /// The derived `PartialEq` on [`ColumnRef`] would treat these as distinct,
    /// which is why [`ColumnRef::dedup_key`] exists.
    #[test]
    fn validate_rejects_names_differing_only_in_case() {
        let cfg = Config {
            header: Header::Present,
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Amount".to_owned()),
            },
            payee_column: Some(ColumnRef::Name("amount".to_owned())),
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("'Amount' and 'amount' resolve to the same column")
            .to_string();
        assert!(
            msg.contains("amount_columns.column") && msg.contains("payee_column"),
            "should name both colliding fields: {msg}"
        );
    }

    /// A name and an index never collide, since a name's physical position is
    /// unknown until the header is read.
    #[test]
    fn validate_does_not_treat_a_name_and_an_index_as_duplicates() {
        let cfg = Config {
            header: Header::Present,
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(0),
            },
            ..Config::default()
        };
        cfg.validate()
            .expect("a name and an index are not comparable");
    }

    #[test]
    fn validate_rejects_a_missing_commodity() {
        let cfg = Config {
            commodity: None,
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("a profile must say what commodity its rows are in")
            .to_string();
        assert!(msg.contains("commodity"), "should name the field: {msg}");
    }

    #[test]
    fn validate_rejects_a_named_commodity_column_on_a_headerless_file() {
        let cfg = Config {
            header: Header::Absent,
            date_column: ColumnRef::Index(0),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Coin".to_owned()),
            }),
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("a named ref needs a header row")
            .to_string();
        assert!(
            msg.contains("commodity.column"),
            "should name the commodity column: {msg}"
        );
    }

    /// Auto-detection finds the header by matching column *names*, so every
    /// required column must contribute one — the commodity column included.
    #[test]
    fn validate_rejects_an_indexed_commodity_column_under_auto_detect() {
        let cfg = Config {
            header: Header::AutoDetect { max_scan_lines: 10 },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Index(4),
            }),
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("auto-detect needs a name to match on")
            .to_string();
        assert!(
            msg.contains("commodity.column"),
            "should name the commodity column: {msg}"
        );
    }

    #[test]
    fn required_column_names_includes_a_named_commodity_column() {
        let cfg = Config {
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("Coin".to_owned()),
            }),
            ..Config::default()
        };
        assert_eq!(cfg.required_column_names(), vec!["Date", "Amount", "Coin"]);
    }

    #[test]
    fn required_column_names_omits_a_fixed_commodity() {
        let cfg = Config::default();
        assert_eq!(cfg.required_column_names(), vec!["Date", "Amount"]);
    }

    #[test]
    fn validate_rejects_a_commodity_column_colliding_with_the_amount_column() {
        let cfg = Config {
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Amount".to_owned()),
            },
            commodity: Some(CommoditySource::Column {
                column: ColumnRef::Name("amount".to_owned()),
            }),
            ..Config::default()
        };
        let msg = cfg
            .validate()
            .expect_err("one column cannot be both the amount and the commodity")
            .to_string();
        assert!(
            msg.contains("amount_columns.column") && msg.contains("commodity.column"),
            "should name both colliding fields: {msg}"
        );
    }

    /// The old `Preamble::AutoDetect` spelling must fail loudly rather than
    /// deserializing into something with a different meaning.
    #[test]
    fn preamble_rejects_the_old_auto_detect_variant() {
        let err = serde_json::from_str::<Preamble>(r#"{"strategy":"auto_detect"}"#)
            .expect_err("auto_detect moved to header.kind");
        assert!(
            err.to_string().contains("auto_detect"),
            "error should name the removed variant: {err}"
        );
    }

    #[test]
    fn validate_allows_auto_detect_with_positional_optional_columns() {
        // Only the required columns must be named; optional ones may be positional.
        let cfg = Config {
            header: Header::AutoDetect { max_scan_lines: 10 },
            date_column: ColumnRef::Name("Date".to_owned()),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Name("Amount".to_owned()),
            },
            balance_column: Some(ColumnRef::Index(6)),
            ..Config::default()
        };
        cfg.validate()
            .expect("optional positional columns do not affect header detection");
    }

    #[test]
    fn default_config_has_expected_values() {
        let cfg = Config::default();
        assert_eq!(cfg.delimiter, ',');
        assert_eq!(cfg.date_column, ColumnRef::Name("Date".to_owned()));
        assert_eq!(cfg.date_format, "%Y-%m-%d");
        assert_eq!(cfg.decimal_separator, '.');
        assert!(cfg.thousands_separator.is_none());
        assert_eq!(
            cfg.commodity,
            Some(CommoditySource::Fixed {
                code: "AUD".to_owned()
            })
        );
    }

    #[test]
    fn required_column_names_single() {
        let cfg = Config::default();
        assert_eq!(cfg.required_column_names(), vec!["Date", "Amount"]);
    }

    #[test]
    fn required_column_names_omits_positional_refs() {
        // Index refs have no name to match a header line against.
        let cfg = Config {
            date_column: ColumnRef::Index(0),
            ..Config::default()
        };
        assert_eq!(cfg.required_column_names(), vec!["Amount"]);
    }

    #[test]
    fn column_refs_lists_required_then_configured_optionals() {
        let cfg = Config {
            date_column: ColumnRef::Index(0),
            amount_columns: AmountColumns::Single {
                column: ColumnRef::Index(1),
            },
            description_column: Some(ColumnRef::Index(2)),
            ..Config::default()
        };
        let fields: Vec<String> = cfg
            .column_refs()
            .into_iter()
            .map(|(field, _)| field.into_owned())
            .collect();
        assert_eq!(
            fields,
            vec!["date_column", "amount_columns.column", "description_column"]
        );
    }

    #[test]
    fn column_refs_omits_unset_optional_columns() {
        let cfg = Config::default();
        let fields: Vec<String> = cfg
            .column_refs()
            .into_iter()
            .map(|(field, _)| field.into_owned())
            .collect();
        assert_eq!(fields, vec!["date_column", "amount_columns.column"]);
    }

    #[test]
    fn column_refs_names_both_split_amount_columns() {
        let cfg = Config {
            amount_columns: AmountColumns::SplitDebitCredit {
                debit_column: ColumnRef::Index(1),
                credit_column: ColumnRef::Index(2),
            },
            ..Config::default()
        };
        let fields: Vec<String> = cfg
            .column_refs()
            .into_iter()
            .map(|(field, _)| field.into_owned())
            .collect();
        assert_eq!(
            fields,
            vec![
                "date_column",
                "amount_columns.debit_column",
                "amount_columns.credit_column"
            ]
        );
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
                debit_column: ColumnRef::Name("Debit".to_owned()),
                credit_column: ColumnRef::Name("Credit".to_owned()),
            },
            ..Config::default()
        };
        assert_eq!(cfg.required_column_names(), vec!["Date", "Debit", "Credit"]);
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
    fn header_deserializes_present() {
        let h: Header = serde_json::from_str(r#"{"kind": "present"}"#).expect("valid header");
        assert_eq!(h, Header::Present);
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
                column: ColumnRef::Name("Amount".to_owned())
            }
        );
    }

    /// Every profile written before per-row commodities spells this as a bare
    /// string. They must keep deserializing untouched.
    #[test]
    fn commodity_deserializes_a_bare_string_as_fixed() {
        let cfg: Config = serde_json::from_str(
            r#"{"account":"Assets:Bank","source_dir":"Bank","commodity":"AUD"}"#,
        )
        .expect("a bare string is a valid commodity");
        assert_eq!(
            cfg.commodity,
            Some(CommoditySource::Fixed {
                code: "AUD".to_owned()
            })
        );
    }

    #[test]
    fn commodity_deserializes_the_tagged_fixed_form() {
        let c: CommoditySource =
            serde_json::from_str(r#"{"source":"fixed","code":"AUD"}"#).expect("valid");
        assert_eq!(
            c,
            CommoditySource::Fixed {
                code: "AUD".to_owned()
            }
        );
    }

    #[test]
    fn commodity_deserializes_a_named_column() {
        let c: CommoditySource =
            serde_json::from_str(r#"{"source":"column","column":"Coin"}"#).expect("valid");
        assert_eq!(
            c,
            CommoditySource::Column {
                column: ColumnRef::Name("Coin".to_owned())
            }
        );
    }

    /// Headerless files address every column by index, this one included.
    #[test]
    fn commodity_deserializes_an_indexed_column() {
        let c: CommoditySource =
            serde_json::from_str(r#"{"source":"column","column":4}"#).expect("valid");
        assert_eq!(
            c,
            CommoditySource::Column {
                column: ColumnRef::Index(4)
            }
        );
    }

    #[test]
    fn commodity_always_serializes_tagged() {
        let fixed = CommoditySource::Fixed {
            code: "AUD".to_owned(),
        };
        assert_eq!(
            serde_json::to_string(&fixed).expect("serialize"),
            r#"{"source":"fixed","code":"AUD"}"#
        );
        let column = CommoditySource::Column {
            column: ColumnRef::Name("Coin".to_owned()),
        };
        assert_eq!(
            serde_json::to_string(&column).expect("serialize"),
            r#"{"source":"column","column":"Coin"}"#
        );
    }

    #[test]
    fn commodity_absent_deserializes_as_none() {
        let cfg: Config = serde_json::from_str(r#"{"account":"Assets:Bank","source_dir":"Bank"}"#)
            .expect("commodity is optional at the serde layer");
        assert!(cfg.commodity.is_none());
    }

    /// A hand-edited profile with an invalid tag must get a message naming
    /// the unknown variant, not the generic "did not match any variant" a
    /// naive untagged fallback would produce.
    #[test]
    fn commodity_rejects_an_unknown_source_tag() {
        let err = serde_json::from_str::<CommoditySource>(r#"{"source":"bogus","column":"Coin"}"#)
            .expect_err("an unrecognized source tag is not a valid commodity");
        let msg = err.to_string();
        assert!(
            msg.contains("unknown variant") && msg.contains("bogus"),
            "should name the unknown variant: {msg}"
        );
    }

    /// A `fixed` source with no `code` must get a message naming the missing
    /// field.
    #[test]
    fn commodity_rejects_a_fixed_source_missing_code() {
        let err = serde_json::from_str::<CommoditySource>(r#"{"source":"fixed"}"#)
            .expect_err("a fixed commodity without a code is malformed");
        let msg = err.to_string();
        assert!(
            msg.contains("missing field") && msg.contains("code"),
            "should name the missing field: {msg}"
        );
    }
}
