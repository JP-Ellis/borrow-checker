//! Typed changes one scope asks for, and the postings they build.

use core::str::FromStr as _;

use bc_models::MetaKey;
use jiff::civil::Date;

use super::scope::Flag;
use super::scope::Written;
use crate::commands::meta;
use crate::error::CliError;
use crate::error::CliResult;

/// What one scope's modifiers ask for, typed but not yet looked up.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Changes {
    /// `--date`.
    pub date: Option<Date>,
    /// `--description`.
    pub description: Option<String>,
    /// `--account`, as typed.
    pub account: Option<String>,
    /// `--amount`.
    pub amount: Option<bc_models::Amount>,
    /// `--cost` or `--total-cost`.
    pub cost: Option<bc_models::Quote>,
    /// `--lot-date`.
    pub lot_date: Option<Date>,
    /// `--lot-label`.
    pub lot_label: Option<String>,
    /// `--no-cost`.
    pub no_cost: bool,
    /// `--price` or `--total-price`.
    pub price: Option<bc_models::Quote>,
    /// `--no-price`.
    pub no_price: bool,
    /// `--spread`.
    pub spread: Option<(Date, Date)>,
    /// `--no-spread`.
    pub no_spread: bool,
    /// Each `--meta`, key parsed, value as typed.
    pub meta: Vec<(MetaKey, String)>,
    /// Each `--clear-meta`.
    pub clear_meta: Vec<MetaKey>,
    /// Each `--tag`, as typed.
    pub tags: Vec<String>,
    /// Each `--untag`, as typed.
    pub untags: Vec<String>,
}

/// Parses `value` and `code` as an amount.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when `value` is not a decimal.
pub(super) fn amount_of(value: &str, code: &str) -> CliResult<bc_models::Amount> {
    let number = rust_decimal::Decimal::from_str(value)
        .map_err(|e| CliError::Arg(format!("invalid amount '{value}': {e}")))?;
    Ok(bc_models::Amount::new(
        number,
        bc_models::CommodityCode::new(code),
    ))
}

/// Parses a `YYYY-MM-DD` date.
fn date_of(text: &str) -> CliResult<Date> {
    Date::from_str(text).map_err(|e| CliError::Arg(format!("invalid date '{text}': {e}")))
}

/// Refuses a negative cost or price; `what` names which.
fn non_negative(amount: bc_models::Amount, what: &str) -> Result<bc_models::Amount, String> {
    if amount.value().is_sign_negative() && !amount.value().is_zero() {
        return Err(format!("negative {what} not allowed"));
    }
    Ok(amount)
}

/// Stores `value` in `slot`, refusing a second one.
fn once<T>(slot: &mut Option<T>, value: T, flag: Flag) -> Result<(), String> {
    if slot.is_some() {
        return Err(format!("--{} is given twice", flag.long()));
    }
    *slot = Some(value);
    Ok(())
}

impl Changes {
    /// Types `modifiers`, the modifiers of the scope `context` names.
    ///
    /// # Errors
    ///
    /// Returns [`CliError::Arg`], prefixed with `context`, for a malformed
    /// value, a single-valued flag given twice, `--cost` with `--total-cost`,
    /// `--price` with `--total-price`, a `--no-*` flag beside what it clears,
    /// one key under `--meta` and `--clear-meta`, one tag under `--tag` and
    /// `--untag`, or a spread whose `FROM` is after its `UNTIL`.
    pub(super) fn from_written(modifiers: &[Written], context: &str) -> CliResult<Self> {
        Self::build(modifiers).map_err(|e| CliError::Arg(format!("{context}: {e}")))
    }

