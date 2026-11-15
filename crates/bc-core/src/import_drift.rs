//! The pure series walk behind import balance drift.
//!
//! A statement series is the rows of one import that touch one account and
//! commodity, each with the balance the source printed. [`check_series`]
//! compares those balances with a ledger balance function at the opening, at
//! each day boundary and at the close, and reports where the difference moves.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::pin::pin;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Balances;
use bc_models::CommodityCode;
use futures_util::TryStreamExt as _;
use jiff::civil::Date;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::AccountPath;
use crate::AccountResolver;
use crate::BcError;
use crate::BcResult;
use crate::CommodityResolver;
use crate::LegFate;
use crate::PlannedRow;
use crate::RawPosting;
use crate::RawTransaction;
use crate::Resolution;
use crate::RowFate;
use crate::import_exec::document_residual;
use crate::legs::LegFilter;
use crate::legs::LegScope;
use crate::legs::resolved_transactions;
use crate::warning::DriftCause;
use crate::warning::DriftKind;
use crate::warning::Warning;

/// One import row's contribution to a series.
#[derive(Debug, Clone)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "the import executor builds and reads these fields directly"
)]
pub(crate) struct SeriesRow {
    /// The caller's index for the row; warnings carry it back.
    pub(crate) row: usize,
    /// The row's date.
    pub(crate) date: Date,
    /// Where the row sits in the source, for display.
    pub(crate) location: Option<String>,
    /// The net of the row's legs on the series account and commodity.
    pub(crate) movement: Decimal,
    /// The balance the source printed, before orientation.
    pub(crate) reported: Decimal,
    /// Set when a series leg was neither booked nor stored.
    pub(crate) missing: Option<DriftCause>,
}

/// The account and commodity a series is stated in.
#[derive(Debug, Clone)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "the import executor builds and reads these fields directly"
)]
pub(crate) struct SeriesKey {
    /// The account's colon-joined path.
    pub(crate) account_path: String,
    /// The commodity code.
    pub(crate) commodity_code: String,
}

/// The outcome of walking one series.
#[derive(Debug, Clone)]
#[expect(
    clippy::field_scoped_visibility_modifiers,
    reason = "the import executor builds and reads these fields directly"
)]
pub(crate) struct SeriesResult {
    /// Drift warnings with the row index each names, if it names one.
    pub(crate) warnings: Vec<(Option<usize>, Warning)>,
    /// The summary of the comparison.
    pub(crate) check: BalanceCheck,
}

/// What an import compared for one series.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct BalanceCheck {
    /// The series account's colon-joined path.
    pub account_path: String,
    /// The commodity the balances are stated in.
    pub commodity_code: String,
    /// The first date of the series.
    pub from: Date,
    /// The last date of the series.
    pub to: Date,
    /// Whether the source and the ledger agreed.
    pub status: BalanceCheckStatus,
}

/// The result of comparing one series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BalanceCheckStatus {
    /// Every checkpoint matched and the difference never moved.
    Agreed,
    /// At least one drift warning was raised.
    Differs,
    /// Nothing was compared, or a checkpoint was skipped (the series account
    /// did not resolve, or a sum overflowed).
    Unchecked,
}

/// The net movement an import run books, by account path and date.
///
/// Both import sinks feed one of these through the same fold, so a plan and a
/// commit report the same dated movement for the same input.
#[derive(Debug, Default)]
pub(crate) struct Movement {
    /// Net movement per `(account path, date)`.
    buckets: BTreeMap<(String, Date), Balances>,
    /// Paths with a bucket that overflowed and so holds a short figure.
    overflowed: BTreeSet<String>,
}

impl Movement {
    /// Adds `amount` to `path`'s bucket on `date`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the bucket overflows; it keeps its last
    /// good value, and [`Self::through`] reports nothing for `path`.
    pub(crate) fn add(&mut self, path: &str, date: Date, amount: &Amount) -> BcResult<()> {
        let added = self.bucket(path, date).try_add(amount);
        added.map_err(|error| {
            self.overflowed.insert(path.to_owned());
            BcError::BadData(error.to_string())
        })
    }

    /// Moves `path`'s bucket on `date` by `add` minus `subtract`, per commodity.
    ///
    /// The bucket is claimed whatever the movement. Every commodity is folded
    /// even after one overflows, so a multi-commodity movement is not dropped
    /// at the first failure.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any commodity overflows; that bucket
    /// keeps its last good value, and [`Self::through`] reports nothing for
    /// `path`.
    pub(crate) fn shift(
        &mut self,
        path: &str,
        date: Date,
        add: Option<&Balances>,
        subtract: Option<&Balances>,
    ) -> BcResult<()> {
        let entry = self.bucket(path, date);
        let mut failure = None;
        for (code, value) in add.into_iter().flat_map(Balances::iter) {
            if let Err(error) = entry.try_add(&Amount::new(value, code)) {
                failure.get_or_insert(error);
            }
        }
        for (code, value) in subtract.into_iter().flat_map(Balances::iter) {
            if let Err(error) = entry.try_sub(&Amount::new(value, code)) {
                failure.get_or_insert(error);
            }
        }
        let Some(error) = failure else {
            return Ok(());
        };
        self.overflowed.insert(path.to_owned());
        Err(BcError::BadData(error.to_string()))
    }

