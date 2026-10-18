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
use crate::host::bindings::exports::borrow_checker::sdk::exporter as we;

/// Stands in for the location of a row whose plugin reported none.
///
/// Matches what `bc_core::import_exec` renders for the same case, so one
/// document's diagnostics read the same whichever layer emitted them.
const UNKNOWN_SOURCE: &str = "<unknown source>";

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

/// Translates a plugin's wire reconciliation state.
fn wit_reconciliation(r: wt::Reconciliation) -> bc_models::Reconciliation {
    match r {
        wt::Reconciliation::Unreconciled => bc_models::Reconciliation::Unreconciled,
        wt::Reconciliation::Flagged => bc_models::Reconciliation::Flagged,
        wt::Reconciliation::Reconciled => bc_models::Reconciliation::Reconciled,
    }
}

/// Translates a plugin's native posting block.
///
/// # Errors
///
/// Returns [`bc_core::ImportError::Parse`] when a spread date is invalid.
fn wit_native_posting(
    n: wt::NativePosting,
) -> Result<bc_core::NativePosting, bc_core::ImportError> {
    Ok(bc_core::NativePosting::builder()
        .maybe_handle(n.handle)
        .maybe_spread_from(n.spread_from.map(wit_date).transpose()?)
        .maybe_spread_until(n.spread_until.map(wit_date).transpose()?)
        .build())
}

/// Translates a plugin's stated price or cost figure.
///
/// # Arguments
///
/// * `what` - `"price"` or `"cost"`, for the diagnostic.
/// * `q` - The figure as the plugin stated it.
///
/// # Returns
///
/// The figure in the same form (per unit or total).
///
/// # Errors
///
/// Returns [`bc_core::ImportError::Parse`] when the amount does not parse,
/// and [`bc_core::ImportError::BadValue`] naming `what` when it is negative:
/// a price or cost is stated without sign, so a negative one is a plugin
/// defect rather than a fact about the leg.
fn wit_quote(what: &str, q: wt::Quote) -> Result<bc_models::Quote, bc_core::ImportError> {
    let (amount, total) = match q {
        wt::Quote::PerUnit(a) => (Amount::try_from(a)?, false),
        wt::Quote::Total(a) => (Amount::try_from(a)?, true),
    };
    if amount.value().is_sign_negative() && !amount.value().is_zero() {
        return Err(bc_core::ImportError::BadValue {
            field: what.to_owned(),
            detail: format!(
                "plugin returned a negative {what} {} {}",
                amount.value(),
                amount.commodity().as_str()
            ),
        });
    }
    Ok(if total {
        bc_models::Quote::Total(amount)
    } else {
        bc_models::Quote::PerUnit(amount)
    })
}

/// Translates a plugin's stated cost basis.
///
/// # Arguments
///
/// * `c` - The cost as the plugin stated it.
///
/// # Returns
///
/// The cost basis with its lot date and label.
///
/// # Errors
///
/// As [`wit_quote`] for the basis, and as [`wit_date`] for the lot date.
fn wit_cost(c: wt::Cost) -> Result<bc_models::Cost, bc_core::ImportError> {
    Ok(bc_models::Cost::builder()
        .basis(wit_quote("cost", c.basis)?)
        .maybe_date(c.date.map(wit_date).transpose()?)
        .maybe_label(c.label)
        .build())
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
/// * `location` - Where the document stated these entries, for diagnostics.
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
    location: &str,
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
                location,
                key = entry.key.as_str(),
                %error,
                "plugin stated a metadata key that is not usable; dropping the entry"
            ),
        }
    }
    Ok(out)
}

/// Converts one WIT posting leg into the form the import pipeline reads.
///
/// Takes the location from its transaction: a leg does not carry one of its
/// own, and a diagnostic naming no row leaves the user nothing to fix.
///
/// # Arguments
///
/// * `p` - The leg as the plugin stated it.
/// * `location` - Where the document stated the transaction this leg belongs
///   to, for diagnostics.
///
/// # Returns
///
/// The translated leg.
///
/// # Errors
///
/// Returns [`bc_core::ImportError::Parse`] when an amount, a balance, a
/// price, a cost or a metadata value does not carry the type the plugin
/// claimed for it, and [`bc_core::ImportError::BadValue`] naming `price` or
/// `cost` when that figure is negative.
fn wit_posting(
    p: wt::RawPosting,
    location: &str,
) -> Result<bc_core::RawPosting, bc_core::ImportError> {
    Ok(bc_core::RawPosting::builder()
        .account(p.account)
        .maybe_amount(p.amount.map(Amount::try_from).transpose()?)
        .maybe_balance(p.balance.map(Amount::try_from).transpose()?)
        .maybe_price(p.price.map(|q| wit_quote("price", q)).transpose()?)
        .maybe_cost(p.cost.map(wit_cost).transpose()?)
        .tags(p.tags)
        .metadata(wit_metadata(p.metadata, location)?)
        .maybe_native(p.native.map(wit_native_posting).transpose()?)
        .build())
}

