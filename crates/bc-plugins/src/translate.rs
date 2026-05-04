//! Translate WIT-generated types to `bc-core` types.
//!
//! These conversions are the only place that couples `bc-plugins` to both
//! the WIT interface and `bc-core`'s domain types.

use bc_models::Amount;
use bc_models::CommodityCode;
use rust_decimal::Decimal;

use crate::host::bindings::borrow_checker::sdk::types as wt;

impl TryFrom<wt::RawTransaction> for bc_core::RawTransaction {
    type Error = bc_core::ImportError;

    fn try_from(t: wt::RawTransaction) -> Result<Self, Self::Error> {
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
            Amount::from(t.amount),
            t.balance.map(Amount::from),
            t.payee,
            t.description,
            t.reference,
        ))
    }
}

impl From<wt::Amount> for Amount {
    #[inline]
    fn from(a: wt::Amount) -> Self {
        let decimal = Decimal::new(a.minor_units, u32::from(a.scale));
        Amount::new(decimal, CommodityCode::new(a.currency))
    }
}

impl From<wt::ImportError> for bc_core::ImportError {
    fn from(e: wt::ImportError) -> Self {
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
}

#[cfg(test)]
mod tests {
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
            bc_core::RawTransaction::try_from(t).is_err(),
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
            bc_core::RawTransaction::try_from(t).is_err(),
            "Feb 30 should fail date construction"
        );
    }

    #[test]
    fn bad_value_wit_error_maps_to_parse_not_false_bad_value() {
        // The WIT boundary flattens BadValue { field, detail } into a single string.
        // On the host side, we must not reconstruct a fake BadValue with field="plugin"
        // — that misleads callers who match on the field name.
        let wit_err = wt::ImportError::BadValue("amount: not a number".to_owned());
        let bc_err = bc_core::ImportError::from(wit_err);
        assert!(
            matches!(bc_err, bc_core::ImportError::Parse(_)),
            "bad-value WIT error should map to Parse, not BadValue{{field=plugin}}; got: {bc_err:?}"
        );
    }
}