    /// Per-path totals over all dates, sorted by path.
    pub(crate) fn totals(&self) -> BTreeMap<String, Balances> {
        let mut totals: BTreeMap<String, Balances> = BTreeMap::new();
        for ((path, _), balances) in &self.buckets {
            let entry = totals.entry(path.clone()).or_default();
            for (code, value) in balances.iter() {
                // A total past `Decimal`'s range keeps its last good value.
                if entry.try_add(&Amount::new(value, code)).is_err() {
                    tracing::warn!(
                        account = path.as_str(),
                        "the account's planned total overflowed; the reported figure is short"
                    );
                }
            }
        }
        totals
    }

    /// Net movement on `path` in `commodity` dated on or before `date`.
    ///
    /// `None` when the sum, or a bucket of `path`, overflowed.
    pub(crate) fn through(&self, path: &str, commodity: &str, date: Date) -> Option<Decimal> {
        if self.overflowed.contains(path) {
            return None;
        }
        let range = (path.to_owned(), Date::MIN)..=(path.to_owned(), date);
        self.buckets
            .range(range)
            .filter_map(|(_, balances)| balances.get(commodity))
            .try_fold(Decimal::ZERO, Decimal::checked_add)
    }

    /// The bucket for `path` on `date`, created empty if absent.
    fn bucket(&mut self, path: &str, date: Date) -> &mut Balances {
        self.buckets.entry((path.to_owned(), date)).or_default()
    }
}

// MARK: Ledger

/// Per-commodity end-of-day ledger for one account, read once before the write
/// loop.
#[derive(Debug, Default)]
pub(crate) struct DayEnd {
    /// Per-day net, by commodity code and then transaction date.
    nets: BTreeMap<String, BTreeMap<Date, Decimal>>,
    /// Commodities whose per-day net overflowed while reading.
    overflowed: BTreeSet<String>,
}

impl DayEnd {
    /// Reads every stored leg on `account`, elided residuals included.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on a query failure or an unreadable stored leg.
    pub(crate) async fn read(pool: &SqlitePool, account: &AccountId) -> BcResult<Self> {
        let filter = LegFilter::new(LegScope::Accounts(core::slice::from_ref(account)));
        let mut stream = pin!(resolved_transactions(pool, &filter)?);
        let mut day_end = Self::default();
        while let Some(tx) = stream.try_next().await? {
            for leg in tx.legs() {
                if let Some(code) = leg.commodity() {
                    day_end.add(code.as_str(), tx.date(), leg.value());
                }
            }
        }
        Ok(day_end)
    }

    /// Adds `value` to `code`'s net on `date`, marking `code` on overflow.
    fn add(&mut self, code: &str, date: Date, value: Decimal) {
        let net = self
            .nets
            .entry(code.to_owned())
            .or_default()
            .entry(date)
            .or_default();
        if let Some(sum) = net.checked_add(value) {
            *net = sum;
        } else {
            self.overflowed.insert(code.to_owned());
        }
    }

    /// Stored balance in `commodity` at end of `date`.
    ///
    /// `None` when the sum, or a stored day's net, overflowed.
    pub(crate) fn through(&self, commodity: &str, date: Date) -> Option<Decimal> {
        if self.overflowed.contains(commodity) {
            return None;
        }
        self.nets
            .get(commodity)
            .map_or(Some(Decimal::ZERO), |days| {
                days.range(..=date)
                    .try_fold(Decimal::ZERO, |sum, (_, net)| sum.checked_add(*net))
            })
    }
}

// MARK: Series assembly

/// Groups `raws` into series: legs with a `balance`, keyed by (stated path,
/// balance commodity), sorted by key. A row joins each series once, in
/// document order.
pub(crate) fn series_of(raws: &[RawTransaction]) -> Vec<(SeriesKey, Vec<usize>)> {
    let mut groups: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (index, raw) in raws.iter().enumerate() {
        for posting in &raw.postings {
            let Some(balance) = &posting.balance else {
                continue;
            };
            let members = groups
                .entry((
                    posting.account.clone(),
                    balance.commodity().as_str().to_owned(),
                ))
                .or_default();
            if members.last() != Some(&index) {
                members.push(index);
            }
        }
    }
    groups
        .into_iter()
        .map(|((account_path, commodity_code), members)| {
            (
                SeriesKey {
                    account_path,
                    commodity_code,
                },
                members,
            )
        })
        .collect()
}

