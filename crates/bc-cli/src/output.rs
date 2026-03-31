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
pub fn print_json<T: serde::Serialize>(value: &T) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints a formatted table with column headers and rows to stdout.
///
/// Uses [`comfy_table`] with an ASCII preset for clean, portable terminal
/// output without box-drawing characters.
///
/// # Arguments
///
/// * `headers` - Column header labels.
/// * `rows` - Table rows; each inner `Vec<String>` is one row of cell values.
#[inline]
pub fn print_table(headers: &[&str], rows: &[Vec<String>]) {
    let mut table = comfy_table::Table::new();
    table
        .load_preset(comfy_table::presets::ASCII_NO_BORDERS)
        .set_header(headers.to_vec());
    for row in rows {
        table.add_row(row.clone());
    }
    println!("{table}");
}
