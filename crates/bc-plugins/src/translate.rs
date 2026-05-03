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
/// # Errors
///
/// Returns an [`bc_core::ImportError`] if the plugin returned an invalid calendar date,
/// or if any date field is out of range for the target integer type.
pub(crate) fn wit_to_raw_transaction(
    t: wt::RawTransaction,
) -> Result<bc_core::RawTransaction, bc_core::ImportError> {
    let year = i16::try_from(t.date.year).map_err(|_e| {
        bc_core::ImportError::Parse(format!(
            "plugin returned year out of range: {}",
            t.date.year
        ))
    })?;
    let month = i8::try_from(t.date.month).map_err(|_e| {
        bc_core::ImportError::Parse(format!(
            "plugin returned month out of range: {}",
            t.date.month
        ))
    })?;
    let day = i8::try_from(t.date.day).map_err(|_e| {
        bc_core::ImportError::Parse(format!("plugin returned day out of range: {}", t.date.day))
    })?;
    let date = jiff::civil::Date::new(year, month, day).map_err(|e| {
        bc_core::ImportError::Parse(format!(
            "plugin returned invalid date {}-{:02}-{:02}: {e}",
            t.date.year, t.date.month, t.date.day
        ))
    })?;

    Ok(bc_core::RawTransaction::new(
        date,
        wit_to_amount(t.amount),
        t.balance.map(wit_to_amount),
        t.payee,
        t.description,
        t.reference,
    ))
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
        // The WIT `bad-value` merges field+detail into one string. Rather than
        // reconstruct a fake BadValue with field="plugin", map to Parse so
        // callers are not misled when matching on the field name.
        wt::ImportError::BadValue(s) => bc_core::ImportError::Parse(format!("bad value: {s}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::bindings::borrow_checker::sdk::types as wt;

    #[test]
    fn wit_to_raw_transaction_rejects_out_of_range_month() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 99_u8,
                day: 1_u8,
            },
            amount: wt::Amount {
                minor_units: 1000_i64,
                currency: "AUD".to_owned(),
                scale: 2_u8,
            },
            balance: None,
            payee: None,
            description: "test".to_owned(),
            reference: None,
        };
        assert!(
            wit_to_raw_transaction(t).is_err(),
            "month 99 should fail date construction"
        );
    }

    #[test]
    fn wit_to_raw_transaction_rejects_out_of_range_day() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 2_u8,
                day: 30_u8,
            },
            amount: wt::Amount {
                minor_units: 0_i64,
                currency: "AUD".to_owned(),
                scale: 2_u8,
            },
            balance: None,
            payee: None,
            description: "test".to_owned(),
            reference: None,
        };
        assert!(
            wit_to_raw_transaction(t).is_err(),
            "Feb 30 should fail date construction"
        );
    }

    #[test]
    fn bad_value_wit_error_maps_to_parse_not_false_bad_value() {
        // The WIT boundary flattens BadValue { field, detail } into a single string.
        // On the host side, we must not reconstruct a fake BadValue with field="plugin"
        // — that misleads callers who match on the field name.
        let wit_err = wt::ImportError::BadValue("amount: not a number".to_owned());
        let bc_err = wit_to_import_error(wit_err);
        assert!(
            matches!(bc_err, bc_core::ImportError::Parse(_)),
            "bad-value WIT error should map to Parse, not BadValue{{field=plugin}}; got: {bc_err:?}"
        );
    }
}