impl From<wt::NativeTransaction> for bc_core::NativeTransaction {
    #[inline]
    fn from(n: wt::NativeTransaction) -> Self {
        bc_core::NativeTransaction::builder()
            .maybe_handle(n.handle)
            .maybe_reconciliation(n.reconciliation.map(wit_reconciliation))
            .delete(n.delete)
            .build()
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
        // Rendered before the builder consumes `t`, and named the same way
        // `bc_core::import_exec` names a row with no stated location.
        let location = t
            .source_location
            .as_ref()
            .map_or(UNKNOWN_SOURCE, |stated| stated.display.as_str())
            .to_owned();
        Ok(bc_core::RawTransaction::builder()
            .date(date)
            .description(t.description)
            .maybe_reference(t.reference)
            .tags(t.tags)
            .metadata(wit_metadata(t.metadata, &location)?)
            .maybe_source_location(t.source_location.map(Into::into))
            .postings(
                t.postings
                    .into_iter()
                    .map(|p| wit_posting(p, &location))
                    .collect::<Result<Vec<_>, _>>()?,
            )
            .maybe_native(t.native.map(Into::into))
            .build())
    }
}

impl TryFrom<wt::AccountOpen> for bc_core::AccountOpen {
    type Error = bc_core::ImportError;

    fn try_from(o: wt::AccountOpen) -> Result<Self, Self::Error> {
        Ok(Self::builder()
            .date(wit_date(o.date)?)
            .account(o.account)
            .commodities(o.commodities)
            .maybe_source_location(o.source_location.map(Into::into))
            .build())
    }
}

impl TryFrom<wt::AccountClose> for bc_core::AccountClose {
    type Error = bc_core::ImportError;

    fn try_from(c: wt::AccountClose) -> Result<Self, Self::Error> {
        Ok(Self::builder()
            .date(wit_date(c.date)?)
            .account(c.account)
            .maybe_source_location(c.source_location.map(Into::into))
            .build())
    }
}

impl TryFrom<wt::Directive> for bc_core::Directive {
    type Error = bc_core::ImportError;

    fn try_from(d: wt::Directive) -> Result<Self, Self::Error> {
        match d {
            wt::Directive::Transaction(t) => {
                bc_core::RawTransaction::try_from(t).map(Self::Transaction)
            }
            wt::Directive::Open(o) => bc_core::AccountOpen::try_from(o).map(Self::Open),
            wt::Directive::Close(c) => bc_core::AccountClose::try_from(c).map(Self::Close),
        }
    }
}

impl From<wt::SourceOutcome> for bc_core::SourceOutcome {
    #[inline]
    fn from(o: wt::SourceOutcome) -> Self {
        let status = match o.status {
            wt::SourceStatus::Parsed(directives) => bc_core::SourceStatus::Parsed {
                directives: usize::try_from(directives).unwrap_or(usize::MAX),
            },
            wt::SourceStatus::Failed(reason) => bc_core::SourceStatus::Failed { reason },
        };
        Self::new(o.source.into(), status)
    }
}

impl TryFrom<wt::ImportOutput> for bc_core::ImportOutput {
    type Error = bc_core::ImportError;