/// What a resolved series compares against.
#[derive(Debug)]
struct Target {
    /// The rendered path and the registered code.
    key: SeriesKey,
    /// The series account.
    account_id: AccountId,
    /// The account's stored ledger before this run's writes.
    stored: DayEnd,
}

/// One series of a run, prepared before the write loop.
#[derive(Debug)]
pub(crate) struct Series {
    /// The key as the document states it.
    stated: SeriesKey,
    /// Indices into the run's rows, in document order.
    rows: Vec<usize>,
    /// `None` when the account or the commodity does not resolve.
    target: Option<Target>,
}

/// The registered spelling of `code`, if it names a registered commodity.
fn registered<'r>(commodities: &'r CommodityResolver, code: &str) -> Option<&'r str> {
    commodities.resolve(&CommodityCode::new(code.trim()))
}

/// The rendered path of `stated`, if it parses.
fn rendered(stated: &str) -> Option<String> {
    AccountPath::parse(stated).ok().map(|path| path.to_string())
}

/// Resolves each series of `raws` and reads its account's stored ledger.
///
/// An account the run's declarations create has no stored ledger, so it is not
/// read.
///
/// # Arguments
///
/// * `pool` - The pool the ledger is read on.
/// * `accounts` - The run's account view, declarations included.
/// * `commodities` - The run's commodity code resolver.
/// * `created` - Rendered paths the run's declarations create.
/// * `raws` - Parsed transactions in document order.
///
/// # Errors
///
/// Returns [`BcError`] when a ledger read fails.
pub(crate) async fn prepare_series(
    pool: &SqlitePool,
    accounts: &AccountResolver,
    commodities: &CommodityResolver,
    created: &[String],
    raws: &[RawTransaction],
) -> BcResult<Vec<Series>> {
    let mut prepared = Vec::new();
    for (stated, rows) in series_of(raws) {
        let resolved = AccountPath::parse(&stated.account_path)
            .ok()
            .and_then(|path| match accounts.resolve(&path) {
                Resolution::Resolved { id, .. } => Some((path.to_string(), id)),
                Resolution::Missing { .. } => None,
            })
            .zip(registered(commodities, &stated.commodity_code));
        let target = match resolved {
            Some(((account_path, account_id), code)) => {
                let stored = if created.contains(&account_path) {
                    DayEnd::default()
                } else {
                    DayEnd::read(pool, &account_id).await?
                };
                Some(Target {
                    key: SeriesKey {
                        account_path,
                        commodity_code: code.to_owned(),
                    },
                    account_id,
                    stored,
                })
            }
            None => None,
        };
        prepared.push(Series {
            stated,
            rows,
            target,
        });
    }
    Ok(prepared)
}

/// The value `posting` moves the series account by in the series commodity.
///
/// `None` when the leg sits elsewhere or moves another commodity.
///
/// # Errors
///
/// Returns [`BcError::BadData`] when an elided leg's residual overflows.
fn leg_value(
    raw: &RawTransaction,
    posting: &RawPosting,
    key: &SeriesKey,
    commodities: &CommodityResolver,
) -> BcResult<Option<Decimal>> {
    if rendered(&posting.account).as_deref() != Some(key.account_path.as_str()) {
        return Ok(None);
    }
    let amount = match &posting.amount {
        Some(amount) => Some(amount.clone()),
        None => document_residual(raw, commodities)?,
    };
    Ok(amount.and_then(|held| {
        (registered(commodities, held.commodity().as_str()) == Some(key.commodity_code.as_str()))
            .then(|| held.value())
    }))
}

/// Builds the series row for raw row `index`; `None` when its movement
/// overflowed or it states no balance in the series.
fn series_row(
    index: usize,
    raw: &RawTransaction,
    report: Option<&PlannedRow>,
    series: &Series,
    target: &Target,
    commodities: &CommodityResolver,
) -> Option<SeriesRow> {
    // A conflict row's legs are already stored, under several transactions.
    let conflict = report.is_some_and(|row| matches!(row.fate, RowFate::Conflict { .. }));
    let mut movement = Decimal::ZERO;
    let mut missing = None;
    for (leg_index, posting) in raw.postings.iter().enumerate() {
        let Some(value) = leg_value(raw, posting, &target.key, commodities).ok()? else {
            continue;
        };
        movement = movement.checked_add(value)?;
        if missing.is_none() && !conflict {
            missing = report
                .and_then(|row| row.legs.get(leg_index))
                .and_then(|leg| match leg.fate {
                    LegFate::Skipped(cause) => Some(DriftCause::LegSkipped {
                        cause: cause.label().to_owned(),
                    }),
                    LegFate::Rejected => Some(DriftCause::LegDeleted),
                    LegFate::New | LegFate::Stored => None,
                });
        }
    }
    let reported = raw.postings.iter().find_map(|posting| {
        posting
            .balance
            .as_ref()
            .filter(|balance| {
                posting.account == series.stated.account_path
                    && balance.commodity().as_str() == series.stated.commodity_code
            })
            .map(Amount::value)
    })?;
    Some(SeriesRow {
        row: index,
        date: raw.date,
        location: raw
            .source_location
            .as_ref()
            .map(|location| location.display.clone()),
        movement,
        reported,
        missing,
    })
}

