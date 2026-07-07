//! Translate WIT-generated types to `bc-core` types.
//!
//! These conversions are the only place that couples `bc-plugins` to both
//! the WIT interface and `bc-core`'s domain types.

use bc_models::Amount;
use bc_models::CommodityCode;
use rust_decimal::Decimal;

use crate::host::bindings::borrow_checker::sdk::types as wt;

/// Converts a WIT wire-format date into a validated `jiff` civil date.
///
/// # Errors
///
/// Returns [`bc_core::ImportError::Parse`] if any component is out of range,
/// or if the combination does not form a valid calendar date.
fn wit_date(d: wt::Date) -> Result<jiff::civil::Date, bc_core::ImportError> {
    let year = i16::try_from(d.year).map_err(|_e| {
        bc_core::ImportError::Parse(format!("plugin returned year out of range: {}", d.year))
    })?;
    let month = i8::try_from(d.month).map_err(|_e| {
        bc_core::ImportError::Parse(format!("plugin returned month out of range: {}", d.month))
    })?;
    let day = i8::try_from(d.day).map_err(|_e| {
        bc_core::ImportError::Parse(format!("plugin returned day out of range: {}", d.day))
    })?;
    jiff::civil::Date::new(year, month, day).map_err(|e| {
        bc_core::ImportError::Parse(format!(
            "plugin returned invalid date {}-{:02}-{:02}: {e}",
            d.year, d.month, d.day
        ))
    })
}

impl From<wt::RawPosting> for bc_core::RawPosting {
    fn from(p: wt::RawPosting) -> Self {
        bc_core::RawPosting::builder()
            .account(p.account)
            .maybe_amount(p.amount.map(Amount::from))
            .maybe_balance(p.balance.map(Amount::from))
            .maybe_note(p.note)
            .tags(p.tags)
            .build()
    }
}

impl TryFrom<wt::RawTransaction> for bc_core::RawTransaction {
    type Error = bc_core::ImportError;

    fn try_from(t: wt::RawTransaction) -> Result<Self, Self::Error> {
        let date = wit_date(t.date)?;
        if t.postings.is_empty() {
            return Err(bc_core::ImportError::Parse(
                "plugin returned a raw transaction with no postings".to_owned(),
            ));
        }
        let extra_dates = t
            .extra_dates
            .into_iter()
            .map(|(label, raw_date)| wit_date(raw_date).map(|parsed| (label, parsed)))
            .collect::<Result<Vec<_>, _>>()?;

        Ok(bc_core::RawTransaction::builder()
            .date(date)
            .maybe_payee(t.payee)
            .description(t.description)
            .maybe_note(t.note)
            .maybe_reference(t.reference)
            .tags(t.tags)
            .extra_dates(extra_dates)
            .postings(t.postings.into_iter().map(Into::into).collect())
            .build())
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
    use pretty_assertions::assert_eq;

    use crate::host::bindings::borrow_checker::sdk::types as wt;

    #[test]
    fn wit_to_raw_transaction_rejects_out_of_range_month() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 99_u8,
                day: 1_u8,
            },
            payee: None,
            description: "test".to_owned(),
            note: None,
            reference: None,
            tags: vec![],
            extra_dates: vec![],
            postings: vec![],
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
            payee: None,
            description: "test".to_owned(),
            note: None,
            reference: None,
            tags: vec![],
            extra_dates: vec![],
            postings: vec![],
        };
        assert!(
            bc_core::RawTransaction::try_from(t).is_err(),
            "Feb 30 should fail date construction"
        );
    }

    #[test]
    fn wit_to_raw_transaction_maps_postings() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 6_u8,
                day: 27_u8,
            },
            payee: None,
            description: "Coffee".to_owned(),
            note: None,
            reference: None,
            tags: vec![],
            extra_dates: vec![],
            postings: vec![wt::RawPosting {
                account: "Assets:Bank:Checking".to_owned(),
                amount: Some(wt::Amount {
                    minor_units: -500_i64,
                    currency: "AUD".to_owned(),
                    scale: 2_u8,
                }),
                balance: None,
                note: None,
                tags: vec![],
            }],
        };
        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        assert_eq!(core.postings.len(), 1);
        assert_eq!(
            core.postings.first().expect("one posting").account,
            "Assets:Bank:Checking"
        );
    }

    #[test]
    fn wit_to_raw_transaction_rejects_empty_postings() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 6_u8,
                day: 27_u8,
            },
            payee: None,
            description: "Coffee".to_owned(),
            note: None,
            reference: None,
            tags: vec![],
            extra_dates: vec![],
            postings: vec![],
        };
        assert!(
            matches!(
                bc_core::RawTransaction::try_from(t),
                Err(bc_core::ImportError::Parse(_))
            ),
            "a transaction with no postings should be rejected at the WIT→core boundary"
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
