//! Locates the start of the CSV data.
//!
//! Two independent axes: how many leading metadata lines to discard
//! ([`Preamble`]), and whether a header row follows and how to find it
//! ([`Header`]). The preamble is applied first, then the header is located
//! within what remains.

use crate::config::Header;
use crate::config::Preamble;

/// Finds the start of the CSV data within `bytes`.
///
/// The preamble is discarded first, then the header is located within what
/// remains. The two are independent.
///
/// # Arguments
///
/// * `bytes` - Raw file bytes.
/// * `preamble` - How many leading lines to discard.
/// * `header` - Whether a header row exists and how to find it.
/// * `delimiter` - Field delimiter used in the CSV (needed for `AutoDetect`).
/// * `required_columns` - Column names that must all appear in the header line
///   (used by `AutoDetect`; case-insensitive).
///
/// # Returns
///
/// A sub-slice of `bytes` starting at the header row, or at the first data
/// byte when the file has no header.
///
/// # Errors
///
/// Returns [`bc_sdk::ImportError::Parse`] if:
/// - `SkipLines` requests more lines than the file contains.
/// - `AutoDetect` exhausts `max_scan_lines` without finding a matching header.
pub(crate) fn find_csv_start<'a>(
    bytes: &'a [u8],
    preamble: &Preamble,
    header: &Header,
    delimiter: char,
    required_columns: &[&str],
) -> Result<&'a [u8], bc_sdk::ImportError> {
    let after_preamble = match *preamble {
        Preamble::None => bytes,
        Preamble::SkipLines { lines } => skip_lines(bytes, lines)?,
    };

    match *header {
        Header::Present | Header::Absent => Ok(after_preamble),
        Header::AutoDetect { max_scan_lines } => {
            auto_detect(after_preamble, delimiter, required_columns, max_scan_lines)
        }
    }
}

/// Skips exactly `n` newline-terminated lines from the start of `bytes`.
fn skip_lines(bytes: &[u8], n: u32) -> Result<&[u8], bc_sdk::ImportError> {
    let mut remaining = bytes;
    for i in 0..n {
        match remaining.iter().position(|&b| b == b'\n') {
            Some(pos) => {
                remaining = &remaining[pos + 1..];
            }
            None => {
                return Err(bc_sdk::ImportError::Parse(format!(
                    "file too short: requested to skip {n} lines but only {i} lines exist"
                )));
            }
        }
    }
    Ok(remaining)
}

/// Scans at most `max_scan_lines` lines looking for one that contains all
/// `required_columns` as CSV fields (case-insensitive).
fn auto_detect<'a>(
    bytes: &'a [u8],
    delimiter: char,
    required_columns: &[&str],
    max_scan_lines: u32,
) -> Result<&'a [u8], bc_sdk::ImportError> {
    let mut remaining = bytes;
    for line_num in 0..max_scan_lines {
        // Find the end of the current line.
        let line_end = remaining
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(remaining.len());

        let line_bytes = &remaining[..line_end];

        if line_contains_all_columns(line_bytes, delimiter, required_columns) {
            return Ok(remaining);
        }

        if line_end >= remaining.len() {
            // Reached end of file without finding the header.
            return Err(bc_sdk::ImportError::Parse(format!(
                "header not found within {} lines (end of file)",
                line_num + 1
            )));
        }

        remaining = &remaining[line_end + 1..];
    }

    Err(bc_sdk::ImportError::Parse(format!(
        "header not found within {max_scan_lines} lines"
    )))
}