/// The ledger balance at the end of every date the walk of `rows` reads.
///
/// `None` when a sum overflowed.
fn ledger_at(
    rows: &[SeriesRow],
    target: &Target,
    movement: &Movement,
) -> Option<BTreeMap<Date, Decimal>> {
    let first = rows.iter().map(|row| row.date).min()?;
    let opening = first.yesterday().unwrap_or(first);
    let key = &target.key;
    rows.iter()
        .map(|row| row.date)
        .chain([opening])
        .map(|date| {
            let stored = target.stored.through(&key.commodity_code, date)?;
            let booked = movement.through(&key.account_path, &key.commodity_code, date)?;
            Some((date, stored.checked_add(booked)?))
        })
        .collect()
}

/// The unchecked result for `series`, spanning its rows' dates.
fn unchecked(key: &SeriesKey, series: &Series, raws: &[RawTransaction]) -> BalanceCheck {
    let dates = series
        .rows
        .iter()
        .filter_map(|&index| raws.get(index).map(|raw| raw.date));
    let from = dates.clone().min().unwrap_or_default();
    let to = dates.max().unwrap_or_default();
    BalanceCheck {
        account_path: key.account_path.clone(),
        commodity_code: key.commodity_code.clone(),
        from,
        to,
        status: BalanceCheckStatus::Unchecked,
    }
}

/// Walks one resolved series; `None` when a sum overflowed.
fn walk(
    series: &Series,
    target: &Target,
    raws: &[RawTransaction],
    reports: &[PlannedRow],
    movement: &Movement,
    commodities: &CommodityResolver,
) -> Option<SeriesResult> {
    let walked = series
        .rows
        .iter()
        .map(|&index| {
            series_row(
                index,
                raws.get(index)?,
                reports.get(index),
                series,
                target,
                commodities,
            )
        })
        .collect::<Option<Vec<SeriesRow>>>()?;
    let ledger = ledger_at(&walked, target, movement)?;
    Some(check_series(
        &target.key,
        &target.account_id,
        &walked,
        |date| ledger.get(&date).copied().unwrap_or_default(),
    ))
}

/// Compares each series with the ledger this run leaves behind.
///
/// Each row a warning names takes the first such warning as its `drift`. A
/// series whose account or commodity does not resolve, or whose sums overflow,
/// is reported unchecked with no warnings.
///
/// # Arguments
///
/// * `series` - The run's series, as [`prepare_series`] left them.
/// * `raws` - Parsed transactions in document order.
/// * `reports` - The run's row reports, index-aligned with `raws`.
/// * `movement` - What this run booked, by date.
/// * `commodities` - The run's commodity code resolver.
///
/// # Returns
///
/// The drift warnings in series order, and one check per series.
pub(crate) fn drift_checks(
    series: &[Series],
    raws: &[RawTransaction],
    reports: &mut [PlannedRow],
    movement: &Movement,
    commodities: &CommodityResolver,
) -> (Vec<Warning>, Vec<BalanceCheck>) {
    let mut warnings = Vec::new();
    let mut checks = Vec::with_capacity(series.len());
    for one in series {
        let Some(target) = &one.target else {
            checks.push(unchecked(&one.stated, one, raws));
            continue;
        };
        let Some(result) = walk(one, target, raws, reports, movement, commodities) else {
            tracing::warn!(
                account = %target.key.account_path,
                commodity = %target.key.commodity_code,
                "a balance series sum overflowed; its check is skipped"
            );
            checks.push(unchecked(&target.key, one, raws));
            continue;
        };
        for (row, warning) in result.warnings {
            if let Some(report) = row.and_then(|index| reports.get_mut(index))
                && report.drift.is_none()
            {
                report.drift = Some(warning.clone());
            }
            warnings.push(warning);
        }
        checks.push(result.check);
    }
    (warnings, checks)
}