    fn try_from(o: wt::ImportOutput) -> Result<Self, Self::Error> {
        let directives = o
            .directives
            .into_iter()
            .map(bc_core::Directive::try_from)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(
            directives,
            o.sources.into_iter().map(Into::into).collect(),
        ))
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

// MARK: Export direction

/// Converts a civil date to its wire form.
fn date_to_wit(d: jiff::civil::Date) -> wt::Date {
    wt::Date {
        year: i32::from(d.year()),
        // A valid civil date has month 1..=12 and day 1..=31.
        month: u8::try_from(d.month()).unwrap_or_default(),
        day: u8::try_from(d.day()).unwrap_or_default(),
    }
}

/// Converts an amount to its wire form, keeping the stated scale.
fn amount_to_wit(a: &Amount) -> wt::Amount {
    wt::Amount {
        value: a.value().to_string(),
        commodity: a.commodity().as_str().to_owned(),
    }
}

/// Converts a price or cost figure to its wire form, keeping its form.
fn quote_to_wit(q: &bc_models::Quote) -> wt::Quote {
    match *q {
        bc_models::Quote::PerUnit(ref a) => wt::Quote::PerUnit(amount_to_wit(a)),
        bc_models::Quote::Total(ref a) => wt::Quote::Total(amount_to_wit(a)),
    }
}

/// Converts a cost basis to its wire form.
fn cost_to_wit(c: &bc_models::Cost) -> wt::Cost {
    wt::Cost {
        basis: quote_to_wit(c.basis()),
        date: c.date().map(date_to_wit),
        label: c.label().map(str::to_owned),
    }
}

/// Converts a reconciliation state to its wire form.
fn reconciliation_to_wit(r: bc_models::Reconciliation) -> wt::Reconciliation {
    match r {
        bc_models::Reconciliation::Flagged => wt::Reconciliation::Flagged,
        bc_models::Reconciliation::Reconciled => wt::Reconciliation::Reconciled,
        // The enum is non-exhaustive; a state added later reads as unconfirmed
        // until it has a wire form.
        bc_models::Reconciliation::Unreconciled | _ => wt::Reconciliation::Unreconciled,
    }
}

/// Converts one metadata entry to its wire form.
///
/// # Errors
///
/// Returns [`bc_core::ExportError::Write`] for an account value that is bound
/// to an id: the wire form names an account by path, and a snapshot states
/// every account reference as one.
fn meta_entry_to_wit(e: &bc_core::RawMetaEntry) -> Result<wt::MetaEntry, bc_core::ExportError> {
    let value = match e.value {
        bc_core::RawMetaValue::AccountPath(ref path) => wt::MetaValue::Account(path.clone()),
        bc_core::RawMetaValue::Resolved(MetaValue::Text(ref text)) => {
            wt::MetaValue::Text(text.clone())
        }
        bc_core::RawMetaValue::Resolved(MetaValue::Number(n)) => {
            wt::MetaValue::Number(n.to_string())
        }
        bc_core::RawMetaValue::Resolved(MetaValue::Boolean(flag)) => wt::MetaValue::Boolean(flag),
        bc_core::RawMetaValue::Resolved(MetaValue::Date(date)) => {
            wt::MetaValue::Date(date_to_wit(date))
        }
        bc_core::RawMetaValue::Resolved(MetaValue::Timestamp(ts)) => {
            wt::MetaValue::Timestamp(ts.to_string())
        }
        bc_core::RawMetaValue::Resolved(MetaValue::Amount(ref a)) => {
            wt::MetaValue::Amount(amount_to_wit(a))
        }
        bc_core::RawMetaValue::Resolved(_) | _ => {
            return Err(bc_core::ExportError::Write(format!(
                "metadata `{}` holds an account id, which the snapshot must state as a path",
                e.key
            )));
        }
    };
    Ok(wt::MetaEntry {
        key: e.key.as_str().to_owned(),
        value,
    })
}

/// Converts a metadata list to its wire form, in order.
fn metadata_to_wit(
    entries: &[bc_core::RawMetaEntry],
) -> Result<Vec<wt::MetaEntry>, bc_core::ExportError> {
    entries.iter().map(meta_entry_to_wit).collect()
}

/// Converts one posting leg to its wire form.
fn posting_entry_to_wit(
    p: &bc_core::ExportPostingEntry,
) -> Result<we::ExportPosting, bc_core::ExportError> {
    Ok(we::ExportPosting {
        handle: p.handle.clone(),
        account: p.account.clone(),
        amount: p.amount.as_ref().map(amount_to_wit),
        price: p.price.as_ref().map(quote_to_wit),
        cost: p.cost.as_ref().map(cost_to_wit),
        tags: p.tags.clone(),
        metadata: metadata_to_wit(&p.metadata)?,
        spread_from: p.spread_from.map(date_to_wit),
        spread_until: p.spread_until.map(date_to_wit),
    })
}

/// Converts one transaction to its wire form.
fn transaction_entry_to_wit(
    t: &bc_core::ExportTransactionEntry,
) -> Result<we::ExportTransaction, bc_core::ExportError> {
    Ok(we::ExportTransaction {
        handle: t.handle.clone(),
        date: date_to_wit(t.date),
        description: t.description.clone(),
        reconciliation: reconciliation_to_wit(t.reconciliation),
        tags: t.tags.clone(),
        metadata: metadata_to_wit(&t.metadata)?,
        postings: t
            .postings
            .iter()
            .map(posting_entry_to_wit)
            .collect::<Result<_, _>>()?,
        unbalanced: t.unbalanced,
    })
}

impl TryFrom<&bc_core::ExportSnapshot> for we::ExportSnapshot {
    type Error = bc_core::ExportError;

