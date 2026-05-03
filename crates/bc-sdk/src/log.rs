//! Structured logging macros for BorrowChecker plugins.
//!
//! These macros compile to calls through the host-provided `logger` WIT import,
//! which the host re-emits via `tracing` with `target = "bc::plugin"`.
//!
//! # Usage
//!
//! ```rust,ignore
//! bc_sdk::warn!("multiple commodities"; dropped = count, kept = first_currency);
//! bc_sdk::debug!("parsed transaction"; date = %date, amount = amount);
//! ```

use crate::__bindings::borrow_checker::sdk::logger::LogField;
use crate::__bindings::borrow_checker::sdk::logger::LogLevel;
use crate::__bindings::borrow_checker::sdk::logger::log as wit_log;

/// Emits a log entry at the given level through the host logger import.
///
/// Internal helper called by the level-specific macros.
///
/// # Arguments
///
/// * `level` - The log level to emit at.
/// * `message` - The log message string.
/// * `fields` - Structured key-value fields to attach to the log entry.
#[doc(hidden)]
#[inline]
pub fn __emit(level: LogLevel, message: &str, fields: &[(&str, &str)]) {
    let wit_fields: Vec<LogField> = fields
        .iter()
        .map(|(k, v)| LogField {
            key: (*k).to_owned(),
            value: (*v).to_owned(),
        })
        .collect();
    wit_log(level, message, &wit_fields);
}

/// Emits a `TRACE`-level log entry.
///
/// ```rust,ignore
/// bc_sdk::trace!("sniffing header"; offset = 0, len = bytes.len());
/// ```
#[macro_export]
macro_rules! trace {
    ($msg:literal $(; $($key:ident = $val:expr),* $(,)?)?) => {{
        $crate::log::__emit(
            $crate::__bindings::borrow_checker::sdk::logger::LogLevel::Trace,
            $msg,
            &[$( $( (stringify!($key), &format!("{}", $val)) ),* )?],
        );
    }};
}

/// Emits a `DEBUG`-level log entry.
///
/// ```rust,ignore
/// bc_sdk::debug!("parsed transaction"; date = %date, amount = amount);
/// ```
#[macro_export]
macro_rules! debug {
    ($msg:literal $(; $($key:ident = $val:expr),* $(,)?)?) => {{
        $crate::log::__emit(
            $crate::__bindings::borrow_checker::sdk::logger::LogLevel::Debug,
            $msg,
            &[$( $( (stringify!($key), &format!("{}", $val)) ),* )?],
        );
    }};
}

/// Emits an `INFO`-level log entry.
///
/// ```rust,ignore
/// bc_sdk::info!("import complete"; count = transactions.len());
/// ```
#[macro_export]
macro_rules! info {
    ($msg:literal $(; $($key:ident = $val:expr),* $(,)?)?) => {{
        $crate::log::__emit(
            $crate::__bindings::borrow_checker::sdk::logger::LogLevel::Info,
            $msg,
            &[$( $( (stringify!($key), &format!("{}", $val)) ),* )?],
        );
    }};
}

/// Emits a `WARN`-level log entry.
///
/// ```rust,ignore
/// bc_sdk::warn!("multiple commodities"; dropped = count, kept = first_currency);
/// ```
#[macro_export]
macro_rules! warn {
    ($msg:literal $(; $($key:ident = $val:expr),* $(,)?)?) => {{
        $crate::log::__emit(
            $crate::__bindings::borrow_checker::sdk::logger::LogLevel::Warn,
            $msg,
            &[$( $( (stringify!($key), &format!("{}", $val)) ),* )?],
        );
    }};
}

/// Emits an `ERROR`-level log entry.
///
/// ```rust,ignore
/// bc_sdk::error!("failed to parse row"; row = row_number, reason = e);
/// ```
#[macro_export]
macro_rules! error {
    ($msg:literal $(; $($key:ident = $val:expr),* $(,)?)?) => {{
        $crate::log::__emit(
            $crate::__bindings::borrow_checker::sdk::logger::LogLevel::Error,
            $msg,
            &[$( $( (stringify!($key), &format!("{}", $val)) ),* )?],
        );
    }};
}