/// A row with its balance oriented to the ledger's sign.
#[derive(Debug, Clone, Copy)]
struct Entry<'a> {
    /// The source row.
    row: &'a SeriesRow,
    /// The row's printed balance after orientation.
    signed: Decimal,
}

/// A comparison of the source and the ledger at the end of a date.
#[derive(Debug, Clone)]
struct Point {
    /// The date whose end is compared.
    date: Date,
    /// The oriented source balance.
    reported: Decimal,
    /// The ledger balance.
    derived: Decimal,
    /// `reported - derived`.
    diff: Decimal,
}

impl Point {
    /// `None` when the difference overflows.
    fn new(date: Date, reported: Decimal, derived: Decimal) -> Option<Self> {
        Some(Self {
            date,
            reported,
            derived,
            diff: reported.checked_sub(derived)?,
        })
    }
}

/// The row's printed balance, negated when `positive` is false.
fn oriented(row: &SeriesRow, positive: bool) -> Decimal {
    let mut balance = row.reported;
    if !positive {
        balance.set_sign_positive(balance.is_sign_negative());
    }
    balance
}

/// `true` when `cur - prev` equals the row's movement. An overflow fails.
fn step_holds(prev: Decimal, cur: Decimal, movement: Decimal) -> bool {
    cur.checked_sub(prev) == Some(movement)
}

/// How many steps of `order` follow from their row's movement under `positive`.
fn votes(order: &[&SeriesRow], positive: bool) -> usize {
    order
        .windows(2)
        .filter(|pair| {
            matches!(pair, [a, b]
                if step_holds(oriented(a, positive), oriented(b, positive), b.movement))
        })
        .count()
}

/// The candidate with the most votes; the earliest wins a tie.
fn best_candidate<'a, 'b>(
    candidates: &[(&'b [&'a SeriesRow], bool)],
) -> Option<(&'b [&'a SeriesRow], bool)> {
    let mut best: Option<(usize, &'b [&'a SeriesRow], bool)> = None;
    for &(order, positive) in candidates {
        let count = votes(order, positive);
        if best.is_none_or(|(top, ..)| count > top) {
            best = Some((count, order, positive));
        }
    }
    best.map(|(_, order, positive)| (order, positive))
}

/// Chooses the direction and sign, and returns the rows in that order.
fn orient(rows: &[SeriesRow]) -> Vec<Entry<'_>> {
    let doc: Vec<&SeriesRow> = rows.iter().collect();
    let rev: Vec<&SeriesRow> = rows.iter().rev().collect();
    let mut candidates: Vec<(&[&SeriesRow], bool)> = Vec::new();
    for order in [doc.as_slice(), rev.as_slice()] {
        if order.is_sorted_by_key(|r| r.date) {
            candidates.push((order, true));
            candidates.push((order, false));
        }
    }
    let sort = candidates.is_empty();
    if sort {
        candidates = vec![(doc.as_slice(), true), (doc.as_slice(), false)];
    }
    let Some((order, positive)) = best_candidate(&candidates) else {
        return Vec::new();
    };
    let mut entries: Vec<Entry<'_>> = order
        .iter()
        .map(|&row| Entry {
            row,
            signed: oriented(row, positive),
        })
        .collect();
    if sort {
        entries.sort_by_key(|e| e.row.date);
    }
    entries
}