    #[inline]
    fn try_from(s: &bc_core::ExportSnapshot) -> Result<Self, Self::Error> {
        Ok(we::ExportSnapshot {
            level: match s.level {
                bc_core::ExportLevel::Archive => we::ExportLevel::Archive,
                bc_core::ExportLevel::Working => we::ExportLevel::Working,
            },
            accounts: s
                .accounts
                .iter()
                .map(|a| {
                    Ok(we::ExportAccount {
                        path: a.path.clone(),
                        opened_on: a.opened_on.map(date_to_wit),
                        closed_on: a.closed_on.map(date_to_wit),
                        commodities: a.commodities.clone(),
                        extra: metadata_to_wit(&a.extra)?,
                    })
                })
                .collect::<Result<_, bc_core::ExportError>>()?,
            commodities: s
                .commodities
                .iter()
                .map(|c| {
                    Ok(we::ExportCommodity {
                        code: c.code.clone(),
                        extra: metadata_to_wit(&c.extra)?,
                    })
                })
                .collect::<Result<_, bc_core::ExportError>>()?,
            transactions: s
                .transactions
                .iter()
                .map(transaction_entry_to_wit)
                .collect::<Result<_, _>>()?,
            budgets: s
                .budgets
                .iter()
                .map(|b| {
                    Ok(we::ExportBudget {
                        account: b.account.clone(),
                        effective_from: date_to_wit(b.effective_from),
                        period: b.period.clone(),
                        target: b.target.as_ref().map(amount_to_wit),
                        expression: b.expression.clone(),
                        extra: metadata_to_wit(&b.extra)?,
                    })
                })
                .collect::<Result<_, bc_core::ExportError>>()?,
            balances: s
                .balances
                .iter()
                .map(|b| we::ExportBalance {
                    date: date_to_wit(b.date),
                    account: b.account.clone(),
                    amount: amount_to_wit(&b.amount),
                })
                .collect(),
        })
    }
}

impl From<we::ExportFile> for bc_core::ExportFile {
    #[inline]
    fn from(f: we::ExportFile) -> Self {
        Self {
            path: f.path,
            contents: f.contents,
        }
    }
}

impl From<we::ExportError> for bc_core::ExportError {
    #[inline]
    fn from(e: we::ExportError) -> Self {
        match e {
            we::ExportError::Unsupported => bc_core::ExportError::Unsupported,
            we::ExportError::Unspellable(items) => bc_core::ExportError::Unspellable(items),
            we::ExportError::Other(detail) => bc_core::ExportError::Plugin(detail),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::str::FromStr as _;

    use bc_models::Amount;
    use bc_models::Cost;
    use bc_models::MetaValue;
    use bc_models::Quote;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::wit_cost;
    use super::wit_date;
    use super::wit_metadata;
    use super::wit_quote;
    use super::wit_reconciliation;
    use crate::host::bindings::borrow_checker::sdk::types as wt;
    use crate::host::bindings::exports::borrow_checker::sdk::exporter as we;

    #[test]
    fn an_import_output_keeps_each_source_outcome() {
        let wit = wt::ImportOutput {
            directives: Vec::new(),
            sources: vec![
                wt::SourceOutcome {
                    source: wt::SourceLocation {
                        display: "2025-06.csv".to_owned(),
                        uri: None,
                    },
                    status: wt::SourceStatus::Parsed(12),
                },
                wt::SourceOutcome {
                    source: wt::SourceLocation {
                        display: "2025-07.csv".to_owned(),
                        uri: None,
                    },
                    status: wt::SourceStatus::Failed("bad date".to_owned()),
                },
            ],
        };

        let output = bc_core::ImportOutput::try_from(wit).expect("a valid output");

        assert_eq!(
            output.sources,
            vec![
                bc_core::SourceOutcome::parsed("2025-06.csv", 12),
                bc_core::SourceOutcome::failed("2025-07.csv", "bad date"),
            ]
        );
    }

    #[test]
    fn an_open_directive_round_trips() {
        let wit = wt::Directive::Open(wt::AccountOpen {
            date: wt::Date {
                year: 2024_i32,
                month: 3_u8,
                day: 1_u8,
            },
            account: "Assets:Bank:Checking".to_owned(),
            commodities: vec!["AUD".to_owned(), "XTS".to_owned()],
            source_location: Some(wt::SourceLocation {
                display: "ledger line 4".to_owned(),
                uri: None,
            }),
        });

        let directive = bc_core::Directive::try_from(wit).expect("a valid open");

        let expected = bc_core::AccountOpen::builder()
            .date(jiff::civil::date(2024, 3, 1))
            .account("Assets:Bank:Checking")
            .commodities(vec!["AUD".to_owned(), "XTS".to_owned()])
            .source_location(
                bc_core::SourceLocation::builder()
                    .display("ledger line 4")
                    .build(),
            )
            .build();
        assert_eq!(directive, bc_core::Directive::Open(expected));
    }

    #[test]
    fn an_open_with_an_impossible_date_is_a_parse_error() {
        let wit = wt::Directive::Open(wt::AccountOpen {
            date: wt::Date {
                year: 2024_i32,
                month: 2_u8,
                day: 30_u8,
            },
            account: "Assets:Bank:Checking".to_owned(),
            commodities: vec![],
            source_location: None,
        });

        let err = bc_core::Directive::try_from(wit).expect_err("2024-02-30 does not exist");

        assert!(
            matches!(err, bc_core::ImportError::Parse(_)),
            "an impossible date must be a parse error, got {err:?}"
        );
    }

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
            native: None,
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
            native: None,
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
            native: None,
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
                native: None,
                account: "Assets:Bank:Checking".to_owned(),
                amount: Some(wt::Amount {
                    value: "-5.00".to_owned(),
                    commodity: "AUD".to_owned(),
                }),
                balance: None,
                price: None,
                cost: None,
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
            native: None,
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
            native: None,
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
                native: None,
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                price: None,
                cost: None,
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
            native: None,
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
                native: None,
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                price: None,
                cost: None,
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
            native: None,
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
                native: None,
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                price: None,
                cost: None,
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
            native: None,
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
                native: None,
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                price: None,
                cost: None,
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
            native: None,
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
                native: None,
                account: "Assets:Bank".to_owned(),
                amount: None,
                balance: None,
                price: None,
                cost: None,
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

    fn wit_amount(value: &str, commodity: &str) -> wt::Amount {
        wt::Amount {
            value: value.to_owned(),
            commodity: commodity.to_owned(),
        }
    }

    #[test]
    fn a_price_crosses_in_its_stated_form() {
        let per_unit =
            wit_quote("price", wt::Quote::PerUnit(wit_amount("332", "AUD"))).expect("per-unit");
        assert_eq!(per_unit, Quote::PerUnit(Amount::new(dec!(332), "AUD")));
        let total = wit_quote("price", wt::Quote::Total(wit_amount("6.37", "AUD"))).expect("total");
        assert_eq!(total, Quote::Total(Amount::new(dec!(6.37), "AUD")));
    }

    #[test]
    fn a_negative_quote_is_a_bad_value_naming_the_field() {
        let err =
            wit_quote("cost", wt::Quote::PerUnit(wit_amount("-105", "AUD"))).expect_err("rejects");
        let bc_core::ImportError::BadValue { field, detail } = err else {
            panic!("expected BadValue, got {err:?}");
        };
        assert_eq!(field, "cost");
        assert_eq!(detail, "plugin returned a negative cost -105 AUD");
    }

    #[test]
    fn a_cost_keeps_its_lot_date_and_label() {
        let cost = wit_cost(wt::Cost {
            basis: wt::Quote::Total(wit_amount("210", "AUD")),
            date: Some(wt::Date {
                year: 2024_i32,
                month: 3_u8,
                day: 1_u8,
            }),
            label: Some("lot-a".to_owned()),
        })
        .expect("cost");
        assert_eq!(
            cost,
            Cost::builder()
                .basis(Quote::Total(Amount::new(dec!(210), "AUD")))
                .date(jiff::civil::date(2024, 3, 1))
                .label("lot-a")
                .build()
        );
    }

    #[test]
    fn a_posting_carries_its_price_and_cost_to_the_core() {
        let t = wt::RawTransaction {
            native: None,
            date: wt::Date {
                year: 2026_i32,
                month: 1_u8,
                day: 15_u8,
            },
            description: "Software".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
            postings: vec![
                wt::RawPosting {
                    native: None,
                    account: "Expenses:Software".to_owned(),
                    amount: Some(wit_amount("4.00", "USD")),
                    balance: None,
                    price: Some(wt::Quote::Total(wit_amount("6.37", "AUD"))),
                    cost: None,
                    tags: vec![],
                    metadata: vec![],
                },
                wt::RawPosting {
                    native: None,
                    account: "Assets:Bank".to_owned(),
                    amount: Some(wit_amount("-6.37", "AUD")),
                    balance: None,
                    price: None,
                    cost: None,
                    tags: vec![],
                    metadata: vec![],
                },
            ],
        };
        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        let [software, bank] =
            <[bc_core::RawPosting; 2]>::try_from(core.postings).expect("two postings");
        assert_eq!(
            software.price,
            Some(Quote::Total(Amount::new(dec!(6.37), "AUD")))
        );
        assert!(software.cost.is_none());
        assert!(bank.price.is_none());
    }

    #[test]
    fn native_transaction_crosses_the_boundary() {
        let t = wt::RawTransaction {
            date: wt::Date {
                year: 2025_i32,
                month: 6_u8,
                day: 27_u8,
            },
            description: "Insurance".to_owned(),
            reference: None,
            tags: vec![],
            metadata: vec![],
            source_location: None,
            postings: vec![wt::RawPosting {
                account: "Expenses:Insurance".to_owned(),
                amount: Some(wit_amount("120.00", "AUD")),
                balance: None,
                price: None,
                cost: None,
                tags: vec![],
                metadata: vec![],
                native: Some(wt::NativePosting {
                    handle: Some("p1".to_owned()),
                    spread_from: Some(wt::Date {
                        year: 2025_i32,
                        month: 7_u8,
                        day: 1_u8,
                    }),
                    spread_until: Some(wt::Date {
                        year: 2026_i32,
                        month: 6_u8,
                        day: 30_u8,
                    }),
                }),
            }],
            native: Some(wt::NativeTransaction {
                handle: Some("a1b".to_owned()),
                reconciliation: Some(wt::Reconciliation::Flagged),
                delete: true,
            }),
        };
        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        let native = core.native.expect("transaction block carried");
        assert_eq!(native.handle.as_deref(), Some("a1b"));
        assert_eq!(
            native.reconciliation,
            Some(bc_models::Reconciliation::Flagged)
        );
        assert!(native.delete);
        let posting = core.postings.first().expect("one posting");
        let leg = posting.native.as_ref().expect("posting block carried");
        assert_eq!(leg.handle.as_deref(), Some("p1"));
        assert_eq!(leg.spread_from, Some(jiff::civil::date(2025, 7, 1)));
        assert_eq!(leg.spread_until, Some(jiff::civil::date(2026, 6, 30)));
    }

    #[test]
    fn an_absent_native_block_stays_absent() {
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
                account: "Expenses:Coffee".to_owned(),
                amount: None,
                balance: None,
                price: None,
                cost: None,
                tags: vec![],
                metadata: vec![],
                native: None,
            }],
            native: None,
        };
        let core = bc_core::RawTransaction::try_from(t).expect("valid");
        assert!(core.native.is_none());
        assert!(core.postings.iter().all(|p| p.native.is_none()));
    }