    /// [`Self::from_written`] without the context prefix.
    fn build(modifiers: &[Written]) -> Result<Self, String> {
        let mut out = Self::default();
        let mut cost_kind: Option<Flag> = None;
        let mut price_kind: Option<Flag> = None;
        for item in modifiers {
            let v = |i: usize| item.values.get(i).map_or("", String::as_str);
            let err = |e: CliError| e.to_string();
            match item.flag {
                Flag::Date => once(&mut out.date, date_of(v(0)).map_err(err)?, item.flag)?,
                Flag::Description => once(&mut out.description, v(0).to_owned(), item.flag)?,
                Flag::Account => once(&mut out.account, v(0).to_owned(), item.flag)?,
                Flag::Amount => once(
                    &mut out.amount,
                    amount_of(v(0), v(1)).map_err(err)?,
                    item.flag,
                )?,
                Flag::Cost | Flag::TotalCost => {
                    if let Some(first) = cost_kind.replace(item.flag)
                        && first != item.flag
                    {
                        return Err("--cost and --total-cost cannot both be given".into());
                    }
                    let amount = non_negative(amount_of(v(0), v(1)).map_err(err)?, "cost")?;
                    let quote = if item.flag == Flag::Cost {
                        bc_models::Quote::PerUnit(amount)
                    } else {
                        bc_models::Quote::Total(amount)
                    };
                    once(&mut out.cost, quote, item.flag)?;
                }
                Flag::Price | Flag::TotalPrice => {
                    if let Some(first) = price_kind.replace(item.flag)
                        && first != item.flag
                    {
                        return Err("--price and --total-price cannot both be given".into());
                    }
                    let amount = non_negative(amount_of(v(0), v(1)).map_err(err)?, "price")?;
                    let quote = if item.flag == Flag::Price {
                        bc_models::Quote::PerUnit(amount)
                    } else {
                        bc_models::Quote::Total(amount)
                    };
                    once(&mut out.price, quote, item.flag)?;
                }
                Flag::LotDate => once(&mut out.lot_date, date_of(v(0)).map_err(err)?, item.flag)?,
                Flag::LotLabel => once(&mut out.lot_label, v(0).to_owned(), item.flag)?,
                Flag::Spread => {
                    let (from, until) = (date_of(v(0)).map_err(err)?, date_of(v(1)).map_err(err)?);
                    if from > until {
                        return Err(format!("--spread FROM {from} is after UNTIL {until}"));
                    }
                    once(&mut out.spread, (from, until), item.flag)?;
                }
                Flag::NoCost => out.no_cost = true,
                Flag::NoPrice => out.no_price = true,
                Flag::NoSpread => out.no_spread = true,
                Flag::Meta => {
                    let (key, value) = meta::parse_meta_arg(v(0)).map_err(err)?;
                    out.meta.push((key, value));
                }
                Flag::ClearMeta => out
                    .clear_meta
                    .push(meta::parse_meta_key(v(0)).map_err(err)?),
                Flag::Tag => out.tags.push(v(0).to_owned()),
                Flag::Untag => out.untags.push(v(0).to_owned()),
                Flag::Id | Flag::Find | Flag::Posting | Flag::Add | Flag::Set | Flag::Remove => {}
            }
        }
        if out.no_cost && (out.cost.is_some() || out.lot_date.is_some() || out.lot_label.is_some())
        {
            return Err("--no-cost cannot sit beside a cost flag".into());
        }
        if out.no_price && out.price.is_some() {
            return Err("--no-price cannot sit beside a price flag".into());
        }
        if out.no_spread && out.spread.is_some() {
            return Err("--no-spread cannot sit beside --spread".into());
        }
        if let Some((key, _)) = out.meta.iter().find(|(k, _)| out.clear_meta.contains(k)) {
            return Err(format!("--meta and --clear-meta both name '{key}'"));
        }
        if let Some(tag) = out.tags.iter().find(|t| out.untags.contains(t)) {
            return Err(format!("--tag and --untag both name '{tag}'"));
        }
        Ok(out)
    }