/// The row, location and cause to name for a day on which the difference moved.
fn name_change(
    day: &[Entry<'_>],
    before: Option<Entry<'_>>,
) -> (Option<usize>, Option<String>, Option<DriftCause>) {
    if let Some(entry) = day.iter().find(|e| e.row.missing.is_some()) {
        return (
            Some(entry.row.row),
            entry.row.location.clone(),
            entry.row.missing.clone(),
        );
    }
    let mut prev = before;
    for &entry in day {
        if let Some(p) = prev
            && !step_holds(p.signed, entry.signed, entry.row.movement)
        {
            return (
                Some(entry.row.row),
                entry.row.location.clone(),
                Some(DriftCause::SourceStep),
            );
        }
        prev = Some(entry);
    }
    (None, None, None)
}

/// Builds a [`Warning::BalanceDrift`] for `point`.
fn drift(
    key: &SeriesKey,
    account_id: &AccountId,
    kind: DriftKind,
    point: &Point,
    since: Option<Date>,
    named: (Option<String>, Option<DriftCause>),
) -> Warning {
    Warning::BalanceDrift {
        account_id: account_id.clone(),
        account_path: key.account_path.clone(),
        commodity_code: key.commodity_code.clone(),
        kind,
        date: point.date,
        since,
        location: named.0,
        cause: named.1,
        reported: point.reported,
        derived: point.derived,
    }
}

/// Packages the warnings with the status they imply.
fn finish(
    key: &SeriesKey,
    from: Date,
    to: Date,
    warnings: Vec<(Option<usize>, Warning)>,
    skipped: bool,
) -> SeriesResult {
    let status = if skipped {
        BalanceCheckStatus::Unchecked
    } else if warnings.is_empty() {
        BalanceCheckStatus::Agreed
    } else {
        BalanceCheckStatus::Differs
    };
    SeriesResult {
        warnings,
        check: BalanceCheck {
            account_path: key.account_path.clone(),
            commodity_code: key.commodity_code.clone(),
            from,
            to,
            status,
        },
    }
}

/// Walks one series and reports where the source and the ledger disagree.
///
/// `ledger(d)` is the derived balance at the end of day `d`. The walk orients
/// the series (direction and sign), compares the opening, each day boundary
/// and the close, and raises a warning where the difference is nonzero at the
/// ends or changes between boundaries. A sum that overflows fails its step or
/// skips its checkpoint, with a `tracing` warning for the checkpoint.
pub(crate) fn check_series(
    key: &SeriesKey,
    account_id: &AccountId,
    rows: &[SeriesRow],
    ledger: impl Fn(Date) -> Decimal,
) -> SeriesResult {
    let entries = orient(rows);
    let mut warnings: Vec<(Option<usize>, Warning)> = Vec::new();
    let (Some(first), Some(last)) = (entries.first(), entries.last()) else {
        let from = Date::default();
        return finish(key, from, from, warnings, false);
    };
    let (from, to) = (first.row.date, last.row.date);

    let opening_date = from.yesterday().unwrap_or(from);
    let mut skipped = false;
    let mut prev: Option<Point> = first
        .signed
        .checked_sub(first.row.movement)
        .and_then(|reported| Point::new(opening_date, reported, ledger(opening_date)));
    if prev.is_none() {
        skipped = true;
        tracing::warn!(
            account = %key.account_path,
            date = %opening_date,
            "opening balance difference overflowed; checkpoint skipped"
        );
    }
    if let Some(opening) = &prev
        && !opening.diff.is_zero()
    {
        warnings.push((
            None,
            drift(
                key,
                account_id,
                DriftKind::Opening,
                opening,
                None,
                (None, None),
            ),
        ));
    }

    let mut before: Option<Entry<'_>> = None;
    let mut closing: Option<Point> = None;
    for day in entries.chunk_by(|a, b| a.row.date == b.row.date) {
        let Some(&end) = day.last() else { continue };
        if let Some(point) = Point::new(end.row.date, end.signed, ledger(end.row.date)) {
            if let Some(p) = &prev
                && p.diff != point.diff
            {
                let (row, location, cause) = name_change(day, before);
                warnings.push((
                    row,
                    drift(
                        key,
                        account_id,
                        DriftKind::Changed { before: p.diff },
                        &point,
                        Some(p.date),
                        (location, cause),
                    ),
                ));
            }
            prev = Some(point.clone());
            closing = Some(point);
        } else {
            skipped = true;
            tracing::warn!(
                account = %key.account_path,
                date = %end.row.date,
                "balance difference overflowed; checkpoint skipped"
            );
        }
        before = Some(end);
    }

    if let Some(point) = &closing
        && !point.diff.is_zero()
    {
        warnings.push((
            None,
            drift(
                key,
                account_id,
                DriftKind::Closing,
                point,
                None,
                (None, None),
            ),
        ));
    }

    finish(key, from, to, warnings, skipped)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::BTreeMap;

    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    fn key() -> SeriesKey {
        SeriesKey {
            account_path: "Assets:Bank".to_owned(),
            commodity_code: "AUD".to_owned(),
        }
    }

    fn rows(spec: &[(Date, Decimal, Decimal)]) -> Vec<SeriesRow> {
        spec.iter()
            .enumerate()
            .map(|(i, &(date, movement, reported))| SeriesRow {
                row: i,
                date,
                location: Some(format!("row {}", i.saturating_add(1))),
                movement,
                reported,
                missing: None,
            })
            .collect()
    }

    /// A ledger with `opening` before every listed day and the per-day nets.
    fn ledger(opening: Decimal, days: &[(Date, Decimal)]) -> impl Fn(Date) -> Decimal + use<> {
        let map: BTreeMap<Date, Decimal> = days.iter().copied().collect();
        move |d| {
            map.range(..=d)
                .fold(opening, |acc, (_, net)| acc.saturating_add(*net))
        }
    }

    fn walk(rows: &[SeriesRow], ledger: impl Fn(Date) -> Decimal) -> SeriesResult {
        check_series(&key(), &AccountId::new(), rows, ledger)
    }

    fn kinds(result: &SeriesResult) -> Vec<&DriftKind> {
        result
            .warnings
            .iter()
            .filter_map(|(_, w)| {
                if let Warning::BalanceDrift { kind, .. } = w {
                    Some(kind)
                } else {
                    None
                }
            })
            .collect()
    }

    #[test]
    fn clean_month_agrees() {
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 2), dec!(-30), dec!(50)),
        ]);
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 2), dec!(-30)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(result.warnings, Vec::new());
        assert_eq!(result.check.status, BalanceCheckStatus::Agreed);
        assert_eq!(result.check.from, date(2026, 10, 1));
        assert_eq!(result.check.to, date(2026, 10, 2));
    }

    #[test]
    fn standing_offset_warns_open_and_close() {
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 2), dec!(-30), dec!(50)),
        ]);
        let l = ledger(
            dec!(50),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 2), dec!(-30)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![&DriftKind::Opening, &DriftKind::Closing]
        );
        for (row, w) in &result.warnings {
            assert_eq!(*row, None);
            let Warning::BalanceDrift {
                reported, derived, ..
            } = w
            else {
                panic!("not drift: {w:?}");
            };
            assert_eq!(reported.checked_sub(*derived), Some(dec!(50)));
        }
        let Some((_, Warning::BalanceDrift { date: d, .. })) = result.warnings.first() else {
            panic!("no opening");
        };
        assert_eq!(*d, date(2026, 9, 30));
        assert_eq!(result.check.status, BalanceCheckStatus::Differs);
    }

    #[test]
    fn dropped_row_names_the_next_row() {
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 3), dec!(-30), dec!(40)),
        ]);
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 3), dec!(-30)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![&DriftKind::Changed { before: dec!(0) }, &DriftKind::Closing]
        );
        let Some((
            Some(row),
            Warning::BalanceDrift {
                date: d,
                since,
                location,
                cause,
                ..
            },
        )) = result.warnings.first()
        else {
            panic!("no changed warning");
        };
        assert_eq!(*row, 1);
        assert_eq!(*d, date(2026, 10, 3));
        assert_eq!(*since, Some(date(2026, 10, 1)));
        assert_eq!(location.as_deref(), Some("row 2"));
        assert_eq!(*cause, Some(DriftCause::SourceStep));
        let Some((
            _,
            Warning::BalanceDrift {
                reported, derived, ..
            },
        )) = result.warnings.get(1)
        else {
            panic!("no closing");
        };
        assert_eq!(reported.checked_sub(*derived), Some(dec!(-10)));
    }

    #[test]
    fn duplicated_row_names_the_duplicate() {
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 1), dec!(-20), dec!(80)),
        ]);
        let l = ledger(dec!(100), &[(date(2026, 10, 1), dec!(-40))]);
        let result = walk(&r, l);
        let Some((
            Some(row),
            Warning::BalanceDrift {
                kind,
                date: d,
                location,
                cause,
                ..
            },
        )) = result.warnings.first()
        else {
            panic!("no changed warning");
        };
        assert_eq!(*row, 1);
        assert_eq!(location.as_deref(), Some("row 2"));
        assert_eq!(*kind, DriftKind::Changed { before: dec!(0) });
        assert_eq!(*d, date(2026, 10, 1));
        assert_eq!(*cause, Some(DriftCause::SourceStep));
    }

    #[test]
    fn missing_leg_names_its_row() {
        let mut r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 2), dec!(-30), dec!(50)),
            (date(2026, 10, 3), dec!(-10), dec!(40)),
        ]);
        let skipped = DriftCause::LegSkipped {
            cause: "unresolved commodity".into(),
        };
        if let Some(row) = r.get_mut(1) {
            row.missing = Some(skipped.clone());
        }
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 3), dec!(-10)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![&DriftKind::Changed { before: dec!(0) }, &DriftKind::Closing]
        );
        let Some((
            Some(row),
            Warning::BalanceDrift {
                location, cause, ..
            },
        )) = result.warnings.first()
        else {
            panic!("no changed warning");
        };
        assert_eq!(*row, 1);
        assert_eq!(location.as_deref(), Some("row 2"));
        assert_eq!(*cause, Some(skipped));
    }

    #[test]
    fn hand_entered_ledger_leg_names_the_day() {
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 2), dec!(-30), dec!(50)),
            (date(2026, 10, 3), dec!(-10), dec!(40)),
        ]);
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 2), dec!(-35)),
                (date(2026, 10, 3), dec!(-10)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![&DriftKind::Changed { before: dec!(0) }, &DriftKind::Closing]
        );
        let Some((
            row,
            Warning::BalanceDrift {
                date: d,
                location,
                cause,
                ..
            },
        )) = result.warnings.first()
        else {
            panic!("no changed warning");
        };
        assert_eq!(*row, None);
        assert_eq!(*d, date(2026, 10, 2));
        assert_eq!(*location, None);
        assert_eq!(*cause, None);
    }

    #[test]
    fn drop_then_duplicate_cancel_at_ends() {
        // The source omits a -10 on 10-02 (balances still include it) and
        // prints the 10-04 row twice; the ledger books both copies.
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 3), dec!(-30), dec!(40)),
            (date(2026, 10, 4), dec!(-10), dec!(30)),
            (date(2026, 10, 4), dec!(-10), dec!(30)),
        ]);
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 3), dec!(-30)),
                (date(2026, 10, 4), dec!(-20)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![
                &DriftKind::Changed { before: dec!(0) },
                &DriftKind::Changed { before: dec!(-10) },
            ]
        );
        assert_eq!(result.check.status, BalanceCheckStatus::Differs);
    }

    #[test]
    fn orients_newest_first_inverted_sign() {
        let r = rows(&[
            (date(2026, 10, 2), dec!(-30), dec!(-50)),
            (date(2026, 10, 1), dec!(-20), dec!(-80)),
        ]);
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 2), dec!(-30)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(result.warnings, Vec::new());
        assert_eq!(result.check.status, BalanceCheckStatus::Agreed);
    }

    #[test]
    fn single_row_uses_printed_sign() {
        let r = rows(&[(date(2026, 10, 1), dec!(-20), dec!(80))]);
        let l = ledger(dec!(100), &[(date(2026, 10, 1), dec!(-20))]);
        let result = walk(&r, l);
        assert_eq!(result.warnings, Vec::new());
        assert_eq!(result.check.status, BalanceCheckStatus::Agreed);
    }

    #[test]
    fn overflowing_balance_skips_the_checkpoint() {
        let r = rows(&[(date(2026, 10, 1), dec!(-20), Decimal::MAX)]);
        let l = ledger(Decimal::MIN, &[(date(2026, 10, 1), dec!(-20))]);
        let result = walk(&r, l);
        assert_eq!(result.warnings, Vec::new());
        assert_eq!(result.check.status, BalanceCheckStatus::Unchecked);
    }

    #[test]
    fn overflowing_last_day_still_closes_on_the_last_valid_checkpoint() {
        let r = rows(&[
            (date(2026, 10, 1), dec!(-20), dec!(80)),
            (date(2026, 10, 2), dec!(-30), Decimal::MAX),
        ]);
        let l = ledger(
            dec!(50),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 2), Decimal::MIN),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![&DriftKind::Opening, &DriftKind::Closing]
        );
        let Some((_, Warning::BalanceDrift { date: d, .. })) = result.warnings.get(1) else {
            panic!("no closing");
        };
        assert_eq!(*d, date(2026, 10, 1));
        assert_eq!(result.check.status, BalanceCheckStatus::Unchecked);
    }

    #[test]
    fn unorderable_dates_keep_document_order_with_the_vote_sign_then_sort_by_date() {
        // Dates go up then down, so neither direction is admissible. The
        // printed balances are negated; the duplicate on 10-03 must stay in
        // document order after the stable sort, so the second copy is named.
        let r = rows(&[
            (date(2026, 10, 2), dec!(-30), dec!(-50)),
            (date(2026, 10, 3), dec!(-10), dec!(-40)),
            (date(2026, 10, 3), dec!(-10), dec!(-40)),
            (date(2026, 10, 1), dec!(-20), dec!(-80)),
        ]);
        let l = ledger(
            dec!(100),
            &[
                (date(2026, 10, 1), dec!(-20)),
                (date(2026, 10, 2), dec!(-30)),
                (date(2026, 10, 3), dec!(-20)),
            ],
        );
        let result = walk(&r, l);
        assert_eq!(
            kinds(&result),
            vec![&DriftKind::Changed { before: dec!(0) }, &DriftKind::Closing]
        );
        let Some((Some(row), Warning::BalanceDrift { cause, .. })) = result.warnings.first() else {
            panic!("no changed warning");
        };
        assert_eq!(*row, 2);
        assert_eq!(*cause, Some(DriftCause::SourceStep));
        assert_eq!(result.check.from, date(2026, 10, 1));
    }

    #[test]
    fn a_tie_goes_to_document_order_and_printed_sign() {
        // Both (doc, +) and (rev, -) explain one step; (doc, +) wins.
        let r = rows(&[
            (date(2026, 10, 1), dec!(10), dec!(90)),
            (date(2026, 10, 1), dec!(10), dec!(100)),
        ]);
        let l = ledger(dec!(80), &[(date(2026, 10, 1), dec!(20))]);
        let result = walk(&r, l);
        assert_eq!(result.warnings, Vec::new());
        assert_eq!(result.check.status, BalanceCheckStatus::Agreed);
    }
}
