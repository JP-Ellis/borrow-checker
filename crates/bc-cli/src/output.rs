//! Output helpers for human-readable tables and JSON.

use crate::error::CliResult;

/// Serialises `value` to pretty-printed JSON and prints it to stdout.
///
/// # Errors
///
/// Returns [`crate::error::CliError::Json`] if serialisation fails.
#[expect(
    clippy::print_stdout,
    reason = "CLI binary: stdout is the intended output channel"
)]
#[expect(dead_code, reason = "used by command handlers in subsequent tasks")]
#[inline]
pub fn print_json<T: serde::Serialize>(value: &T) -> CliResult<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Prints a table row with left-aligned fixed-width columns separated by two spaces.
///
/// # Arguments
///
/// * `columns` - Slice of `(text, width)` pairs. Text is truncated to `width` if longer.
#[expect(
    clippy::print_stdout,
    reason = "CLI binary: stdout is the intended output channel"
)]
#[expect(dead_code, reason = "used by command handlers in subsequent tasks")]
#[inline]
pub fn print_row(columns: &[(&str, usize)]) {
    let parts: Vec<String> = columns
        .iter()
        .map(|(text, width)| format!("{text:<width$}"))
        .collect();
    println!("{}", parts.join("  "));
}

/// Prints a horizontal divider of `width` dashes.
///
/// # Arguments
///
/// * `width` - Number of dash characters to print.
#[expect(
    clippy::print_stdout,
    reason = "CLI binary: stdout is the intended output channel"
)]
#[expect(dead_code, reason = "used by command handlers in subsequent tasks")]
#[inline]
pub fn print_divider(width: usize) {
    println!("{}", "-".repeat(width));
}
