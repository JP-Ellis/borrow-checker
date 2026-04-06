//! Translate WIT-generated types to `bc-core` types.
//!
//! These conversions are the only place that couples `bc-plugins` to both
//! the WIT interface and `bc-core`'s domain types.

use bc_models::Amount;
use bc_models::CommodityCode;
use rust_decimal::Decimal;

use crate::host::bindings::borrow_checker::sdk::types as wt;

/// Converts a WIT-generated `RawTransaction` to a [`bc_core::RawTransaction`].
///
/// # Arguments
///
/// * `t` - A WIT-generated `RawTransaction` value returned by a plugin.
///
/// # Returns
///
/// A [`bc_core::RawTransaction`] with all fields mapped.
///
/// # Panics
///
/// Panics if the date values are out of range. Plugin-returned dates are
/// expected to be valid; an invalid date indicates a plugin bug.
#[must_use]
#[expect(
    clippy::as_conversions,
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::expect_used,
    reason = "valid calendar dates fit in i16/i8; plugin bug produces a panic"
)]
pub(crate) fn wit_to_raw_transaction(t: wt::RawTransaction) -> bc_core::RawTransaction {
    let date = jiff::civil::Date::new(t.date.year as i16, t.date.month as i8, t.date.day as i8)
        .expect("plugin returned an invalid date");

    bc_core::RawTransaction::new(
        date,
        wit_to_amount(t.amount),
        t.balance.map(wit_to_amount),
        t.payee,
        t.description,
        t.reference,
    )
}

/// Converts a WIT-generated `Amount` to a [`bc_models::Amount`].
///
/// # Arguments
///
/// * `a` - A WIT-generated `Amount` value (minor units + currency + scale).
///
/// # Returns
///
/// A [`bc_models::Amount`] with the value reconstructed as a [`Decimal`].
#[must_use]
pub(crate) fn wit_to_amount(a: wt::Amount) -> Amount {
    let decimal = Decimal::new(a.minor_units, u32::from(a.scale));
    Amount::new(decimal, CommodityCode::new(a.currency))
}

/// Converts a WIT-generated `ImportError` to a [`bc_core::ImportError`].
///
/// # Arguments
///
/// * `e` - A WIT-generated `ImportError` variant returned by a plugin.
///
/// # Returns
///
/// A [`bc_core::ImportError`] with the message preserved.
#[must_use]
pub(crate) fn wit_to_import_error(e: wt::ImportError) -> bc_core::ImportError {
    match e {
        wt::ImportError::InvalidConfig(s) => {
            bc_core::ImportError::Parse(format!("invalid config: {s}"))
        }
        wt::ImportError::Parse(s) => bc_core::ImportError::Parse(s),
        wt::ImportError::MissingField(s) => bc_core::ImportError::MissingField(s),
        // The WIT `bad-value` merges field and detail into one string; map to
        // the structured `BadValue` variant so callers can still match on it.
        wt::ImportError::BadValue(s) => bc_core::ImportError::BadValue {
            field: "plugin".to_owned(),
            detail: s,
        },
    }
}
