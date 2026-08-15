//! Translate WIT-generated types to `bc-core` types.
//!
//! These conversions are the only place that couples `bc-plugins` to both
//! the WIT interface and `bc-core`'s domain types.

use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::MetaKey;
use bc_models::MetaValue;
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

/// Converts one WIT metadata value into the form the import pipeline reads.
///
/// Six of the seven types are self-contained. The seventh names an account by
/// path, and binding a path needs the account tree, which this crate cannot
/// see — so the path travels on as [`bc_core::RawMetaValue::AccountPath`] and
/// `bc-core` binds it where it binds a leg's account.
///
/// # Arguments
///
/// * `v` - The value as the plugin stated it.
///
/// # Returns
///
/// The translated value.
///
/// # Errors
///
/// Returns [`bc_core::ImportError::Parse`] when a number, a date or a
/// timestamp does not carry the type the plugin claimed for it.
fn wit_meta_value(v: wt::MetaValue) -> Result<bc_core::RawMetaValue, bc_core::ImportError> {
    let resolved = match v {
        wt::MetaValue::Text(text) => MetaValue::Text(text),
        // `from_str_exact` for the reason `wit_amount` uses it: `from_str`
        // silently rounds a value carrying more significant digits than
        // `Decimal` holds, which is precision loss dressed as success.
        wt::MetaValue::Number(ref raw) => {
            MetaValue::Number(Decimal::from_str_exact(raw).map_err(|e| {
                bc_core::ImportError::Parse(format!(
                    "plugin returned an unparsable metadata number {raw:?}: {e}"
                ))
            })?)
        }
        wt::MetaValue::Boolean(flag) => MetaValue::Boolean(flag),
        wt::MetaValue::Date(date) => MetaValue::Date(wit_date(date)?),
        wt::MetaValue::Timestamp(ref raw) => {
            MetaValue::Timestamp(raw.parse::<jiff::Timestamp>().map_err(|e| {
                bc_core::ImportError::Parse(format!(
                    "plugin returned an unparsable metadata timestamp {raw:?}: {e}"
                ))
            })?)
        }
        wt::MetaValue::Amount(amount) => MetaValue::Amount(Amount::try_from(amount)?),
        wt::MetaValue::Account(path) => return Ok(bc_core::RawMetaValue::AccountPath(path)),
    };
    Ok(bc_core::RawMetaValue::Resolved(resolved))
}

/// Converts a plugin's metadata list, dropping the entries it misnamed.
///
/// A key outside `[a-z][a-z0-9_-]*` costs its own entry and nothing else: the
/// rest of the row is worth importing, and the value is one annotation, not
/// the money.
///
/// # Arguments
///
/// * `entries` - The entries as the plugin stated them, in display order.
///
/// # Returns
///
/// The entries whose key was usable, in the order stated.
///
/// # Errors
///
/// Returns [`bc_core::ImportError::Parse`] when a value does not carry the
/// type the plugin claimed for it. A malformed value is the plugin's own
/// defect, unlike a key a user chose.
fn wit_metadata(
    entries: Vec<wt::MetaEntry>,
) -> Result<Vec<bc_core::RawMetaEntry>, bc_core::ImportError> {
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        match MetaKey::new(entry.key.clone()) {
            Ok(key) => out.push(
                bc_core::RawMetaEntry::builder()
                    .key(key)
                    .value(wit_meta_value(entry.value)?)
                    .build(),
            ),
            Err(error) => tracing::warn!(
                key = entry.key.as_str(),
                %error,
                "plugin stated a metadata key that is not usable; dropping the entry"
            ),
        }
    }
    Ok(out)
}

impl TryFrom<wt::RawPosting> for bc_core::RawPosting {
    type Error = bc_core::ImportError;

    fn try_from(p: wt::RawPosting) -> Result<Self, Self::Error> {
        Ok(bc_core::RawPosting::builder()
            .account(p.account)
            .maybe_amount(p.amount.map(Amount::try_from).transpose()?)
            .maybe_balance(p.balance.map(Amount::try_from).transpose()?)
            .tags(p.tags)
            .metadata(wit_metadata(p.metadata)?)
            .build())
    }
}

impl From<wt::SourceLocation> for bc_core::SourceLocation {
    #[inline]
    fn from(l: wt::SourceLocation) -> Self {
        bc_core::SourceLocation::builder()
            .display(l.display)
            .maybe_uri(l.uri)
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
        Ok(bc_core::RawTransaction::builder()
            .date(date)
            .description(t.description)
            .maybe_reference(t.reference)
            .tags(t.tags)
            .metadata(wit_metadata(t.metadata)?)
            .maybe_source_location(t.source_location.map(Into::into))
            .postings(
                t.postings
                    .into_iter()
                    .map(bc_core::RawPosting::try_from)
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .build())
    }
}

impl TryFrom<wt::Amount> for Amount {
    type Error = bc_core::ImportError;