    // MARK: Export direction

    fn amount(value: &str, commodity: &str) -> Amount {
        Amount::new(
            Decimal::from_str_exact(value).expect("decimal"),
            bc_models::CommodityCode::new(commodity),
        )
    }

    fn entry(key: &str, value: bc_core::RawMetaValue) -> bc_core::RawMetaEntry {
        bc_core::RawMetaEntry::builder()
            .key(bc_models::MetaKey::new(key.to_owned()).expect("key"))
            .value(value)
            .build()
    }

    /// Reads a wire date back into a civil date.
    fn civil(d: wt::Date) -> jiff::civil::Date {
        wit_date(d).expect("valid date")
    }

    fn metadata_back(entries: Vec<wt::MetaEntry>) -> Vec<bc_core::RawMetaEntry> {
        wit_metadata(entries, "test").expect("valid metadata")
    }

    fn snapshot_back(w: we::ExportSnapshot) -> bc_core::ExportSnapshot {
        bc_core::ExportSnapshot {
            level: match w.level {
                we::ExportLevel::Working => bc_core::ExportLevel::Working,
                we::ExportLevel::Archive => bc_core::ExportLevel::Archive,
            },
            accounts: w
                .accounts
                .into_iter()
                .map(|a| bc_core::ExportAccountEntry {
                    path: a.path,
                    opened_on: a.opened_on.map(civil),
                    closed_on: a.closed_on.map(civil),
                    commodities: a.commodities,
                    extra: metadata_back(a.extra),
                })
                .collect(),
            commodities: w
                .commodities
                .into_iter()
                .map(|c| bc_core::ExportCommodityEntry {
                    code: c.code,
                    extra: metadata_back(c.extra),
                })
                .collect(),
            transactions: w
                .transactions
                .into_iter()
                .map(|t| bc_core::ExportTransactionEntry {
                    handle: t.handle,
                    date: civil(t.date),
                    description: t.description,
                    reconciliation: wit_reconciliation(t.reconciliation),
                    tags: t.tags,
                    metadata: metadata_back(t.metadata),
                    postings: t
                        .postings
                        .into_iter()
                        .map(|p| bc_core::ExportPostingEntry {
                            handle: p.handle,
                            account: p.account,
                            amount: p.amount.map(|a| Amount::try_from(a).expect("amount")),
                            price: p.price.map(|q| wit_quote("price", q).expect("price")),
                            cost: p.cost.map(|c| wit_cost(c).expect("cost")),
                            tags: p.tags,
                            metadata: metadata_back(p.metadata),
                            spread_from: p.spread_from.map(civil),
                            spread_until: p.spread_until.map(civil),
                        })
                        .collect(),
                    unbalanced: t.unbalanced,
                })
                .collect(),
            budgets: w
                .budgets
                .into_iter()
                .map(|b| bc_core::ExportBudgetEntry {
                    account: b.account,
                    effective_from: civil(b.effective_from),
                    period: b.period,
                    target: b.target.map(|a| Amount::try_from(a).expect("amount")),
                    expression: b.expression,
                    extra: metadata_back(b.extra),
                })
                .collect(),
            balances: w
                .balances
                .into_iter()
                .map(|b| bc_core::ExportBalanceEntry {
                    date: civil(b.date),
                    account: b.account,
                    amount: Amount::try_from(b.amount).expect("amount"),
                })
                .collect(),
        }
    }

