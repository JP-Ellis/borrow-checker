//! Output helpers for human-readable tables and JSON.

#![expect(
    clippy::print_stdout,
    reason = "CLI binary: stdout is the intended output channel"
)]

use crate::error::CliResult;

/// Serialises `value` to pretty-printed JSON and prints it to stdout.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Json`] if serialisation fails.
#[inline]
pub fn print_json<T>(value: &T) -> CliResult<()>
where
    T: serde::Serialize,
{
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints a formatted table with column headers and rows to stdout.
///
/// Uses [`comfy_table`] with an ASCII style for clean, portable terminal
/// output without box-drawing characters.
///
/// # Arguments
///
/// * `headers` - Column header labels.
/// * `rows` - Table rows; each inner `Vec<String>` is one row of cell values.
#[inline]
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    println!("{}", format_table(headers, rows));
}

/// Renders a table in the same ASCII style [`print_table`] prints, without
/// the trailing newline.
///
/// # Arguments
///
/// * `headers` - Column header labels.
/// * `rows` - Table rows; each inner `Vec<String>` is one row of cell values.
#[inline]
#[must_use]
pub fn format_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut table = comfy_table::Table::new();
    table
        .load_style(comfy_table::presets::ASCII_NO_BORDERS)
        .set_header(headers.to_vec());
    for row in rows {
        table.add_row(row.clone());
    }
    table.to_string()
}

/// Formats `balances` as `0.5 BTC, 100.00 USD`.
#[must_use]
pub fn format_balances(balances: &bc_models::Balances) -> String {
    balances
        .iter()
        .map(|(code, value)| format!("{value} {code}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Serialises `value` and adds a `warnings` key naming each warning's
/// [`Display`](core::fmt::Display) rendering.
///
/// Used for the JSON payload of a write that can raise warnings, whether
/// [`bc_core::Warning`]s or the CLI's own, on a value that otherwise carries
/// none of its own: the payload stays a plain object a script can index
/// straight into, rather than nesting the written value under its own key.
///
/// Each call site also prints these same warnings to stderr unconditionally,
/// so stdout stays parseable while a human still sees them on the terminal.
/// A `--json` consumer that captures stderr too will see each warning twice
/// by design.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Json`] if `value` cannot serialise.
pub fn with_warnings<T, W>(value: &T, warnings: &[W]) -> CliResult<serde_json::Value>
where
    T: serde::Serialize,
    W: core::fmt::Display,
{
    let mut json = serde_json::to_value(value)?;
    if let serde_json::Value::Object(ref mut map) = json {
        map.insert(
            "warnings".to_owned(),
            serde_json::Value::Array(
                warnings
                    .iter()
                    .map(|warning| serde_json::Value::String(warning.to_string()))
                    .collect(),
            ),
        );
    }
    Ok(json)
}

/// Prints one `warning:` line per warning to stderr.
pub fn warn_all<W>(warnings: &[W])
where
    W: core::fmt::Display,
{
    for warning in warnings {
        #[expect(clippy::print_stderr, reason = "CLI output")]
        {
            eprintln!("warning: {warning}");
        }
    }
}

/// Formats `value` rounded half away from zero and padded to `decimals` places.
#[must_use]
pub fn format_at(value: rust_decimal::Decimal, decimals: u8) -> String {
    let mut rounded = value.round_dp_with_strategy(
        u32::from(decimals),
        rust_decimal::RoundingStrategy::MidpointAwayFromZero,
    );
    rounded.rescale(u32::from(decimals));
    rounded.to_string()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;

    #[rstest]
    #[case(dec!(-12345.6700), 2, "-12345.67")]
    #[case(dec!(-0.05), 2, "-0.05")]
    #[case(dec!(3), 2, "3.00")]
    #[case(dec!(0.125), 2, "0.13")]
    #[case(dec!(1.5), 0, "2")]
    fn formats_at_precision(#[case] value: Decimal, #[case] decimals: u8, #[case] expected: &str) {
        assert_eq!(format_at(value, decimals), expected);
    }
}