    #[inline]
    fn try_from(a: wt::Amount) -> Result<Self, Self::Error> {
        let value = Decimal::from_str_exact(&a.value).map_err(|e| {
            bc_core::ImportError::Parse(format!(
                "plugin returned an unparsable amount {:?}: {e}",
                a.value
            ))
        })?;
        Ok(Amount::new(value, CommodityCode::new(a.commodity)))
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
    use std::str::FromStr as _;

    use pretty_assertions::assert_eq;

    use crate::host::bindings::borrow_checker::sdk::types as wt;

    #[test]
    fn wit_amount_parses_a_decimal_string_preserving_scale() {
        let a = wt::Amount {
            value: "50.00".to_owned(),
            commodity: "AUD".to_owned(),
        };
        let parsed = bc_models::Amount::try_from(a).expect("valid decimal string");
        assert_eq!(parsed.value().to_string(), "50.00");
        assert_eq!(parsed.commodity().as_str(), "AUD");
    }

    #[test]
    fn wit_amount_carries_eighteen_decimal_places() {
        let a = wt::Amount {
            value: "123.456789012345678".to_owned(),
            commodity: "ETH".to_owned(),
        };
        let parsed = bc_models::Amount::try_from(a).expect("valid decimal string");
        assert_eq!(parsed.value().to_string(), "123.456789012345678");
    }

    /// `Decimal::from_str_exact` must be used rather than `from_str`: the
    /// latter silently rounds a value with more significant digits than
    /// `Decimal` can hold, which is exactly the precision loss this task
    /// exists to eliminate. This value has 33 significant digits — beyond
    /// `Decimal`'s ~28-29 digit capacity — so `from_str_exact` must reject
    /// it rather than round it down to a value indistinguishable from `1`.
    #[test]
    fn wit_amount_rejects_a_value_with_more_precision_than_decimal_holds() {
        let raw = "1.00000000000000000000000000000005";
        assert_eq!(
            raw.chars().filter(char::is_ascii_digit).count(),
            33,
            "test fixture must genuinely exceed Decimal's digit capacity"
        );
        assert_eq!(
            rust_decimal::Decimal::from_str(raw).expect("from_str rounds rather than rejecting"),
            rust_decimal::Decimal::from_str("1").expect("valid decimal"),
            "from_str must round this value down to 1, proving the precision \
             loss from_str_exact is meant to prevent"
        );

        let a = wt::Amount {
            value: raw.to_owned(),
            commodity: "AUD".to_owned(),
        };
        bc_models::Amount::try_from(a)
            .expect_err("a value with more precision than Decimal can hold must be rejected");
    }

    /// A string wire format admits a failure the old integer one could not.
    #[test]
    fn wit_amount_rejects_an_unparsable_string() {
        let a = wt::Amount {
            value: "not-a-number".to_owned(),
            commodity: "AUD".to_owned(),
        };
        let err = bc_models::Amount::try_from(a).expect_err("garbage must not parse");
        assert!(
            err.to_string().contains("not-a-number"),
            "the error should quote the offending value: {err}"
        );
    }

    #[test]
    fn wit_to_raw_transaction_rejects_out_of_range_month() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 99_u8,
                day: 1_u8,
            },
            description: "test".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
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
            description: "test".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
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
            description: "Coffee".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
            postings: vec![wt::RawPosting {
                account: "Assets:Bank:Checking".to_owned(),
                amount: Some(wt::Amount {
                    value: "-5.00".to_owned(),
                    commodity: "AUD".to_owned(),
                }),
                balance: None,
                tags: vec![],
                metadata: vec![],
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
            description: "Coffee".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
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

    #[test]
    fn wit_to_raw_transaction_carries_source_location() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 6_u8,
                day: 27_u8,
            },
            description: "SPLIT".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: Some(wt::SourceLocation {
                display: "ledger/2025.beancount:412".to_owned(),
                uri: Some("file:///ledger/2025.beancount#L412".to_owned()),
            }),
            postings: vec![wt::RawPosting {
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                tags: vec![],
                metadata: vec![],
            }],
        };

        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        let location = core.source_location.expect("location carried through");
        assert_eq!(location.display, "ledger/2025.beancount:412");
        assert_eq!(
            location.uri.as_deref(),
            Some("file:///ledger/2025.beancount#L412")
        );
    }