    fn date(year: i16, month: i8, day: i8) -> jiff::civil::Date {
        jiff::civil::date(year, month, day)
    }

    fn full_snapshot() -> bc_core::ExportSnapshot {
        let resolved = |v| bc_core::RawMetaValue::Resolved(v);
        bc_core::ExportSnapshot {
            level: bc_core::ExportLevel::Archive,
            accounts: vec![bc_core::ExportAccountEntry {
                path: "Assets:Bank:123456789".to_owned(),
                opened_on: Some(date(2020, 1, 2)),
                closed_on: Some(date(2024, 12, 31)),
                commodities: vec!["AUD".to_owned(), "USD".to_owned()],
                extra: vec![entry(
                    "bc-kind",
                    resolved(MetaValue::Text("asset".to_owned())),
                )],
            }],
            commodities: vec![bc_core::ExportCommodityEntry {
                code: "AUD".to_owned(),
                extra: vec![entry("bc-precision", resolved(MetaValue::Number(dec!(2))))],
            }],
            transactions: vec![bc_core::ExportTransactionEntry {
                handle: "tx-1".to_owned(),
                date: date(2024, 3, 1),
                description: "Woolworths".to_owned(),
                reconciliation: bc_models::Reconciliation::Flagged,
                tags: vec!["groceries".to_owned()],
                metadata: vec![
                    entry("checked", resolved(MetaValue::Boolean(true))),
                    entry("posted", resolved(MetaValue::Date(date(2024, 3, 2)))),
                    entry(
                        "seen",
                        resolved(MetaValue::Timestamp(
                            "2024-03-01T10:00:00Z".parse().expect("timestamp"),
                        )),
                    ),
                    entry("fee", resolved(MetaValue::Amount(amount("1.50", "AUD")))),
                    entry(
                        "source",
                        bc_core::RawMetaValue::AccountPath("Assets:Bank:123456789".to_owned()),
                    ),
                ],
                postings: vec![
                    bc_core::ExportPostingEntry {
                        handle: Some("p-1".to_owned()),
                        account: "Assets:Brokerage".to_owned(),
                        amount: Some(amount("10.50", "ETF")),
                        price: Some(bc_models::Quote::Total(amount("1050.00", "AUD"))),
                        cost: Some(
                            bc_models::Cost::builder()
                                .basis(bc_models::Quote::PerUnit(amount("100.00", "AUD")))
                                .date(date(2023, 6, 1))
                                .label("lot-a")
                                .build(),
                        ),
                        tags: vec!["lot".to_owned()],
                        metadata: vec![entry("note", resolved(MetaValue::Text("fill".to_owned())))],
                        spread_from: Some(date(2024, 3, 1)),
                        spread_until: Some(date(2024, 3, 31)),
                    },
                    bc_core::ExportPostingEntry {
                        handle: None,
                        account: "Expenses:Groceries".to_owned(),
                        amount: None,
                        price: None,
                        cost: None,
                        tags: Vec::new(),
                        metadata: Vec::new(),
                        spread_from: None,
                        spread_until: None,
                    },
                ],
                // Not what these legs imply; set so a dropped field fails the
                // round trip.
                unbalanced: true,
            }],
            budgets: vec![bc_core::ExportBudgetEntry {
                account: "Expenses:Groceries".to_owned(),
                effective_from: date(2024, 1, 1),
                period: "monthly".to_owned(),
                target: Some(amount("400.00", "AUD")),
                expression: Some("avg(3)".to_owned()),
                extra: vec![entry("bc-rollover", resolved(MetaValue::Boolean(false)))],
            }],
            balances: vec![bc_core::ExportBalanceEntry {
                date: date(2024, 3, 1),
                account: "Assets:Bank:123456789".to_owned(),
                amount: amount("1234.50", "AUD"),
            }],
        }
    }