    /// Whether no modifier was given.
    #[cfg_attr(not(test), expect(dead_code, reason = "used by transaction edit"))]
    pub(super) fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A scope's changes with accounts, metadata and tags looked up.
#[derive(Debug, Clone)]
pub(super) struct Resolved {
    /// The typed changes.
    pub changes: Changes,
    /// `--account`, resolved.
    #[cfg_attr(not(test), expect(dead_code, reason = "used by transaction edit"))]
    pub account: Option<bc_models::AccountId>,
    /// Each `--meta`, typed by the key registry, in order.
    pub entries: Vec<bc_models::MetaEntry>,
    /// Each `--tag`, resolved.
    pub tags: Vec<bc_models::TagId>,
    /// Each `--untag` that names an existing tag.
    #[cfg_attr(not(test), expect(dead_code, reason = "used by transaction edit"))]
    pub untags: Vec<bc_models::TagId>,
}

/// `stored` with `add` appended where absent and `remove` dropped, order kept.
pub(super) fn retag(
    stored: &[bc_models::TagId],
    add: &[bc_models::TagId],
    remove: &[bc_models::TagId],
) -> Vec<bc_models::TagId> {
    let mut out: Vec<bc_models::TagId> = stored
        .iter()
        .filter(|t| !remove.contains(t))
        .cloned()
        .collect();
    for tag in add {
        if !out.contains(tag) {
            out.push(tag.clone());
        }
    }
    out
}

/// The cost `changes` leave on a leg that held `stored`.
///
/// A lot date or label the changes do not name comes from `stored`.
fn cost_of(
    stored: Option<&bc_models::Cost>,
    changes: &Changes,
    context: &str,
) -> CliResult<Option<bc_models::Cost>> {
    if changes.no_cost {
        return Ok(None);
    }
    let restated = changes.lot_date.is_some() || changes.lot_label.is_some();
    let basis = match (&changes.cost, stored) {
        (Some(basis), _) => basis.clone(),
        (None, held) if !restated => return Ok(held.cloned()),
        (None, Some(held)) => held.basis().clone(),
        (None, None) => {
            return Err(CliError::Arg(format!(
                "{context}: --lot-date and --lot-label need a cost"
            )));
        }
    };
    let date = changes
        .lot_date
        .or_else(|| stored.and_then(bc_models::Cost::date));
    let label = changes.lot_label.clone().or_else(|| {
        stored
            .and_then(bc_models::Cost::label)
            .map(ToOwned::to_owned)
    });
    Ok(Some(
        bc_models::Cost::builder()
            .basis(basis)
            .maybe_date(date)
            .maybe_label(label)
            .build(),
    ))
}

/// Builds a new leg on `account` from `resolved`.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when a lot date or label has no cost.
pub(super) fn new_posting(
    account: bc_models::AccountId,
    amount: Option<bc_models::Amount>,
    resolved: &Resolved,
    context: &str,
) -> CliResult<bc_models::Posting> {
    let changes = &resolved.changes;
    Ok(bc_models::Posting::builder()
        .id(bc_models::PostingId::new())
        .account_id(account)
        .maybe_amount(amount)
        .maybe_cost(cost_of(None, changes, context)?)
        .maybe_price(changes.price.clone())
        .metadata(bc_models::Metadata::new(resolved.entries.clone()))
        .tag_ids(retag(&[], &resolved.tags, &[]))
        .maybe_spread_from(changes.spread.map(|(from, _)| from))
        .maybe_spread_until(changes.spread.map(|(_, until)| until))
        .build())
}

/// Applies `resolved` to `stored`, keeping everything it does not name.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when a lot date or label has no cost to attach to.
#[cfg_attr(not(test), expect(dead_code, reason = "used by transaction edit"))]
pub(super) fn set_posting(
    stored: &bc_models::Posting,
    resolved: &Resolved,
    context: &str,
) -> CliResult<bc_models::Posting> {
    let changes = &resolved.changes;
    let spread = if changes.no_spread {
        None
    } else {
        changes
            .spread
            .or(stored.spread_from().zip(stored.spread_until()))
    };
    let price = if changes.no_price {
        None
    } else {
        changes.price.clone().or_else(|| stored.price().cloned())
    };
    Ok(bc_models::Posting::builder()
        .id(stored.id().clone())
        .account_id(
            resolved
                .account
                .clone()
                .unwrap_or_else(|| stored.account_id().clone()),
        )
        .maybe_amount(changes.amount.clone().or_else(|| stored.amount().cloned()))
        .maybe_cost(cost_of(stored.cost(), changes, context)?)
        .maybe_price(price)
        .metadata(meta::apply_changes(
            stored.metadata(),
            &resolved.entries,
            &changes.clear_meta,
        ))
        .tag_ids(retag(stored.tag_ids(), &resolved.tags, &resolved.untags))
        .maybe_spread_from(spread.map(|(from, _)| from))
        .maybe_spread_until(spread.map(|(_, until)| until))
        .build())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::super::scope::Flag;
    use super::super::scope::Written;
    use super::Changes;
    use super::Resolved;
    use super::new_posting;
    use super::retag;
    use super::set_posting;

    fn w(flag: Flag, values: &[&str]) -> Written {
        Written {
            flag,
            values: values.iter().map(|v| (*v).to_owned()).collect(),
        }
    }

    fn aud(value: rust_decimal::Decimal) -> bc_models::Amount {
        bc_models::Amount::new(value, bc_models::CommodityCode::new("AUD"))
    }

    fn resolved(changes: Changes) -> Resolved {
        Resolved {
            changes,
            account: None,
            entries: Vec::new(),
            tags: Vec::new(),
            untags: Vec::new(),
        }
    }

    #[rstest]
    #[case::two_costs(&[w(Flag::Cost, &["1", "AUD"]), w(Flag::TotalCost, &["2", "AUD"])], "--cost and --total-cost")]
    #[case::two_prices(&[w(Flag::Price, &["1", "AUD"]), w(Flag::TotalPrice, &["2", "AUD"])], "--price and --total-price")]
    #[case::no_cost_and_cost(&[w(Flag::NoCost, &[]), w(Flag::LotDate, &["2026-01-01"])], "--no-cost")]
    #[case::no_price_and_price(&[w(Flag::NoPrice, &[]), w(Flag::Price, &["1", "AUD"])], "--no-price")]
    #[case::no_spread_and_spread(&[w(Flag::NoSpread, &[]), w(Flag::Spread, &["2026-01-01", "2026-12-31"])], "--no-spread")]
    #[case::set_and_clear(&[w(Flag::Meta, &["note=x"]), w(Flag::ClearMeta, &["note"])], "both name 'note'")]
    #[case::tag_and_untag(&[w(Flag::Tag, &["person:a"]), w(Flag::Untag, &["person:a"])], "both name 'person:a'")]
    #[case::twice(&[w(Flag::Account, &["A"]), w(Flag::Account, &["B"])], "--account is given twice")]
    #[case::inverted_spread(&[w(Flag::Spread, &["2026-12-31", "2026-01-01"])], "FROM 2026-12-31 is after UNTIL 2026-01-01")]
    #[case::bad_amount(&[w(Flag::Amount, &["abc", "AUD"])], "invalid amount 'abc'")]
    #[case::no_equals(&[w(Flag::Meta, &["note"])], "expected KEY=VALUE")]
    #[case::negative_cost(&[w(Flag::Cost, &["-105", "AUD"])], "negative cost not allowed")]
    #[case::negative_total_cost(&[w(Flag::TotalCost, &["-210", "AUD"])], "negative cost not allowed")]
    #[case::negative_price(&[w(Flag::Price, &["-150", "AUD"])], "negative price not allowed")]
    #[case::negative_total_price(&[w(Flag::TotalPrice, &["-6.37", "AUD"])], "negative price not allowed")]
    fn from_written_rejects(#[case] modifiers: &[Written], #[case] expected: &str) {
        let err = Changes::from_written(modifiers, "--set X")
            .expect_err("rejects")
            .to_string();
        assert!(err.contains(expected), "got: {err}");
        assert!(err.contains("--set X"), "names the scope, got: {err}");
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn from_written_types_every_modifier() {
        let changes = Changes::from_written(
            &[
                w(Flag::Cost, &["105", "AUD"]),
                w(Flag::LotDate, &["2026-02-01"]),
                w(Flag::LotLabel, &["lot-a"]),
                w(Flag::TotalPrice, &["6.37", "AUD"]),
                w(Flag::Spread, &["2026-01-01", "2026-12-31"]),
                w(Flag::Meta, &["note=a=b"]),
                w(Flag::Tag, &["person:a"]),
            ],
            "--add X",
        )
        .expect("types");
        assert_eq!(
            changes.cost,
            Some(bc_models::Quote::PerUnit(aud(dec!(105))))
        );
        assert_eq!(changes.lot_date, Some(date(2026, 2, 1)));
        assert_eq!(changes.lot_label.as_deref(), Some("lot-a"));
        assert_eq!(
            changes.price,
            Some(bc_models::Quote::Total(aud(dec!(6.37))))
        );
        assert_eq!(changes.spread, Some((date(2026, 1, 1), date(2026, 12, 31))));
        assert_eq!(changes.meta.len(), 1);
        assert_eq!(changes.meta[0].1, "a=b");
        assert_eq!(changes.tags, vec!["person:a".to_owned()]);
    }

    #[test]
    fn is_empty_only_without_modifiers() {
        assert!(
            Changes::from_written(&[], "--set X")
                .expect("types")
                .is_empty()
        );
        let switch = Changes::from_written(&[w(Flag::NoPrice, &[])], "--set X").expect("types");
        assert!(!switch.is_empty());
    }

    fn stored() -> bc_models::Posting {
        bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(bc_models::AccountId::new())
            .amount(aud(dec!(50)))
            .cost(
                bc_models::Cost::builder()
                    .basis(bc_models::Quote::PerUnit(aud(dec!(1))))
                    .date(date(2026, 1, 1))
                    .label("lot-a")
                    .build(),
            )
            .price(bc_models::Quote::PerUnit(aud(dec!(2))))
            .metadata(bc_models::Metadata::new(vec![bc_models::MetaEntry::new(
                bc_models::MetaKey::new("note").expect("valid key"),
                bc_models::MetaValue::Text("a".to_owned()),
            )]))
            .tag_ids(vec![bc_models::TagId::new()])
            .spread_from(date(2026, 1, 1))
            .spread_until(date(2026, 1, 31))
            .build()
    }

    #[test]
    fn set_keeps_what_it_does_not_name() {
        let before = stored();
        let changes = Changes {
            amount: Some(aud(dec!(45))),
            ..Changes::default()
        };
        let after = set_posting(&before, &resolved(changes), "--set X").expect("sets");
        assert_eq!(after.id(), before.id());
        assert_eq!(after.account_id(), before.account_id());
        assert_eq!(after.amount(), Some(&aud(dec!(45))));
        assert_eq!(after.cost(), before.cost());
        assert_eq!(after.price(), before.price());
        assert_eq!(after.tag_ids(), before.tag_ids());
        assert_eq!(after.spread_from(), before.spread_from());
        assert_eq!(after.spread_until(), before.spread_until());
        assert_eq!(after.metadata(), before.metadata());
    }

    #[test]
    fn a_new_basis_keeps_the_stored_lot_date_and_label() {
        let before = stored();
        let changes = Changes {
            cost: Some(bc_models::Quote::PerUnit(aud(dec!(2)))),
            ..Changes::default()
        };
        let after = set_posting(&before, &resolved(changes), "--set X").expect("sets");
        let cost = after.cost().expect("cost kept");
        assert_eq!(cost.basis(), &bc_models::Quote::PerUnit(aud(dec!(2))));
        assert_eq!(cost.date(), Some(date(2026, 1, 1)));
        assert_eq!(cost.label(), Some("lot-a"));
    }

    #[test]
    fn set_clears_what_no_flags_name() {
        let before = stored();
        let changes = Changes {
            no_cost: true,
            no_price: true,
            no_spread: true,
            ..Changes::default()
        };
        let after = set_posting(&before, &resolved(changes), "--set X").expect("sets");
        assert_eq!(after.cost(), None);
        assert_eq!(after.price(), None);
        assert_eq!(after.spread_from(), None);
        assert_eq!(after.spread_until(), None);
    }

    #[test]
    fn a_lot_date_alone_rewrites_the_stored_cost() {
        let before = stored();
        let changes = Changes {
            lot_date: Some(date(2026, 2, 2)),
            ..Changes::default()
        };
        let after = set_posting(&before, &resolved(changes), "--set X").expect("sets");
        let cost = after.cost().expect("cost kept");
        assert_eq!(cost.basis(), before.cost().expect("stored").basis());
        assert_eq!(cost.date(), Some(date(2026, 2, 2)));
    }

    #[test]
    fn a_lot_date_without_any_cost_is_refused() {
        let changes = Changes {
            lot_date: Some(date(2026, 2, 2)),
            ..Changes::default()
        };
        let err = new_posting(
            bc_models::AccountId::new(),
            Some(aud(dec!(5))),
            &resolved(changes),
            "--add X",
        )
        .expect_err("refuses")
        .to_string();
        assert!(
            err.contains("--add X: --lot-date and --lot-label need a cost"),
            "got: {err}"
        );
    }

    #[test]
    fn retag_adds_once_and_removes_only_what_it_names() {
        let a = bc_models::TagId::new();
        let b = bc_models::TagId::new();
        let c = bc_models::TagId::new();
        assert_eq!(
            retag(&[a.clone(), b.clone()], &[a.clone(), c.clone()], &[b]),
            vec![a, c]
        );
    }
}