    #[test]
    fn wit_to_raw_transaction_tolerates_an_absent_source_location() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 6_u8,
                day: 27_u8,
            },
            description: "SPLIT".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
            postings: vec![wt::RawPosting {
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                tags: vec![],
                metadata: vec![],
            }],
        };

        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        assert!(
            core.source_location.is_none(),
            "a source with no address must not be forced to fabricate one"
        );
    }

    /// Pairs `key` with `value` on the wire.
    fn wit_entry(key: &str, value: wt::MetaValue) -> wt::MetaEntry {
        wt::MetaEntry {
            key: key.to_owned(),
            value,
        }
    }

    /// Translates `entries` as one transaction's metadata.
    fn translated(entries: Vec<wt::MetaEntry>) -> Vec<bc_core::RawMetaEntry> {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2026_i32,
                month: 1_u8,
                day: 15_u8,
            },
            description: "Coffee".to_owned(),
            reference: None,
            tags: vec![],
            metadata: entries,
            source_location: None,
            postings: vec![wt::RawPosting {
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                tags: vec![],
                metadata: vec![],
            }],
        };
        bc_core::RawTransaction::try_from(t)
            .expect("valid metadata")
            .metadata
    }

    /// Builds the value half of the single translated entry.
    fn translated_value(value: wt::MetaValue) -> bc_core::RawMetaValue {
        let entries = translated(vec![wit_entry("thing", value)]);
        let [entry] = <[bc_core::RawMetaEntry; 1]>::try_from(entries)
            .expect("exactly one entry should survive");
        entry.value
    }

    #[test]
    fn every_wire_value_type_reaches_the_pipeline() {
        assert_eq!(
            translated_value(wt::MetaValue::Text("Generic Grocer".to_owned())),
            bc_core::RawMetaValue::Resolved(bc_models::MetaValue::Text(
                "Generic Grocer".to_owned()
            ))
        );
        assert_eq!(
            translated_value(wt::MetaValue::Number("1502.50".to_owned())),
            bc_core::RawMetaValue::Resolved(bc_models::MetaValue::Number(
                rust_decimal::Decimal::from_str("1502.50").expect("valid decimal")
            ))
        );
        assert_eq!(
            translated_value(wt::MetaValue::Boolean(true)),
            bc_core::RawMetaValue::Resolved(bc_models::MetaValue::Boolean(true))
        );
        assert_eq!(
            translated_value(wt::MetaValue::Date(wt::Date {
                year: 2026_i32,
                month: 1_u8,
                day: 15_u8
            })),
            bc_core::RawMetaValue::Resolved(bc_models::MetaValue::Date(jiff::civil::date(
                2026, 1, 15
            )))
        );
        assert_eq!(
            translated_value(wt::MetaValue::Timestamp("2023-11-14T22:13:20Z".to_owned())),
            bc_core::RawMetaValue::Resolved(bc_models::MetaValue::Timestamp(
                jiff::Timestamp::from_second(1_700_000_000).expect("valid timestamp")
            ))
        );
        assert_eq!(
            translated_value(wt::MetaValue::Amount(wt::Amount {
                value: "42.00".to_owned(),
                commodity: "AUD".to_owned(),
            })),
            bc_core::RawMetaValue::Resolved(bc_models::MetaValue::Amount(bc_models::Amount::new(
                rust_decimal::Decimal::from_str("42.00").expect("valid decimal"),
                bc_models::CommodityCode::new("AUD"),
            )))
        );
    }

    /// This crate holds no account tree, so a path travels on unbound and
    /// `bc-core` binds it where it binds a leg's account.
    #[test]
    fn an_account_value_stays_a_path() {
        assert_eq!(
            translated_value(wt::MetaValue::Account("Assets:Bank:Savings".to_owned())),
            bc_core::RawMetaValue::AccountPath("Assets:Bank:Savings".to_owned())
        );
    }

    #[test]
    fn a_number_carrying_more_precision_than_decimal_holds_is_rejected() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2026_i32,
                month: 1_u8,
                day: 15_u8,
            },
            description: "Coffee".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![wit_entry(
                "invoice",
                wt::MetaValue::Number("1.00000000000000000000000000000005".to_owned()),
            )],
            source_location: None,
            postings: vec![wt::RawPosting {
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                tags: vec![],
                metadata: vec![],
            }],
        };
        let error = bc_core::RawTransaction::try_from(t)
            .expect_err("a value beyond Decimal's capacity must be rejected");
        assert!(
            error.to_string().contains("metadata number"),
            "the error should name what failed: {error}"
        );
    }

    /// A user chose the key, so a bad one costs its own entry and nothing
    /// else. `Payee` also folds to `payee`, which is why the registry can hold
    /// one key rather than two spellings of it.
    #[test]
    fn an_unusable_key_is_dropped_and_its_siblings_survive() {
        let entries = translated(vec![
            wit_entry("Payee", wt::MetaValue::Text("Generic Grocer".to_owned())),
            wit_entry("in voice", wt::MetaValue::Text("dropped".to_owned())),
            wit_entry("note", wt::MetaValue::Text("weekly shop".to_owned())),
        ]);

        let keys: Vec<&str> = entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["payee", "note"]);
    }

    #[test]
    fn a_posting_carries_its_own_metadata() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2026_i32,
                month: 1_u8,
                day: 15_u8,
            },
            description: "Coffee".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
            postings: vec![wt::RawPosting {
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                tags: vec![],
                metadata: vec![wit_entry(
                    "note",
                    wt::MetaValue::Text("paid by card".to_owned()),
                )],
            }],
        };

        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        let posting = core.postings.first().expect("one posting");
        assert_eq!(
            posting.metadata,
            vec![bc_core::RawMetaEntry::resolved(
                bc_models::MetaKey::new("note").expect("valid key"),
                bc_models::MetaValue::Text("paid by card".to_owned()),
            )]
        );
    }
}