    #[test]
    fn snapshot_translates_to_wit() {
        let snapshot = full_snapshot();

        let wire = we::ExportSnapshot::try_from(&snapshot).expect("translates");
        // The stated scale survives the crossing.
        let first_amount = wire
            .transactions
            .first()
            .and_then(|t| t.postings.first())
            .and_then(|p| p.amount.as_ref())
            .map(|a| a.value.as_str());
        assert_eq!(first_amount, Some("10.50"));
        assert_eq!(snapshot_back(wire), snapshot);
    }

    #[test]
    fn a_bound_account_id_in_metadata_is_refused() {
        let mut snapshot = bc_core::ExportSnapshot::default();
        snapshot.commodities.push(bc_core::ExportCommodityEntry {
            code: "AUD".to_owned(),
            extra: vec![entry(
                "owner",
                bc_core::RawMetaValue::Resolved(MetaValue::Account(bc_models::AccountId::new())),
            )],
        });

        let err = we::ExportSnapshot::try_from(&snapshot).expect_err("refused");
        assert!(matches!(err, bc_core::ExportError::Write(_)), "got {err:?}");
    }

    #[test]
    fn export_errors_keep_their_meaning() {
        assert!(matches!(
            bc_core::ExportError::from(we::ExportError::Unsupported),
            bc_core::ExportError::Unsupported
        ));
        assert!(matches!(
            bc_core::ExportError::from(we::ExportError::Unspellable(vec!["x".to_owned()])),
            bc_core::ExportError::Unspellable(items) if items == ["x"]
        ));
        assert!(matches!(
            bc_core::ExportError::from(we::ExportError::Other("boom".to_owned())),
            bc_core::ExportError::Plugin(detail) if detail == "boom"
        ));
    }
}