/// Parses `line` as a single CSV row and checks that all `required_columns`
/// appear (case-insensitive) among its fields.
fn line_contains_all_columns(line: &[u8], delimiter: char, required_columns: &[&str]) -> bool {
    // Use the csv crate to parse the single line so we handle quoting correctly.
    let Ok(line_utf8) = core::str::from_utf8(line) else {
        return false;
    };
    // Trim a trailing carriage return so Windows-style CRLF lines work.
    let line_str = line_utf8.trim_end_matches('\r');

    // The `as` cast is safe: non-ASCII delimiters are rejected in
    // CsvImporter::import before find_csv_start is called.
    let delimiter_byte = delimiter as u8;

    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter_byte)
        .has_headers(false)
        .from_reader(line_str.as_bytes());

    let Some(Ok(record)) = reader.records().next() else {
        return false;
    };

    let fields: Vec<&str> = record.iter().collect();
    required_columns.iter().all(|req| {
        fields
            .iter()
            .any(|f| f.trim().eq_ignore_ascii_case(req.trim()))
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::config::Header;
    use crate::config::Preamble;

    #[test]
    fn none_returns_bytes_unchanged() {
        let data = b"Date,Amount\n2025-01-01,10.00\n";
        let result = find_csv_start(data, &Preamble::None, &Header::Present, ',', &[])
            .expect("should succeed");
        assert_eq!(result, data.as_slice());
    }

    #[test]
    fn skip_lines_zero_returns_unchanged() {
        let data = b"Date,Amount\n";
        let result = find_csv_start(
            data,
            &Preamble::SkipLines { lines: 0 },
            &Header::Present,
            ',',
            &[],
        )
        .expect("should succeed");
        assert_eq!(result, data.as_slice());
    }

    #[test]
    fn skip_lines_skips_correct_number() {
        let data = b"line1\nline2\nDate,Amount\n2025-01-01,10.00\n";
        let result = find_csv_start(
            data,
            &Preamble::SkipLines { lines: 2 },
            &Header::Present,
            ',',
            &[],
        )
        .expect("should succeed");
        assert_eq!(result, b"Date,Amount\n2025-01-01,10.00\n".as_slice());
    }

    #[test]
    fn skip_lines_too_many_returns_error() {
        let data = b"only one line\n";
        find_csv_start(
            data,
            &Preamble::SkipLines { lines: 5 },
            &Header::Present,
            ',',
            &[],
        )
        .expect_err("should fail when requesting more lines than exist");
    }

    #[test]
    fn auto_detect_finds_header_on_first_line() {
        let data = b"Date,Amount\n2025-01-01,10.00\n";
        let result = find_csv_start(
            data,
            &Preamble::None,
            &Header::AutoDetect { max_scan_lines: 10 },
            ',',
            &["Date", "Amount"],
        )
        .expect("should succeed");
        assert_eq!(result, data.as_slice());
    }

    #[test]
    fn auto_detect_finds_header_after_preamble() {
        let data = b"Name,Value\nOther,Row\nDate,Amount\n2025-01-01,10.00\n";
        let result = find_csv_start(
            data,
            &Preamble::None,
            &Header::AutoDetect { max_scan_lines: 10 },
            ',',
            &["Date", "Amount"],
        )
        .expect("should succeed");
        assert_eq!(result, b"Date,Amount\n2025-01-01,10.00\n".as_slice());
    }

    #[test]
    fn auto_detect_case_insensitive() {
        let data = b"date,AMOUNT\n2025-01-01,10.00\n";
        let result = find_csv_start(
            data,
            &Preamble::None,
            &Header::AutoDetect { max_scan_lines: 5 },
            ',',
            &["Date", "Amount"],
        )
        .expect("should succeed");
        assert_eq!(result, data.as_slice());
    }

    #[test]
    fn auto_detect_exceeds_max_scan_lines_returns_error() {
        let data = b"line1\nline2\nline3\nDate,Amount\n";
        find_csv_start(
            data,
            &Preamble::None,
            &Header::AutoDetect { max_scan_lines: 2 },
            ',',
            &["Date", "Amount"],
        )
        .expect_err("should fail when header is not found within max_scan_lines");
    }

    #[test]
    fn skip_lines_then_auto_detect_compose() {
        // The two axes are now independent: discard a fixed banner, then scan for
        // the header within what remains.
        let data = b"BANNER\nDate,Value\nDate,Amount\n2025-01-01,10.00\n";
        let result = find_csv_start(
            data,
            &Preamble::SkipLines { lines: 1 },
            &Header::AutoDetect { max_scan_lines: 5 },
            ',',
            &["Date", "Amount"],
        )
        .expect("should succeed");
        assert_eq!(result, b"Date,Amount\n2025-01-01,10.00\n".as_slice());
    }

    #[test]
    fn absent_header_returns_bytes_unchanged() {
        let data = b"01/02/2025,120.00,GENERIC GROCER\n";
        let result = find_csv_start(data, &Preamble::None, &Header::Absent, ',', &[])
            .expect("should succeed");
        assert_eq!(result, data.as_slice());
    }
}
