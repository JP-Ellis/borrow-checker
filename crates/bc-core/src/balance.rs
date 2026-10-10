//! Balance calculation engine.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::pin::pin;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Balances;
use bc_models::CommodityCode;
use futures_util::TryStreamExt as _;
use rust_decimal::Decimal;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::legs::Interned;
use crate::legs::LegFilter;
use crate::legs::LegScope;
use crate::legs::LegSource;
use crate::legs::ResolvedLeg;
use crate::legs::ResolvedTransaction;
use crate::legs::resolved_transactions;

/// A single time-bucket of posting aggregation data.
///
/// Used for sparklines and period-based cash-flow summaries.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct PostingBucket {
    /// Inclusive start of the bucket period.
    pub start: jiff::civil::Date,
    /// Exclusive end of the bucket period (= start of the next bucket).
    pub end: jiff::civil::Date,
    /// Net money entering the scope in this period, folded per transaction
    /// (see `FlowTotals`).
    pub inflow: Amount,
    /// Net money leaving the scope in this period, as a magnitude.
    pub outflow: Amount,
}

/// Windowed account statistics for the dashboard: in-window flows plus the
/// opening/closing running balances that bracket the window.
///
/// All [`Amount`]s carry the queried commodity. `inflow`, `outflow`,
/// `internal` and `tx_count` cover `[from, until)`, with flows folded per
/// transaction by [`FlowTotals`]; `opening` is the running balance
/// immediately before the window; `closing` is the running balance at the
/// window end.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct PeriodStats {
    /// In-window net inflow (non-negative).
    pub inflow: Amount,
    /// In-window net outflow magnitude (non-negative).
    pub outflow: Amount,
    /// In-window movement between legs of the scope (non-negative).
    pub internal: Amount,
    /// `inflow − outflow` (signed).
    pub net: Amount,
    /// Running balance immediately before the window (`[genesis, from)`).
    pub opening: Amount,
    /// Running balance at the window end (`opening + net`).
    pub closing: Amount,
    /// Count of distinct in-window transactions involving the account
    /// (commodity-agnostic; matches the register's row count).
    pub tx_count: u32,
}

/// In-window flows of an account scope, folded one transaction at a time.
///
/// Per transaction, `P` is the sum of the scope's positive legs and `N` the
/// magnitude of its negative legs. The net `P − N` lands in `inflow` or
/// `outflow`, and `min(P, N)`, which moved between legs of the scope, lands
/// in `internal`. `inflow − outflow` therefore equals the sum of the legs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct FlowTotals {
    /// Net money entering the scope (non-negative).
    pub inflow: Decimal,
    /// Net money leaving the scope, as a magnitude (non-negative).
    pub outflow: Decimal,
    /// Money moved between legs of the scope (non-negative).
    pub internal: Decimal,
}

impl FlowTotals {
    /// Folds one transaction's scope legs, already restricted to one commodity.
    ///
    /// # Arguments
    ///
    /// * `legs` - The signed values of the transaction's legs on the scope.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a running total overflows [`Decimal`].
    #[inline]
    pub fn add_transaction(&mut self, legs: impl IntoIterator<Item = Decimal>) -> BcResult<()> {
        let overflow = || BcError::BadData("flow overflow: sum exceeds Decimal range".into());
        let mut positive = Decimal::ZERO;
        let mut negative = Decimal::ZERO;
        for leg in legs {
            if leg >= Decimal::ZERO {
                positive = positive.checked_add(leg).ok_or_else(overflow)?;
            } else {
                negative = negative.checked_sub(leg).ok_or_else(overflow)?;
            }
        }
        let net = positive.checked_sub(negative).ok_or_else(overflow)?;
        if net >= Decimal::ZERO {
            self.inflow = self.inflow.checked_add(net).ok_or_else(overflow)?;
        } else {
            self.outflow = self.outflow.checked_sub(net).ok_or_else(overflow)?;
        }
        self.internal = self
            .internal
            .checked_add(positive.min(negative))
            .ok_or_else(overflow)?;
        Ok(())
    }
}

/// Per-transaction running balances of an account scope, in every commodity
/// the scope holds.
///
/// Chronological order is the reverse of the register's display order
/// `(date DESC, id ASC)`, i.e. `(date ASC, id DESC)`.
#[derive(Clone, Debug, Default)]
pub struct ScopeLedger {
    /// Scope balance after each transaction, keyed by transaction id.
    after: HashMap<String, Balances>,
    /// Net movement of the scope within each transaction, keyed by transaction id.
    deltas: HashMap<String, Balances>,
    /// Commodities touched on the scope by each transaction, including ones
    /// whose net movement is zero (an internal transfer still has a commodity).
    commodities: HashMap<String, BTreeSet<String>>,
}

impl ScopeLedger {
    /// Scope balance in `commodity` immediately after `tx_id`; zero for an
    /// unknown transaction.
    #[must_use]
    pub fn balance_after(&self, tx_id: &str, commodity: &str) -> Decimal {
        self.after
            .get(tx_id)
            .and_then(|b| b.get(commodity))
            .unwrap_or(Decimal::ZERO)
    }

    /// Net movement of the scope in `commodity` within `tx_id`; zero for an
    /// unknown transaction.
    #[must_use]
    pub fn delta(&self, tx_id: &str, commodity: &str) -> Decimal {
        self.deltas
            .get(tx_id)
            .and_then(|b| b.get(commodity))
            .unwrap_or(Decimal::ZERO)
    }

    /// The single commodity `tx_id` moved on the scope, or `None` when it moved
    /// none or several.
    #[must_use]
    pub fn focal_commodity(&self, tx_id: &str) -> Option<&str> {
        let set = self.commodities.get(tx_id)?;
        (set.len() == 1)
            .then(|| set.iter().next().map(String::as_str))
            .flatten()
    }
}

/// One account's row from the accounts table.
#[derive(Debug)]
struct AccountRow {
    /// Parsed id.
    id: AccountId,
    /// Parent account id, if any.
    parent: Option<String>,
    /// Not archived.
    active: bool,
    /// An asset or liability account.
    holding: bool,
    /// Configured default commodity (`account_commodities.position = 0`).
    configured: Option<String>,
}

/// One account's own (non-rolled-up) totals.
#[derive(Debug, Default)]
struct OwnTotals {
    /// Concrete legs plus attributable residuals, per commodity.
    balances: Balances,
    /// Concrete-leg count per commodity, for the most-used tier.
    counts: Vec<(String, u64)>,
    /// First residual commodity in stream order, for the last tier.
    first_residual: Option<String>,
}

/// One account's running sums while [`Engine::account_totals`] folds the stream.
#[derive(Debug)]
struct TotalsSlot {
    /// The account.
    account: Interned,
    /// Per commodity: running sum and concrete-leg count, in first-seen order.
    sums: Vec<(Interned, Decimal, u64)>,
    /// First residual commodity in stream order.
    first_residual: Option<Interned>,
}

impl TotalsSlot {
    /// Folds one leg into the slot.
    ///
    /// # Arguments
    ///
    /// * `commodity` - The leg's commodity.
    /// * `value` - The leg's amount in `commodity`.
    /// * `source` - Where the value came from.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if the running sum overflows.
    fn add(&mut self, commodity: &Interned, value: Decimal, source: LegSource) -> BcResult<()> {
        let index = self
            .sums
            .iter()
            .position(|(c, ..)| c.idx() == commodity.idx())
            .unwrap_or_else(|| {
                self.sums.push((commodity.clone(), Decimal::ZERO, 0));
                self.sums.len().saturating_sub(1)
            });
        let entry = self
            .sums
            .get_mut(index)
            .ok_or_else(|| BcError::BadData("totals slot index out of range".into()))?;
        entry.1 = entry.1.checked_add(value).ok_or_else(|| {
            BcError::BadData("balance overflow: sum exceeds Decimal range".into())
        })?;
        match source {
            LegSource::Concrete => entry.2 = entry.2.saturating_add(1),
            LegSource::Residual => {
                if self.first_residual.is_none() {
                    self.first_residual = Some(commodity.clone());
                }
            }
            LegSource::Ambiguous => {}
        }
        Ok(())
    }

    /// Converts the running sums into the account's id and [`OwnTotals`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a balance overflows.
    fn finish(self) -> BcResult<(String, OwnTotals)> {
        let mut totals = OwnTotals {
            first_residual: self.first_residual.map(|c| c.as_str().to_owned()),
            ..OwnTotals::default()
        };
        for (code, sum, count) in self.sums {
            totals
                .balances
                .try_add(&Amount::new(sum, code.as_str()))
                .map_err(|e| BcError::BadData(format!("balance overflow: {e}")))?;
            if count > 0 {
                totals.counts.push((code.as_str().to_owned(), count));
            }
        }
        Ok((self.account.as_str().to_owned(), totals))
    }
}

/// `(id, parent_id, active, account_type, configured commodity)` as
/// [`Engine::account_totals`] reads it.
type AccountQueryRow = (String, Option<String>, bool, String, Option<String>);

/// Every account's own totals and default commodity from one ledger pass.
///
/// Built by [`Engine::account_totals`]. Feeds the default-commodity balance
/// map, the subtree roll-up and the net-worth holdings.
#[derive(Debug)]
pub struct AccountTotals {
    /// Every account, keyed by id string.
    accounts: HashMap<String, AccountRow>,
    /// Own totals for accounts that hold at least one leg, keyed by id string.
    own: HashMap<String, OwnTotals>,
}

impl AccountTotals {
    /// The default commodity for `id`: configured, else most-used concrete
    /// commodity (ties to the lowest code), else the first residual commodity.
    fn default_commodity(&self, id: &str, row: &AccountRow) -> Option<String> {
        row.configured.clone().or_else(|| {
            let own = self.own.get(id)?;
            own.counts
                .iter()
                .max_by(|(ca, na), (cb, nb)| na.cmp(nb).then_with(|| cb.cmp(ca)))
                .map(|(code, _)| code.clone())
                .or_else(|| own.first_residual.clone())
        })
    }

    /// Each active account's balance in its default commodity.
    ///
    /// An account with a default commodity and no legs reads zero. An account
    /// with neither is omitted.
    ///
    /// # Returns
    ///
    /// One `(account, balance)` pair per active account with a default
    /// commodity, in no particular order.
    #[inline]
    pub fn defaults(&self) -> impl Iterator<Item = (AccountId, Amount)> + '_ {
        self.accounts
            .iter()
            .filter(|(_, row)| row.active)
            .filter_map(|(id, row)| {
                let code = self.default_commodity(id, row)?;
                let value = self
                    .own
                    .get(id)
                    .and_then(|own| own.balances.get(&code))
                    .unwrap_or(Decimal::ZERO);
                Some((row.id.clone(), Amount::new(value, code)))
            })
    }

    /// Active asset and liability accounts' own totals, for net worth.
    ///
    /// Accounts whose totals are all zero are omitted.
    pub(crate) fn holdings(&self) -> impl Iterator<Item = (&str, &Balances)> {
        self.own.iter().filter_map(|(id, own)| {
            let row = self.accounts.get(id)?;
            (row.active && row.holding && !own.balances.is_empty())
                .then_some((id.as_str(), &own.balances))
        })
    }

    /// Every account's own totals, active or not, including ids missing from
    /// the accounts table.
    #[cfg(test)]
    pub(crate) fn holdings_all_for_test(&self) -> impl Iterator<Item = (&str, &Balances)> {
        self.own
            .iter()
            .map(|(id, own)| (id.as_str(), &own.balances))
    }

    /// Per-commodity totals over every active account and its active descendants.
    ///
    /// Each active account's own totals are added into itself and every
    /// ancestor. Accounts whose whole subtree holds no legs are absent. No
    /// commodity conversion takes place.
    ///
    /// # Returns
    ///
    /// One `(account, balances)` pair per active account with a leg in its
    /// subtree, in no particular order.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a running total overflows.
    #[inline]
    pub fn rollups(&self) -> BcResult<impl Iterator<Item = (AccountId, Balances)>> {
        // Only active accounts contribute their own totals; a leg on an id the
        // accounts map lacks is skipped. A `BTreeMap` visits accounts in a
        // fixed order, so each parent's roll-up folds its children's
        // commodities in a stable order.
        let own: BTreeMap<&str, &Balances> = self
            .own
            .iter()
            .filter(|(id, _)| self.accounts.get(id.as_str()).is_some_and(|row| row.active))
            .map(|(id, own)| (id.as_str(), &own.balances))
            .collect();

        // Fold each account's own totals into itself and every ancestor.
        // Parent links cover *every* account, active or archived: an archived
        // account can still sit between two active ones in the tree (e.g.
        // Assets -> Bank (archived) -> Savings), and the walk must pass
        // through it to reach Assets.
        let mut rolled: HashMap<&str, Balances> = HashMap::new();
        for (acc_id, balances) in own {
            let mut cursor = Some(acc_id);
            // Guards against a corrupt `parent_id` cycle, which would
            // otherwise loop forever; a real tree never revisits an id.
            let mut seen: HashSet<&str> = HashSet::new();
            while let Some(id) = cursor {
                if !seen.insert(id) {
                    break;
                }
                let row = self.accounts.get(id);
                // An archived account in the middle of the chain is skipped
                // as a *result* entry, but the walk still passes through it
                // to reach any active ancestor above it.
                if row.is_some_and(|account| account.active) {
                    let entry = rolled.entry(id).or_default();
                    for (code, value) in balances.iter() {
                        entry
                            .try_add(&Amount::new(value, code))
                            .map_err(|e| BcError::BadData(format!("rollup overflow: {e}")))?;
                    }
                }
                cursor = row.and_then(|account| account.parent.as_deref());
            }
        }

        let out: Vec<(AccountId, Balances)> = rolled
            .into_iter()
            .filter_map(|(id, balances)| {
                self.accounts.get(id).map(|row| (row.id.clone(), balances))
            })
            .collect();
        Ok(out.into_iter())
    }
}

/// How a [`NetWorthRow`] entered its report's total.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Valuation {
    /// The holding is already in the report commodity and was summed as-is.
    Native,
    /// The holding was converted at a rate; the converted amount is what the
    /// total contains.
    Converted(Amount),
    /// No rate was available. The holding is listed but absent from the total.
    Unvalued,
}

/// One holding in a net-worth report: an account's balance in one commodity.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetWorthRow {
    /// The account holding the balance.
    pub account_id: AccountId,
    /// The account's display name.
    pub name: String,
    /// The account's kind, so a caller can label a manual asset differently.
    pub kind: bc_models::AccountKind,
    /// The balance in its own commodity.
    pub balance: Amount,
    /// Whether, and how, `balance` reached the total.
    pub valuation: Valuation,
}

/// The outcome of [`Engine::net_worth`].
///
/// `total` only counts holdings that were native to the report commodity or
/// that a rate could convert. Everything else is in `unvalued`, so a caller
/// can tell an honest zero from a holding it could not price.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct NetWorth {
    /// Sum of every valued holding, in the report commodity.
    pub total: Amount,
    /// Every holding, ordered by account name then commodity. An account with
    /// no holdings at all gets one zero row in the report commodity.
    pub rows: Vec<NetWorthRow>,
    /// Native amounts a rate turned into part of `total`, summed by commodity.
    pub converted: bc_models::Balances,
    /// Native amounts no rate could value, summed by commodity. Not in `total`.
    pub unvalued: bc_models::Balances,
}

/// Calculates account balances from the `postings` projection table.
#[derive(Debug, Clone)]
pub struct Engine {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

/// Values of `tx`'s legs in `commodity`.
fn values_in<'a>(
    tx: &'a ResolvedTransaction,
    commodity: &'a str,
) -> impl Iterator<Item = Decimal> + 'a {
    tx.legs()
        .iter()
        .filter(move |leg| leg.commodity().is_some_and(|c| c.as_str() == commodity))
        .map(ResolvedLeg::value)
}

/// Serialises account ids as the JSON array `json_each`-driven queries expect.
///
/// Shared by every query that scopes a set of accounts through `json_each`,
/// in this module and in [`crate::transaction`].
///
/// # Errors
///
/// Returns [`BcError::BadData`] if serialisation fails.
pub(crate) fn ids_json(ids: &[AccountId]) -> BcResult<String> {
    let strings: Vec<String> = ids.iter().map(ToString::to_string).collect();
    serde_json::to_string(&strings)
        .map_err(|e| BcError::BadData(format!("account id list serialisation: {e}")))
}

/// Builds `count` contiguous period buckets ending with the period containing
/// `as_of`, oldest-first. Each bucket is a half-open `[start, end)` range one
/// `period` wide.
///
/// Every bucket is snapped to the period's own calendar boundary, so the first
/// bucket generally starts *before* `as_of - count * period` and the last one
/// ends *after* `as_of`.
///
/// The WASM side mirrors these snapping rules: `bc-ui` re-implements them in its
/// `coverage_count` helper because `bc-core` is native-only and absent from the
/// WASM bundle. Any change to the snapping here must be mirrored there, or the
/// sparkline's coverage estimate silently drifts from the buckets it labels.
///
/// # Arguments
///
/// * `period` - Bucket width.
/// * `count`  - Number of buckets.
/// * `as_of`  - Reference date; the newest bucket contains it.
///
/// # Returns
///
/// A `Vec` of `(start, end)` ranges, oldest-first, of length `count`.
pub(crate) fn bucket_ranges(
    period: &bc_models::Period,
    count: core::num::NonZeroUsize,
    as_of: jiff::civil::Date,
) -> Vec<(jiff::civil::Date, jiff::civil::Date)> {
    let mut ranges: Vec<(jiff::civil::Date, jiff::civil::Date)> = Vec::with_capacity(count.get());
    let current = period.range_containing(as_of);
    ranges.push(current);
    let mut prev_start = current.0;
    for _ in 1..count.get() {
        let prev =
            period.range_containing(prev_start.saturating_sub(jiff::Span::new().days(1_i32)));
        ranges.push(prev);
        prev_start = prev.0;
    }
    ranges.reverse(); // oldest first
    ranges
}

impl NetWorthRow {
    /// Creates a row; see the field docs for what each part means.
    #[must_use]
    #[inline]
    pub fn new(
        account_id: AccountId,
        name: String,
        kind: bc_models::AccountKind,
        balance: Amount,
        valuation: Valuation,
    ) -> Self {
        Self {
            account_id,
            name,
            kind,
            balance,
            valuation,
        }
    }
}

impl NetWorth {
    /// Creates a report; see the field docs for what each part means.
    #[must_use]
    #[inline]
    pub fn new(
        total: Amount,
        rows: Vec<NetWorthRow>,
        converted: bc_models::Balances,
        unvalued: bc_models::Balances,
    ) -> Self {
        Self {
            total,
            rows,
            converted,
            unvalued,
        }
    }

    /// Adds `value` (already in the report commodity) to `total`.
    fn add_to_total(&mut self, value: Decimal) -> BcResult<()> {
        let sum = self
            .total
            .value()
            .checked_add(value)
            .ok_or_else(|| BcError::BadData("net worth overflow".into()))?;
        self.total = Amount::new(sum, self.total.commodity().clone());
        Ok(())
    }
}

/// Adds `value` of `code` into `balances`, mapping overflow to [`BcError::BadData`].
fn add_holding(balances: &mut bc_models::Balances, value: Decimal, code: &str) -> BcResult<()> {
    balances
        .try_add(&Amount::new(value, code))
        .map_err(|e| BcError::BadData(format!("net worth overflow for '{code}': {e}")))
}

impl Engine {
    /// Creates a [`Engine`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Returns the running balance for `account_id` in `commodity`.
    ///
    /// Returns an [`Amount`] carrying `commodity`, zero-valued if no postings exist.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if a stored amount cannot be parsed.
    #[inline]
    pub async fn balance_for(&self, account_id: &AccountId, commodity: &str) -> BcResult<Amount> {
        let ids = core::slice::from_ref(account_id);
        let mut stream = pin!(resolved_transactions(
            &self.pool,
            &LegFilter::new(LegScope::Accounts(ids))
        )?);
        let mut total = Decimal::ZERO;
        while let Some(tx) = stream.try_next().await? {
            for value in values_in(&tx, commodity) {
                total = total.checked_add(value).ok_or_else(|| {
                    BcError::BadData("balance overflow: sum exceeds Decimal range".into())
                })?;
            }
        }
        Ok(Amount::new(total, commodity))
    }

    /// Builds the per-transaction running balances of the accounts in `ids`.
    ///
    /// One pass over the scope's postings: concrete legs plus the residual of
    /// each elided leg. Cost is linear in the scope's posting count.
    ///
    /// # Arguments
    ///
    /// * `ids` - The accounts folded together (one account, or a subtree).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure, an unparsable stored value, or
    /// a running total overflowing [`Decimal`].
    pub async fn scope_ledger(&self, ids: &[AccountId]) -> BcResult<ScopeLedger> {
        // (date, tx_id, commodity, delta) for every leg, before ordering.
        let mut legs: Vec<(jiff::civil::Date, String, String, Decimal)> = Vec::new();
        {
            let mut stream = pin!(resolved_transactions(
                &self.pool,
                &LegFilter::new(LegScope::Accounts(ids))
            )?);
            while let Some(tx) = stream.try_next().await? {
                for leg in tx.legs() {
                    let Some(commodity) = leg.commodity() else {
                        continue; // ambiguous: no commodity, zero value
                    };
                    legs.push((
                        tx.date(),
                        tx.id().to_owned(),
                        commodity.as_str().to_owned(),
                        leg.value(),
                    ));
                }
            }
        }

        // Chronological: date ascending, then id descending (reverse of the
        // register's display order).
        legs.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(&a.1)));

        let mut ledger = ScopeLedger::default();
        let mut running = Balances::new();
        for (_, tx_id, commodity, value) in legs {
            let amount = Amount::new(value, CommodityCode::new(&commodity));
            running
                .try_add(&amount)
                .map_err(|e| BcError::BadData(format!("running balance overflow: {e}")))?;
            let delta = ledger.deltas.entry(tx_id.clone()).or_default();
            delta
                .try_add(&amount)
                .map_err(|e| BcError::BadData(format!("delta overflow: {e}")))?;
            ledger
                .commodities
                .entry(tx_id.clone())
                .or_default()
                .insert(commodity);
            // Overwrite on every leg so the last leg of a transaction leaves
            // the balance after the whole transaction.
            ledger.after.insert(tx_id, running.clone());
        }
        Ok(ledger)
    }

    /// Computes net worth in `commodity` across all active asset and liability accounts.
    ///
    /// - [`DepositAccount`], [`Receivable`], [`VirtualAllocation`], [`Group`]: balance from
    ///   postings, in every commodity the account holds. A [`Group`] is an organisational
    ///   node whose postings belong on its descendants, so its own balance is normally zero.
    /// - [`ManualAsset`]: latest recorded market value from `asset_valuations`.
    /// - Accounts with `AccountType` other than `Asset`/`Liability` are excluded.
    ///
    /// A holding in another commodity is converted through `fx` when a rate exists and
    /// listed in [`NetWorth::unvalued`] when none does. A missing rate is never an error:
    /// the total stays honest by omission, and the caller can see what was omitted.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure, or if a sum overflows.
    ///
    /// [`DepositAccount`]: bc_models::AccountKind::DepositAccount
    /// [`Receivable`]: bc_models::AccountKind::Receivable
    /// [`VirtualAllocation`]: bc_models::AccountKind::VirtualAllocation
    /// [`ManualAsset`]: bc_models::AccountKind::ManualAsset
    /// [`Group`]: bc_models::AccountKind::Group
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "intentional fallback with warning for future AccountKind variants"
    )]
    #[inline]
    pub async fn net_worth(
        &self,
        commodity: &str,
        fx: &dyn crate::fx::FxRateService,
    ) -> BcResult<NetWorth> {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        let accounts = crate::account::Service::new(self.pool.clone())
            .list_active()
            .await?;
        let asset_svc = crate::asset::Service::new(self.pool.clone());
        let mut holdings = self.holdings_by_account().await?;

        let target = bc_models::CommodityCode::new(commodity);
        let mut report = NetWorth::new(
            Amount::new(Decimal::ZERO, commodity),
            Vec::new(),
            bc_models::Balances::new(),
            bc_models::Balances::new(),
        );

        // `list_active` orders by name, so rows come out by name then commodity.
        for account in &accounts {
            match account.account_type() {
                AccountType::Asset | AccountType::Liability => {}
                _ => continue,
            }

            let balances = match account.kind() {
                AccountKind::ManualAsset => {
                    // The newest recorded market value, in whichever commodity it
                    // was recorded, stands in for a posting-based balance.
                    let mut balances = bc_models::Balances::new();
                    if let Some(valuation) = asset_svc.latest_valuation(account.id()).await? {
                        add_holding(
                            &mut balances,
                            valuation.value(),
                            valuation.commodity().as_str(),
                        )?;
                    }
                    balances
                }
                AccountKind::DepositAccount
                | AccountKind::Receivable
                | AccountKind::VirtualAllocation
                | AccountKind::Group => holdings
                    .remove(&account.id().to_string())
                    .unwrap_or_default(),
                _ => {
                    tracing::warn!(
                        account_id = %account.id(),
                        kind = ?account.kind(),
                        "unknown AccountKind in net_worth; using posting-based balance"
                    );
                    holdings
                        .remove(&account.id().to_string())
                        .unwrap_or_default()
                }
            };

            // `Balances` never holds a zero, so an account with nothing in it
            // would otherwise vanish from the rows; give it one zero row so a
            // caller can tell "empty" from "not an asset or liability".
            let mut sorted: Vec<(&str, Decimal)> = balances.iter().collect();
            if sorted.is_empty() {
                sorted.push((commodity, Decimal::ZERO));
            }
            sorted.sort_unstable_by(|a, b| a.0.cmp(b.0));
            for (code, value) in sorted {
                let balance = Amount::new(value, code);
                let valuation = if *balance.commodity() == target {
                    report.add_to_total(value)?;
                    Valuation::Native
                } else {
                    match fx.convert(&balance, &target) {
                        Ok(converted) => {
                            report.add_to_total(converted.value())?;
                            add_holding(&mut report.converted, value, code)?;
                            Valuation::Converted(converted)
                        }
                        Err(e) => {
                            // The report carries this in `unvalued`; the log
                            // line only adds which account it came from.
                            tracing::debug!(
                                account_id = %account.id(),
                                %e,
                                "net_worth: holding left out of the total"
                            );
                            add_holding(&mut report.unvalued, value, code)?;
                            Valuation::Unvalued
                        }
                    }
                };
                report.rows.push(NetWorthRow::new(
                    account.id().clone(),
                    account.name().to_owned(),
                    account.kind(),
                    balance,
                    valuation,
                ));
            }
        }

        Ok(report)
    }

    /// Each active asset or liability account's own totals, concrete legs and
    /// elided-leg residuals together, keyed by account id string.
    ///
    /// Accounts with nothing but zero balances are absent from the map.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure, or if a sum overflows.
    async fn holdings_by_account(&self) -> BcResult<HashMap<String, Balances>> {
        let totals = self.account_totals().await?;
        Ok(totals
            .holdings()
            .map(|(id, balances)| (id.to_owned(), balances.clone()))
            .collect())
    }

    /// Returns the total inflow and outflow for the accounts in `ids` in `commodity`
    /// over `[from, to)`.
    ///
    /// - `inflow` — sum of all positive postings (money entering the accounts).
    /// - `outflow` — absolute sum of all negative postings (money leaving).
    ///
    /// Both values are non-negative. A transfer between two accounts of `ids`
    /// contributes both an inflow and an outflow.
    ///
    /// # Arguments
    ///
    /// * `ids` - The accounts to fold together.
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `from`       - Inclusive start date.
    /// * `to`         - Exclusive end date.
    ///
    /// # Returns
    ///
    /// `(inflow, outflow)` as [`Amount`] values.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn posting_flows_for_set(
        &self,
        ids: &[AccountId],
        commodity: &str,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
    ) -> BcResult<(Amount, Amount)> {
        let filter = LegFilter::new(LegScope::Accounts(ids)).window(from, to);
        let mut stream = pin!(resolved_transactions(&self.pool, &filter)?);
        let mut inflow = Decimal::ZERO;
        let mut outflow = Decimal::ZERO;
        while let Some(tx) = stream.try_next().await? {
            for value in values_in(&tx, commodity) {
                if value >= Decimal::ZERO {
                    inflow = inflow.checked_add(value).ok_or_else(|| {
                        BcError::BadData("inflow overflow: sum exceeds Decimal range".into())
                    })?;
                } else {
                    outflow = outflow.checked_sub(value).ok_or_else(|| {
                        BcError::BadData("outflow overflow: sum exceeds Decimal range".into())
                    })?;
                }
            }
        }
        Ok((
            Amount::new(inflow, commodity),
            Amount::new(outflow, commodity),
        ))
    }

    /// Returns the total inflow and outflow for `account_id` in `commodity` over `[from, to)`.
    ///
    /// - `inflow` — sum of all positive postings (money entering the account).
    /// - `outflow` — absolute sum of all negative postings (money leaving).
    ///
    /// Both values are non-negative.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `from`       - Inclusive start date.
    /// * `to`         - Exclusive end date.
    ///
    /// # Returns
    ///
    /// `(inflow, outflow)` as [`Amount`] values.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn posting_flows(
        &self,
        account_id: &AccountId,
        commodity: &str,
        from: jiff::civil::Date,
        to: jiff::civil::Date,
    ) -> BcResult<(Amount, Amount)> {
        self.posting_flows_for_set(core::slice::from_ref(account_id), commodity, from, to)
            .await
    }

    /// Computes `PeriodStats` over the union of `ids` in `commodity` for `[from, until)`.
    ///
    /// Flows are folded per transaction by `FlowTotals`, so a transfer
    /// between two accounts of the set lands in `internal`; `tx_count`
    /// counts each transaction once.
    ///
    /// # Arguments
    ///
    /// * `ids` - The accounts to fold together (typically a subtree).
    /// * `commodity` - Commodity code (e.g. `"AUD"`).
    /// * `from` - Inclusive window start.
    /// * `until` - Exclusive window end.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure, or if a running
    /// balance would overflow [`Decimal`]'s range.
    #[inline]
    pub async fn account_period_stats_for_set(
        &self,
        ids: &[AccountId],
        commodity: &str,
        from: jiff::civil::Date,
        until: jiff::civil::Date,
    ) -> BcResult<PeriodStats> {
        let filter = LegFilter::new(LegScope::Accounts(ids)).window(jiff::civil::Date::MIN, until);
        let mut stream = pin!(resolved_transactions(&self.pool, &filter)?);
        let mut opening_flows = FlowTotals::default();
        let mut flows = FlowTotals::default();
        let mut tx_count: u32 = 0;
        while let Some(tx) = stream.try_next().await? {
            if tx.date() < from {
                opening_flows.add_transaction(values_in(&tx, commodity))?;
            } else {
                flows.add_transaction(values_in(&tx, commodity))?;
                tx_count = tx_count.saturating_add(1);
            }
        }
        let opening = opening_flows
            .inflow
            .checked_sub(opening_flows.outflow)
            .ok_or_else(|| BcError::BadData("opening balance overflow".into()))?;
        let net = flows
            .inflow
            .checked_sub(flows.outflow)
            .ok_or_else(|| BcError::BadData("net overflow".into()))?;
        let closing = opening
            .checked_add(net)
            .ok_or_else(|| BcError::BadData("closing balance overflow".into()))?;

        Ok(PeriodStats {
            inflow: Amount::new(flows.inflow, commodity),
            outflow: Amount::new(flows.outflow, commodity),
            internal: Amount::new(flows.internal, commodity),
            net: Amount::new(net, commodity),
            opening: Amount::new(opening, commodity),
            closing: Amount::new(closing, commodity),
            tx_count,
        })
    }

    /// Computes `PeriodStats` for `account_id` in `commodity` over `[from, until)`.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity` - Commodity code (e.g. `"AUD"`).
    /// * `from` - Inclusive window start.
    /// * `until` - Exclusive window end.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure, or if a running
    /// balance would overflow [`Decimal`]'s range.
    #[inline]
    pub async fn account_period_stats(
        &self,
        account_id: &AccountId,
        commodity: &str,
        from: jiff::civil::Date,
        until: jiff::civil::Date,
    ) -> BcResult<PeriodStats> {
        self.account_period_stats_for_set(core::slice::from_ref(account_id), commodity, from, until)
            .await
    }

    /// Returns `count` contiguous period buckets ending with the period containing `as_of`,
    /// over the union of `ids`.
    ///
    /// Buckets are returned oldest-first. Each bucket covers exactly one `period`
    /// length. Postings are fetched in a single query and assigned to buckets in Rust.
    /// Flows are folded per transaction by `FlowTotals`; a transaction falls in
    /// exactly one bucket by its date.
    ///
    /// # Arguments
    ///
    /// * `ids` - The accounts to fold together (typically a subtree).
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `period`     - Bucket width. Use [`bc_models::Period::Monthly`] for a 6-month sparkline.
    /// * `count`      - Number of buckets to return.
    /// * `as_of`      - Reference date; the most recent bucket contains this date.
    ///
    /// # Returns
    ///
    /// [`Vec<PostingBucket>`] ordered oldest-first. Length equals `count`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn posting_buckets_for_set(
        &self,
        ids: &[AccountId],
        commodity: &str,
        period: &bc_models::Period,
        count: core::num::NonZeroUsize,
        as_of: jiff::civil::Date,
    ) -> BcResult<Vec<PostingBucket>> {
        let ranges = bucket_ranges(period, count, as_of);

        // ranges is non-empty: count is NonZeroUsize so we always push at least one entry.
        let Some(&(earliest_start, _)) = ranges.first() else {
            return Ok(vec![]);
        };
        let Some(&(_, latest_end)) = ranges.last() else {
            return Ok(vec![]);
        };

        // Distribute transactions into per-range flow accumulators.
        let mut acc: Vec<(jiff::civil::Date, jiff::civil::Date, FlowTotals)> = ranges
            .into_iter()
            .map(|(start, end)| (start, end, FlowTotals::default()))
            .collect();

        let filter = LegFilter::new(LegScope::Accounts(ids)).window(earliest_start, latest_end);
        let mut stream = pin!(resolved_transactions(&self.pool, &filter)?);
        while let Some(tx) = stream.try_next().await? {
            let date = tx.date();
            if let Some(slot) = acc
                .iter_mut()
                .find(|(start, end, _)| date >= *start && date < *end)
            {
                slot.2.add_transaction(values_in(&tx, commodity))?;
            }
        }

        Ok(acc
            .into_iter()
            .map(|(start, end, flows)| PostingBucket {
                start,
                end,
                inflow: Amount::new(flows.inflow, commodity),
                outflow: Amount::new(flows.outflow, commodity),
            })
            .collect())
    }

    /// Returns `count` contiguous period buckets ending with the period containing `as_of`.
    ///
    /// Buckets are returned oldest-first. Each bucket covers exactly one `period`
    /// length. Postings are fetched in a single query and assigned to buckets in Rust.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account to query.
    /// * `commodity`  - Commodity code (e.g. `"AUD"`).
    /// * `period`     - Bucket width. Use [`bc_models::Period::Monthly`] for a 6-month sparkline.
    /// * `count`      - Number of buckets to return.
    /// * `as_of`      - Reference date; the most recent bucket contains this date.
    ///
    /// # Returns
    ///
    /// [`Vec<PostingBucket>`] ordered oldest-first. Length equals `count`.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn posting_buckets(
        &self,
        account_id: &AccountId,
        commodity: &str,
        period: &bc_models::Period,
        count: core::num::NonZeroUsize,
        as_of: jiff::civil::Date,
    ) -> BcResult<Vec<PostingBucket>> {
        self.posting_buckets_for_set(
            core::slice::from_ref(account_id),
            commodity,
            period,
            count,
            as_of,
        )
        .await
    }

    /// Returns the commodity code of the first (default) commodity for `account_id`, or `None`.
    ///
    /// Prefers the configured default from `account_commodities` (position = 0). When no
    /// commodity is configured, falls back to the most-used posting commodity so that
    /// accounts imported without explicit commodity setup still return a useful value. When
    /// every posting on the account is elided (so no stored commodity exists at all), falls
    /// back further to the first commodity of the account's residual in transaction order.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure.
    #[inline]
    pub async fn default_commodity_for(&self, account_id: &AccountId) -> BcResult<Option<String>> {
        let (result,): (Option<String>,) = sqlx::query_as(
            "SELECT COALESCE(
                 (SELECT c.code
                  FROM account_commodities ac
                  JOIN commodities c ON c.id = ac.commodity_id
                  WHERE ac.account_id = ?
                  ORDER BY ac.position
                  LIMIT 1),
                 (SELECT p.commodity
                  FROM postings p
                  WHERE p.account_id = ?
                    AND p.commodity IS NOT NULL
                  GROUP BY p.commodity
                  ORDER BY COUNT(*) DESC, p.commodity ASC
                  LIMIT 1)
             ) AS commodity_code",
        )
        .bind(account_id.to_string())
        .bind(account_id.to_string())
        .fetch_one(&self.pool)
        .await?;

        if let Some(code) = result {
            return Ok(Some(code));
        }

        // Every posting on this account may be elided, in which case no stored
        // commodity exists anywhere. Take the first residual commodity instead.
        let ids = core::slice::from_ref(account_id);
        let mut stream = pin!(resolved_transactions(
            &self.pool,
            &LegFilter::new(LegScope::Accounts(ids))
        )?);
        while let Some(tx) = stream.try_next().await? {
            if let Some(commodity) = tx
                .legs()
                .iter()
                .find(|leg| leg.source() == LegSource::Residual)
                .and_then(ResolvedLeg::commodity)
            {
                return Ok(Some(commodity.as_str().to_owned()));
            }
        }
        Ok(None)
    }

    /// Computes every account's own totals in one pass over the ledger.
    ///
    /// The accounts query and the leg stream share one read transaction, so
    /// both see the same snapshot.
    ///
    /// # Returns
    ///
    /// The totals, from which [`AccountTotals::defaults`],
    /// [`AccountTotals::rollups`] and the net-worth holdings derive.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure, unparsable stored data, or an
    /// overflowing total.
    #[inline]
    pub async fn account_totals(&self) -> BcResult<AccountTotals> {
        let mut tx = self.pool.begin().await?;
        let rows: Vec<AccountQueryRow> = sqlx::query_as(
            "SELECT a.id, a.parent_id, a.archived_at IS NULL, a.account_type, c.code
             FROM accounts a
             LEFT JOIN account_commodities ac ON ac.account_id = a.id AND ac.position = 0
             LEFT JOIN commodities c ON c.id = ac.commodity_id",
        )
        .fetch_all(&mut *tx)
        .await?;
        let asset = crate::db::to_db_str(bc_models::AccountType::Asset)?;
        let liability = crate::db::to_db_str(bc_models::AccountType::Liability)?;
        let mut accounts = HashMap::with_capacity(rows.len());
        for (id, parent, active, account_type, configured) in rows {
            let parsed = id
                .parse::<AccountId>()
                .map_err(|e| BcError::BadData(format!("invalid account id '{id}': {e}")))?;
            let holding = account_type == asset || account_type == liability;
            accounts.insert(
                id,
                AccountRow {
                    id: parsed,
                    parent,
                    active,
                    holding,
                    configured,
                },
            );
        }

        // Dense accumulators indexed by the stream's interned account index.
        let mut slots: Vec<Option<TotalsSlot>> = Vec::new();
        {
            let mut stream = pin!(resolved_transactions(
                &mut *tx,
                &LegFilter::new(LegScope::Ledger)
            )?);
            while let Some(resolved) = stream.try_next().await? {
                for leg in resolved.legs() {
                    let Some(commodity) = leg.commodity() else {
                        continue; // ambiguous: no commodity, zero value
                    };
                    let idx = usize::try_from(leg.account().idx())
                        .map_err(|e| BcError::BadData(format!("account index overflow: {e}")))?;
                    if slots.len() <= idx {
                        slots.resize_with(idx.saturating_add(1), || None);
                    }
                    let slot = slots
                        .get_mut(idx)
                        .ok_or_else(|| BcError::BadData("account slot out of range".into()))?
                        .get_or_insert_with(|| TotalsSlot {
                            account: leg.account().clone(),
                            sums: Vec::new(),
                            first_residual: None,
                        });
                    slot.add(commodity, leg.value(), leg.source())?;
                }
            }
        }
        // Nothing was written, so the snapshot is released rather than committed.
        tx.rollback().await?;

        let own = slots
            .into_iter()
            .flatten()
            .map(TotalsSlot::finish)
            .collect::<BcResult<HashMap<_, _>>>()?;
        Ok(AccountTotals { accounts, own })
    }

    /// Returns the default-commodity balance for every active account.
    ///
    /// Balances are computed live from all postings, not from the `balances`
    /// cache table (which is a write-through cache not yet populated by the application).
    ///
    /// The map key is [`AccountId`]; the value is an [`Amount`] (carrying the default commodity).
    /// Accounts with neither a configured commodity nor any postings are omitted.
    /// Accounts with a commodity (configured or inferred) but no postings are included with a zero
    /// balance.
    ///
    /// The commodity for each account is resolved in priority order:
    /// 1. The configured default from `account_commodities` (position = 0).
    /// 2. The most-used concrete posting commodity (for accounts imported without explicit
    ///    commodity setup); ties break to the lowest commodity code.
    /// 3. The first residual commodity in transaction order (for an account whose postings
    ///    are all elided and therefore carry no stored commodity at all).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure.
    #[inline]
    pub async fn default_balances(&self) -> BcResult<HashMap<AccountId, Amount>> {
        Ok(self.account_totals().await?.defaults().collect())
    }

    /// Computes per-commodity totals over every active account and its active
    /// descendants.
    ///
    /// Own concrete postings and own elided residuals are summed per account,
    /// then each account's totals are added into every ancestor. Accounts
    /// whose whole subtree holds no postings are absent from the map. No
    /// commodity conversion takes place.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or parse failure, or if a running total
    /// overflows [`Decimal`].
    #[inline]
    pub async fn rollup_balances(&self) -> BcResult<HashMap<AccountId, Balances>> {
        Ok(self.account_totals().await?.rollups()?.collect())
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::num::NonZeroUsize;

    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::account::Cascade;

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_reflects_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet account should succeed");
        let acc_b = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income account should succeed");

        // Insert a transaction directly for simplicity
        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_1', '2026-01-01', 'Test', 'reconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction should succeed");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p1', 'tx_1', ?, '100.00', 'AUD', 0)")
            .bind(acc_a.to_string()).execute(&pool).await.expect("insert posting p1 should succeed");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p2', 'tx_1', ?, '-100.00', 'AUD', 1)")
            .bind(acc_b.to_string()).execute(&pool).await.expect("insert posting p2 should succeed");

        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc_a, "AUD")
            .await
            .expect("balance query should succeed");
        assert_eq!(balance.value(), dec!(100.00));
        assert_eq!(balance.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_zero_for_account_with_no_postings(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Empty")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc, "AUD")
            .await
            .expect("balance query should succeed");
        assert_eq!(balance.value(), Decimal::ZERO);
        assert_eq!(balance.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn net_worth_includes_manual_asset_valuation(pool: sqlx::SqlitePool) {
        use bc_models::ValuationSource;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());

        // A ManualAsset with a recorded valuation.
        let house_id = acct_svc
            .create()
            .name("House")
            .account_type(AccountType::Asset)
            .kind(AccountKind::ManualAsset)
            .acquisition_date(jiff::civil::date(2020, 1, 1))
            .acquisition_cost(dec!(500_000))
            .call()
            .await
            .expect("create ManualAsset");

        // A DepositAccount with a posting-based balance.
        let savings_id = acct_svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create DepositAccount");

        // Give the savings account a balance via a direct insert.
        let income_id = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income");
        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_nw1', '2026-01-01', 'Test', 'reconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("tx insert");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_nw1', 'tx_nw1', ?, '50000.00', 'AUD', 0)")
            .bind(savings_id.to_string()).execute(&pool).await.expect("posting insert");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_nw2', 'tx_nw1', ?, '-50000.00', 'AUD', 1)")
            .bind(income_id.to_string()).execute(&pool).await.expect("posting insert 2");

        // Record a valuation for the house.
        let asset_svc = crate::asset::Service::new(pool.clone());
        asset_svc
            .record_valuation(
                &house_id,
                dec!(650_000),
                "AUD",
                ValuationSource::ProfessionalAppraisal,
                jiff::civil::date(2026, 3, 1),
                None,
            )
            .await
            .expect("record valuation");

        let engine = Engine::new(pool.clone());
        let net_worth = engine
            .net_worth("AUD", &crate::fx::NoopFxRateService)
            .await
            .expect("net worth");

        // Expected: savings (50_000) + house valuation (650_000) = 700_000
        // (Income account is excluded from net worth as it's not Asset/Liability)
        assert_eq!(net_worth.total, Amount::new(dec!(700_000), "AUD"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_returns_real_balance(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use rust_decimal_macros::dec;

        // Create commodity
        sqlx::query("INSERT INTO commodities (id, code, decimals, is_iso, symbol_after) VALUES ('com_aud', 'AUD', 2, 1, 0)")
            .execute(&pool)
            .await
            .expect("insert commodity");

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        // Wire the commodity to the account
        sqlx::query(
            "INSERT INTO account_commodities (account_id, commodity_id, position)
             VALUES (?, 'com_aud', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("link commodity");

        // Seed via a real transaction + posting
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('t1', '2026-01-01', 'Test', 'reconciled', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("insert tx");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('p1', 't1', ?, '1234.56', 'AUD', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("insert posting");

        let engine = Engine::new(pool.clone());
        let map = engine
            .default_balances()
            .await
            .expect("default_balances should succeed");

        let bal = map.get(&acc).expect("account should be in map");
        assert_eq!(bal.commodity().as_str(), "AUD");
        assert_eq!(bal.value(), dec!(1234.56));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_zero_for_account_without_balance_row(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        sqlx::query("INSERT INTO commodities (id, code, decimals, is_iso, symbol_after) VALUES ('com_aud2', 'AUD', 2, 1, 0)")
            .execute(&pool)
            .await
            .expect("insert commodity");

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Empty")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        sqlx::query(
            "INSERT INTO account_commodities (account_id, commodity_id, position)
             VALUES (?, 'com_aud2', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("link commodity");

        let engine = Engine::new(pool.clone());
        let map = engine
            .default_balances()
            .await
            .expect("default_balances should succeed");

        let bal = map.get(&acc).expect("account in map");
        assert_eq!(bal.commodity().as_str(), "AUD");
        assert_eq!(bal.value(), rust_decimal::Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_omits_account_without_commodity(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("NoCommodity")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let engine = Engine::new(pool.clone());
        let map = engine
            .default_balances()
            .await
            .expect("default_balances should succeed");

        // Account has no commodity → not included in the map
        assert!(!map.contains_key(&acc));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_splits_inflow_and_outflow(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");
        let income = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income");

        // Two cleared transactions within the range
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tf1', '2026-04-10', 'Pay', 'reconciled', '2026-04-10T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx 1");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pf1a', 'tf1', ?, '1000.00', 'AUD', 0),
                    ('pf1b', 'tf1', ?, '-1000.00', 'AUD', 1)",
        )
        .bind(wallet.to_string())
        .bind(income.to_string())
        .execute(&pool)
        .await
        .expect("postings 1");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tf2', '2026-04-20', 'Expense', 'reconciled', '2026-04-20T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx 2");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pf2a', 'tf2', ?, '-250.00', 'AUD', 0),
                    ('pf2b', 'tf2', ?, '250.00', 'AUD', 1)",
        )
        .bind(wallet.to_string())
        .bind(income.to_string())
        .execute(&pool)
        .await
        .expect("postings 2");

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        assert_eq!(inflow.value(), dec!(1000.00));
        assert_eq!(inflow.commodity().as_str(), "AUD");
        assert_eq!(outflow.value(), dec!(250.00));
        assert_eq!(outflow.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_includes_all_reconciliation_states(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use jiff::civil::date;

        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");

        // Transaction with unreconciled state — should still be included.
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tu1', '2026-04-15', 'Unreconciled', 'unreconciled', '2026-04-15T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("unreconciled tx");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pu1', 'tu1', ?, '500.00', 'AUD', 0)",
        )
        .bind(wallet.to_string())
        .execute(&pool)
        .await
        .expect("posting");

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        // Unreconciled transactions are included.
        assert_eq!(inflow.value(), rust_decimal::Decimal::from(500_i32));
        assert_eq!(inflow.commodity().as_str(), "AUD");
        assert_eq!(outflow.value(), rust_decimal::Decimal::ZERO);
        assert_eq!(outflow.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_flows_respects_date_boundary(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");

        // Inside range
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tb1', '2026-04-01', 'In', 'reconciled', '2026-04-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx in");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pb1', 'tb1', ?, '100.00', 'AUD', 0)",
        )
        .bind(wallet.to_string())
        .execute(&pool)
        .await
        .expect("posting in");

        // On the exclusive upper bound — should be excluded
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tb2', '2026-05-01', 'Boundary', 'reconciled', '2026-05-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx boundary");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pb2', 'tb2', ?, '999.00', 'AUD', 0)",
        )
        .bind(wallet.to_string())
        .execute(&pool)
        .await
        .expect("posting boundary");

        let engine = Engine::new(pool.clone());
        let (inflow, _) = engine
            .posting_flows(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        // Only the 100.00 inside [2026-04-01, 2026-05-01) should be counted
        assert_eq!(inflow.value(), dec!(100.00));
        assert_eq!(inflow.commodity().as_str(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_buckets_returns_correct_count(pool: sqlx::SqlitePool) {
        use core::num::NonZeroUsize;

        use bc_models::AccountKind;
        use bc_models::AccountType;
        use bc_models::Period;
        use jiff::civil::date;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Test")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        let engine = Engine::new(pool.clone());
        let buckets = engine
            .posting_buckets(
                &acc,
                "AUD",
                &Period::Monthly,
                NonZeroUsize::new(6).expect("6 > 0"),
                date(2026, 5, 25),
            )
            .await
            .expect("posting_buckets");

        // Should return exactly 6 monthly buckets, oldest first.
        // as_of = 2026-05-25 → current bucket = [2026-05-01, 2026-06-01)
        // Going back 5 more months: 2026-04, 2026-03, 2026-02, 2026-01, 2025-12
        assert_eq!(buckets.len(), 6);
        #[expect(
            clippy::indexing_slicing,
            reason = "test assertions on known-length vec"
        )]
        {
            assert_eq!(buckets[0].start.to_string(), "2025-12-01");
            assert_eq!(buckets[5].start.to_string(), "2026-05-01");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_buckets_assigns_postings_to_correct_bucket(pool: sqlx::SqlitePool) {
        use core::num::NonZeroUsize;

        use bc_models::AccountKind;
        use bc_models::AccountType;
        use bc_models::Period;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Test")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");

        // One inflow in April, one outflow in May
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tbk1', '2026-04-15', 'April pay', 'reconciled', '2026-04-15T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx april");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pbk1', 'tbk1', ?, '500.00', 'AUD', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("posting april");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at)
             VALUES ('tbk2', '2026-05-10', 'May rent', 'reconciled', '2026-05-10T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx may");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position)
             VALUES ('pbk2', 'tbk2', ?, '-200.00', 'AUD', 0)",
        )
        .bind(acc.to_string())
        .execute(&pool)
        .await
        .expect("posting may");

        let engine = Engine::new(pool.clone());
        let buckets = engine
            .posting_buckets(
                &acc,
                "AUD",
                &Period::Monthly,
                NonZeroUsize::new(2).expect("2 > 0"),
                date(2026, 5, 25),
            )
            .await
            .expect("posting_buckets");

        assert_eq!(buckets.len(), 2);
        #[expect(
            clippy::indexing_slicing,
            reason = "test assertions on known-length vec"
        )]
        {
            // First bucket: April (inflow 500, outflow 0)
            assert_eq!(buckets[0].start.to_string(), "2026-04-01");
            assert_eq!(buckets[0].inflow.value(), dec!(500.00));
            assert_eq!(buckets[0].inflow.commodity().as_str(), "AUD");
            assert_eq!(buckets[0].outflow.value(), rust_decimal::Decimal::ZERO);
            assert_eq!(buckets[0].outflow.commodity().as_str(), "AUD");
            // Second bucket: May (inflow 0, outflow 200)
            assert_eq!(buckets[1].start.to_string(), "2026-05-01");
            assert_eq!(buckets[1].inflow.value(), rust_decimal::Decimal::ZERO);
            assert_eq!(buckets[1].inflow.commodity().as_str(), "AUD");
            assert_eq!(buckets[1].outflow.value(), dec!(200.00));
            assert_eq!(buckets[1].outflow.commodity().as_str(), "AUD");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn balance_includes_all_reconciliation_states(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc_a = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet should succeed");
        let acc_b = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income should succeed");

        let tx_svc = crate::transaction::Service::new(pool.clone());
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 1, 1))
            .description("Unreconciled")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_a.clone())
                    .amount(Amount::new(dec!(100), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(acc_b)
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        tx_svc.create(tx).await.expect("create should succeed");

        let engine = Engine::new(pool.clone());
        let balance = engine
            .balance_for(&acc_a, "AUD")
            .await
            .expect("balance query should succeed");
        // Unreconciled transactions are included in balances.
        assert_eq!(balance.value(), dec!(100));
        assert_eq!(balance.commodity().as_str(), "AUD");
    }

    /// Seeds `wallet` with one transaction per `(date, amount)` pair in AUD,
    /// each balanced against a counter "Other" account.
    async fn seed_postings_aud(
        pool: &sqlx::SqlitePool,
        wallet: &AccountId,
        pairs: &[(jiff::civil::Date, Decimal)],
    ) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let other = acct_svc
            .create()
            .name("Other")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Other account should succeed");

        let tx_svc = crate::transaction::Service::new(pool.clone());
        for &(date, amount) in pairs {
            let tx = Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(date)
                .description("Seed")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(wallet.clone())
                        .amount(Amount::new(amount, CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(other.clone())
                        .amount(Amount::new(
                            Decimal::ZERO
                                .checked_sub(amount)
                                .expect("negation should not overflow"),
                            CommodityCode::new("AUD"),
                        ))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(jiff::Timestamp::now())
                .build();
            tx_svc
                .create(tx)
                .await
                .expect("seed transaction should succeed");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_period_stats_windows_flows_and_balances(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let acc = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet should succeed");

        seed_postings_aud(
            &pool,
            &acc,
            &[
                (date(2026, 5, 20), dec!(100)),
                (date(2026, 6, 10), dec!(-30)),
                (date(2026, 6, 20), dec!(50)),
            ],
        )
        .await;

        let engine = Engine::new(pool.clone());
        let s = engine
            .account_period_stats(&acc, "AUD", date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("account_period_stats should succeed");

        assert_eq!(s.inflow.value(), dec!(50));
        assert_eq!(s.outflow.value(), dec!(30));
        assert_eq!(s.net.value(), dec!(20));
        assert_eq!(s.opening.value(), dec!(100));
        assert_eq!(s.closing.value(), dec!(120));
        assert_eq!(s.tx_count, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn period_stats_for_set_unions_accounts_and_dedups_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let mk = |name: &'static str, ty: AccountType| {
            let svc = acct_svc.clone();
            async move {
                svc.create()
                    .name(name)
                    .account_type(ty)
                    .kind(AccountKind::DepositAccount)
                    .call()
                    .await
                    .expect(name)
            }
        };
        let cheque = mk("Cheque", AccountType::Asset).await;
        let savings = mk("Savings", AccountType::Asset).await;
        let income = mk("Income", AccountType::Income).await;

        // tx_1 (Jan 10): income -> cheque 100
        // tx_2 (Jan 20): cheque -> savings 40  (both legs inside the set: internal)
        // tx_0 (Dec 1, before window): income -> savings 10
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES \
            ('tx_0', '2025-12-01', 'o', 'reconciled', '2025-12-01T00:00:00Z'), \
            ('tx_1', '2026-01-10', 'a', 'reconciled', '2026-01-10T00:00:00Z'), \
            ('tx_2', '2026-01-20', 'b', 'reconciled', '2026-01-20T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("txs");
        for (id, tx, acct, amt, pos) in [
            ("p0a", "tx_0", &savings, "10.00", 0_i32),
            ("p0b", "tx_0", &income, "-10.00", 1_i32),
            ("p1a", "tx_1", &cheque, "100.00", 0_i32),
            ("p1b", "tx_1", &income, "-100.00", 1_i32),
            ("p2a", "tx_2", &savings, "40.00", 0_i32),
            ("p2b", "tx_2", &cheque, "-40.00", 1_i32),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, '{amt}', 'AUD', ?)"
            )))
                .bind(id)
                .bind(tx)
                .bind(acct.to_string())
                .bind(pos)
                .execute(&pool)
                .await
                .expect("posting");
        }

        let engine = Engine::new(pool.clone());
        let stats = engine
            .account_period_stats_for_set(
                &[cheque.clone(), savings.clone()],
                "AUD",
                date(2026, 1, 1),
                date(2026, 2, 1),
            )
            .await
            .expect("stats");

        assert_eq!(stats.opening.value(), dec!(10.00));
        assert_eq!(stats.inflow.value(), dec!(100.00));
        assert_eq!(stats.outflow.value(), dec!(0));
        // tx_2 moves 40 between two accounts of the set.
        assert_eq!(stats.internal.value(), dec!(40.00));
        assert_eq!(stats.closing.value(), dec!(110.00));
        // tx_2 touches both accounts and must count once.
        assert_eq!(stats.tx_count, 2);

        let buckets = engine
            .posting_buckets_for_set(
                &[cheque, savings],
                "AUD",
                &Period::Monthly,
                NonZeroUsize::new(2).expect("2 > 0"),
                date(2026, 1, 15),
            )
            .await
            .expect("buckets");
        assert_eq!(buckets.len(), 2);
        #[expect(
            clippy::indexing_slicing,
            reason = "test assertions on known-length vec"
        )]
        {
            assert_eq!(buckets[0].inflow.value(), dec!(10.00));
            assert_eq!(buckets[1].inflow.value(), dec!(100.00));
            assert_eq!(buckets[1].outflow.value(), dec!(0));
        }
    }

    /// The spec's worked cases, viewed from a scope of two asset accounts.
    #[sqlx::test(migrations = "./migrations")]
    async fn period_stats_for_set_nets_movement_inside_the_scope(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let mk = |name: &'static str, ty: AccountType| {
            let svc = acct_svc.clone();
            async move {
                svc.create()
                    .name(name)
                    .account_type(ty)
                    .kind(AccountKind::DepositAccount)
                    .call()
                    .await
                    .expect(name)
            }
        };
        let everyday = mk("Everyday", AccountType::Asset).await;
        let savings = mk("Savings", AccountType::Asset).await;
        let salary = mk("Salary", AccountType::Income).await;
        let fees = mk("Fees", AccountType::Expense).await;
        let groceries = mk("Groceries", AccountType::Expense).await;

        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES \
            ('tx_t', '2026-01-05', 'transfer', 'unreconciled', '2026-01-05T00:00:00Z'), \
            ('tx_f', '2026-01-10', 'transfer with fee', 'unreconciled', '2026-01-10T00:00:00Z'), \
            ('tx_p', '2026-01-15', 'pay split', 'unreconciled', '2026-01-15T00:00:00Z'), \
            ('tx_g', '2026-01-20', 'groceries', 'unreconciled', '2026-01-20T00:00:00Z'), \
            ('tx_e', '2026-01-25', 'elided transfer', 'unreconciled', '2026-01-25T00:00:00Z'), \
            ('tx_x', '2026-01-28', 'cross-commodity', 'unreconciled', '2026-01-28T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("txs");
        for (id, tx, acct, amt, code, pos) in [
            ("pt1", "tx_t", &everyday, "-100.00", "AUD", 0_i32),
            ("pt2", "tx_t", &savings, "100.00", "AUD", 1_i32),
            ("pf1", "tx_f", &everyday, "-1005.00", "AUD", 0_i32),
            ("pf2", "tx_f", &savings, "1000.00", "AUD", 1_i32),
            ("pf3", "tx_f", &fees, "5.00", "AUD", 2_i32),
            ("pp1", "tx_p", &salary, "-5000.00", "AUD", 0_i32),
            ("pp2", "tx_p", &everyday, "4000.00", "AUD", 1_i32),
            ("pp3", "tx_p", &savings, "1000.00", "AUD", 2_i32),
            ("pg1", "tx_g", &everyday, "-12.34", "AUD", 0_i32),
            ("pg2", "tx_g", &groceries, "12.34", "AUD", 1_i32),
            ("pe1", "tx_e", &savings, "200.00", "AUD", 0_i32),
            ("px1", "tx_x", &everyday, "-150.00", "AUD", 0_i32),
            ("px2", "tx_x", &savings, "100.00", "USD", 1_i32),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, '{amt}', '{code}', ?)"
            )))
            .bind(id)
            .bind(tx)
            .bind(acct.to_string())
            .bind(pos)
            .execute(&pool)
            .await
            .expect("posting");
        }
        // Everyday's leg of tx_e is elided; its residual is -200.00.
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('pe2', 'tx_e', ?, NULL, NULL, 1)")
            .bind(everyday.to_string())
            .execute(&pool)
            .await
            .expect("elided leg");

        let engine = Engine::new(pool.clone());
        let scope = [everyday.clone(), savings.clone()];
        let stats = engine
            .account_period_stats_for_set(&scope, "AUD", date(2026, 1, 1), date(2026, 2, 1))
            .await
            .expect("set stats");

        assert_eq!(stats.inflow.value(), dec!(5000.00));
        // Fee 5 + groceries 12.34 + the AUD side of the cross-commodity move 150.
        assert_eq!(stats.outflow.value(), dec!(167.34));
        // tx_t 100 + tx_f 1000 + tx_e 200 (elided leg resolved by residual).
        assert_eq!(stats.internal.value(), dec!(1300.00));
        assert_eq!(stats.net.value(), dec!(4832.66));
        assert_eq!(stats.tx_count, 6);

        let leaf = engine
            .account_period_stats(&everyday, "AUD", date(2026, 1, 1), date(2026, 2, 1))
            .await
            .expect("leaf stats");
        assert_eq!(leaf.inflow.value(), dec!(4000.00));
        assert_eq!(leaf.outflow.value(), dec!(1467.34));
        assert_eq!(leaf.internal.value(), dec!(0));

        let buckets = engine
            .posting_buckets_for_set(
                &scope,
                "AUD",
                &Period::Monthly,
                NonZeroUsize::new(1).expect("1 > 0"),
                date(2026, 1, 15),
            )
            .await
            .expect("buckets");
        let bucket = buckets.first().expect("one bucket");
        assert_eq!(bucket.inflow.value(), stats.inflow.value());
        assert_eq!(bucket.outflow.value(), stats.outflow.value());
    }

    /// Two accounts in the set each carry their own elided leg, in separate
    /// transactions, with different concrete legs so the two residuals differ.
    /// Each elided leg must resolve against its own transaction's residual, so
    /// neither account's flows are understated.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal/count sums in a test assertion, not production arithmetic"
    )]
    async fn period_stats_for_set_resolves_residuals_per_account(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let mk = |name: &'static str, ty: AccountType| {
            let svc = acct_svc.clone();
            async move {
                svc.create()
                    .name(name)
                    .account_type(ty)
                    .kind(AccountKind::DepositAccount)
                    .call()
                    .await
                    .expect(name)
            }
        };
        let bank_a = mk("BankA", AccountType::Asset).await;
        let bank_b = mk("BankB", AccountType::Asset).await;
        let food = mk("Food", AccountType::Expense).await;

        // tx_ra: Food +80.00 (concrete), BankA elided -> residual -80.00
        // tx_rb: Food +30.00 (concrete), BankB elided -> residual -30.00
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES \
            ('tx_ra', '2026-01-05', 'Groceries A', 'unreconciled', '2026-01-05T00:00:00Z'), \
            ('tx_rb', '2026-01-20', 'Groceries B', 'unreconciled', '2026-01-20T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("txs");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_ra_food', 'tx_ra', ?, '80.00', 'AUD', 0)")
            .bind(food.to_string())
            .execute(&pool)
            .await
            .expect("concrete leg A");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_ra_bank', 'tx_ra', ?, NULL, NULL, 1)")
            .bind(bank_a.to_string())
            .execute(&pool)
            .await
            .expect("elided leg A");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_rb_food', 'tx_rb', ?, '30.00', 'AUD', 0)")
            .bind(food.to_string())
            .execute(&pool)
            .await
            .expect("concrete leg B");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_rb_bank', 'tx_rb', ?, NULL, NULL, 1)")
            .bind(bank_b.to_string())
            .execute(&pool)
            .await
            .expect("elided leg B");

        let engine = Engine::new(pool.clone());
        let from = date(2026, 1, 1);
        let until = date(2026, 2, 1);

        let a_stats = engine
            .account_period_stats(&bank_a, "AUD", from, until)
            .await
            .expect("bank_a stats");
        let b_stats = engine
            .account_period_stats(&bank_b, "AUD", from, until)
            .await
            .expect("bank_b stats");
        assert_eq!(a_stats.outflow.value(), dec!(80.00));
        assert_eq!(b_stats.outflow.value(), dec!(30.00));

        let set_stats = engine
            .account_period_stats_for_set(&[bank_a, bank_b], "AUD", from, until)
            .await
            .expect("set stats");

        // Each residual must resolve against its own account: a misattribution
        // would drop one of the two distinct amounts instead of summing both.
        assert_eq!(set_stats.outflow.value(), dec!(110.00));
        assert_eq!(
            set_stats.outflow.value(),
            a_stats.outflow.value() + b_stats.outflow.value(),
        );
        assert_eq!(
            set_stats.inflow.value(),
            a_stats.inflow.value() + b_stats.inflow.value()
        );
        assert_eq!(
            set_stats.closing.value(),
            a_stats.closing.value() + b_stats.closing.value(),
        );
        assert_eq!(set_stats.tx_count, a_stats.tx_count + b_stats.tx_count);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_period_stats_tx_count_is_distinct_transactions(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let wallet = acct_svc
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet should succeed");
        let other = acct_svc
            .create()
            .name("Other")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Other account should succeed");

        // A single transaction with TWO postings to the wallet (a within-account
        // split). tx_count must count the transaction once, matching the
        // register's row count — not the two commodity-scoped postings.
        let tx = Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 6, 10))
            .description("Split")
            .postings(vec![
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(wallet.clone())
                    .amount(Amount::new(dec!(60), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(wallet.clone())
                    .amount(Amount::new(dec!(40), CommodityCode::new("AUD")))
                    .build(),
                Posting::builder()
                    .id(PostingId::new())
                    .account_id(other.clone())
                    .amount(Amount::new(dec!(-100), CommodityCode::new("AUD")))
                    .build(),
            ])
            .reconciliation(Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::now())
            .build();
        crate::transaction::Service::new(pool.clone())
            .create(tx)
            .await
            .expect("seed split transaction should succeed");

        let engine = Engine::new(pool.clone());
        let s = engine
            .account_period_stats(&wallet, "AUD", date(2026, 6, 1), date(2026, 7, 1))
            .await
            .expect("account_period_stats should succeed");

        // One distinct transaction, even though it has two wallet postings.
        assert_eq!(s.tx_count, 1);
        // Both wallet legs are positive, so the transaction nets to 60 + 40 in.
        assert_eq!(s.inflow.value(), dec!(100));
    }

    #[rstest]
    #[case::weekly(
        Period::Weekly,
        date(2025, 2, 10),
        date(2025, 1, 27),
        date(2025, 2, 17)
    )]
    #[case::monthly(Period::Monthly, date(2025, 3, 15), date(2025, 1, 1), date(2025, 4, 1))]
    #[case::quarterly(
        Period::Quarterly,
        date(2025, 8, 20),
        date(2025, 1, 1),
        date(2025, 10, 1)
    )]
    #[case::calendar_year(
        Period::CalendarYear,
        date(2025, 5, 4),
        date(2023, 1, 1),
        date(2026, 1, 1)
    )]
    fn bucket_ranges_are_contiguous_oldest_first(
        #[case] period: Period,
        #[case] as_of: Date,
        #[case] expected_first_start: Date,
        #[case] expected_last_end: Date,
    ) {
        let ranges = super::bucket_ranges(&period, NonZeroUsize::new(3).expect("3 > 0"), as_of);
        assert_eq!(ranges.len(), 3);

        let first = ranges.first().expect("three buckets");
        let last = ranges.last().expect("three buckets");
        assert_eq!(first.0, expected_first_start);
        assert_eq!(last.1, expected_last_end);
        // Newest bucket contains `as_of`.
        assert!(last.0 <= as_of && as_of < last.1);

        for pair in ranges.windows(2) {
            let [earlier, later] = pair else {
                unreachable!("windows(2) always yields two elements")
            };
            assert!(earlier.0 < later.0, "buckets must be oldest-first");
            assert_eq!(earlier.1, later.0, "buckets must be contiguous");
        }
    }

    /// Anchors the WASM-side mirror of this crate's calendar snapping.
    ///
    /// `bc-ui`'s `coverage_count` re-implements [`super::bucket_ranges`]'s
    /// snapping rules because `bc-core` is absent from the WASM bundle. These
    /// cases pin the `(period, count, as_of)` triples that `bc-ui`'s own tests
    /// assume, so a change to the snapping here fails on this side too.
    #[rstest]
    #[case::daily(
        Period::Custom { days: Some(1), weeks: None, months: None },
        20,
        date(2025, 2, 10),
        date(2025, 1, 22)
    )]
    #[case::weekly(Period::Weekly, 7, date(2025, 2, 10), date(2025, 1, 1))]
    #[case::monthly(Period::Monthly, 8, date(2025, 8, 19), date(2025, 1, 15))]
    #[case::quarterly(Period::Quarterly, 5, date(2025, 8, 19), date(2024, 8, 1))]
    #[case::calendar_year(Period::CalendarYear, 3, date(2025, 8, 19), date(2023, 6, 1))]
    fn bucket_ranges_cover_span_start(
        #[case] period: Period,
        #[case] count: usize,
        #[case] as_of: Date,
        #[case] span_start: Date,
    ) {
        let ranges =
            super::bucket_ranges(&period, NonZeroUsize::new(count).expect("count > 0"), as_of);
        let first = ranges.first().expect("at least one bucket");
        let last = ranges.last().expect("at least one bucket");
        assert!(
            first.0 <= span_start,
            "oldest bucket start {:?} must reach span start {span_start:?}",
            first.0
        );
        assert!(as_of < last.1, "newest bucket must contain as_of");
    }

    /// The #354 repro: `Expenses:Food +50 / Assets:Bank <elided>`.
    ///
    /// Before the fix `Assets:Bank` read zero while `Expenses:Food` read +50.
    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_moves_its_account_balance(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_e1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food', 'tx_e1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank', 'tx_e1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .balance_for(&food, "AUD")
                .await
                .expect("food balance")
                .value(),
            dec!(50.00),
        );
        assert_eq!(
            engine
                .balance_for(&bank, "AUD")
                .await
                .expect("bank balance")
                .value(),
            dec!(-50.00),
            "the elided leg must absorb the residual",
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn ambiguous_transaction_contributes_no_residual(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");
        let fun = acct_svc
            .create()
            .name("Fun")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Fun");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_e2', '2026-01-01', 'Ambiguous', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food2', 'tx_e2', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank2', 'tx_e2', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert first elided leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_fun2', 'tx_e2', ?, NULL, NULL, 2)")
            .bind(fun.to_string()).execute(&pool).await.expect("insert second elided leg");

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .balance_for(&bank, "AUD")
                .await
                .expect("bank balance")
                .value(),
            Decimal::ZERO,
            "a residual split across two elided legs is not attributable",
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn elided_leg_counts_toward_period_flows(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_e3', '2026-01-15', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food3', 'tx_e3', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank3', 'tx_e3', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&bank, "AUD", date(2026, 1, 1), date(2026, 2, 1))
            .await
            .expect("flows");

        assert_eq!(inflow.value(), Decimal::ZERO);
        assert_eq!(outflow.value(), dec!(50.00), "the residual is an outflow");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_include_elided_residuals(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_d1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_d', 'tx_d1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_d', 'tx_d1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());
        let balances = engine.default_balances().await.expect("default balances");

        // The bank account's only posting is elided, so its commodity is
        // inferable only from the residual.
        let bank_balance = balances.get(&bank).expect("bank must appear");
        assert_eq!(bank_balance.value(), dec!(-50.00));
        assert_eq!(bank_balance.commodity().as_str(), "AUD");
        assert_eq!(
            balances.get(&food).expect("food must appear").value(),
            dec!(50.00)
        );
    }

    /// The invariant the ledger migration reconciles against: every commodity
    /// closes to zero across all accounts once residuals are derived.
    #[sqlx::test(migrations = "./migrations")]
    async fn all_accounts_sum_to_zero_per_commodity(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");
        let rent = acct_svc
            .create()
            .name("Rent")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Rent");

        // Three transactions, each with an elided bank leg — the Beancount idiom.
        for (n, account, amount) in [
            ("1", &food, "50.00"),
            ("2", &rent, "1200.00"),
            ("3", &food, "25.50"),
        ] {
            sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES (?, '2026-01-01', 'Test', 'unreconciled', '2026-01-01T00:00:00Z')")
                .bind(format!("tx_z{n}")).execute(&pool).await.expect("insert transaction");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, ?, 'AUD', 0)")
                .bind(format!("p_c{n}")).bind(format!("tx_z{n}")).bind(account.to_string()).bind(amount)
                .execute(&pool).await.expect("insert concrete leg");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, NULL, NULL, 1)")
                .bind(format!("p_e{n}")).bind(format!("tx_z{n}")).bind(bank.to_string())
                .execute(&pool).await.expect("insert elided leg");
        }

        let engine = Engine::new(pool.clone());
        let mut total = Decimal::ZERO;
        for account in [&bank, &food, &rent] {
            total = total
                .checked_add(
                    engine
                        .balance_for(account, "AUD")
                        .await
                        .expect("balance")
                        .value(),
                )
                .expect("no overflow");
        }

        assert_eq!(
            total,
            Decimal::ZERO,
            "AUD must close to zero across all accounts"
        );

        // `default_balances` exercises a different path than `balance_for`: it
        // infers each account's commodity (tier-3 falls back to the residual
        // for Bank, whose every posting is elided) and intersects against the
        // set of non-archived accounts. It must agree with the direct sum.
        let default_balances = engine.default_balances().await.expect("default balances");
        let default_total = [&bank, &food, &rent]
            .into_iter()
            .try_fold(Decimal::ZERO, |acc, account| {
                let value = default_balances
                    .get(account)
                    .unwrap_or_else(|| panic!("{account} must appear in default_balances"))
                    .value();
                acc.checked_add(value)
            })
            .expect("no overflow");

        assert_eq!(
            default_total,
            Decimal::ZERO,
            "default_balances must also close to zero across all accounts"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn default_commodity_falls_back_to_the_residual(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_c1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_c', 'tx_c1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_c', 'tx_c1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .default_commodity_for(&bank)
                .await
                .expect("commodity"),
            Some("AUD".to_owned()),
        );
    }

    /// The residual tier takes the first residual commodity in transaction order.
    #[sqlx::test(migrations = "./migrations")]
    async fn residual_tier_takes_the_first_commodity_in_transaction_order(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        // Bank's own legs are all elided: tx_1 funds it in USD, tx_2 in AUD.
        // tx_2 is dated first, so date order and id order disagree.
        for (tx, code, date) in [("tx_1", "USD", "2026-01-02"), ("tx_2", "AUD", "2026-01-01")] {
            insert_tx(&pool, tx, date).await;
            insert_posting(
                &pool,
                &format!("{tx}_f"),
                tx,
                &food.to_string(),
                Some("5.00"),
                Some(code),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("{tx}_b"),
                tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let code = Engine::new(pool)
            .default_commodity_for(&bank)
            .await
            .expect("commodity");

        assert_eq!(code.as_deref(), Some("USD"));
    }

    /// Commodities used equally often resolve to the lowest code, whatever the
    /// insertion order, so the single-account path agrees with
    /// [`AccountTotals::defaults`].
    #[sqlx::test(migrations = "./migrations")]
    async fn most_used_tie_resolves_to_the_lowest_code(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        // The higher code is inserted first.
        for (tx, code) in [("tx_1", "USD"), ("tx_2", "AUD")] {
            insert_tx(&pool, tx, "2026-01-01").await;
            insert_posting(
                &pool,
                &format!("{tx}_b"),
                tx,
                &bank.to_string(),
                Some("5.00"),
                Some(code),
                0,
            )
            .await;
        }

        let code = Engine::new(pool)
            .default_commodity_for(&bank)
            .await
            .expect("commodity");

        assert_eq!(code.as_deref(), Some("AUD"));
    }

    /// Elided legs must not form their own `GROUP BY` bucket in the tier-2
    /// "most-used posting commodity" subselect. When they outnumber every
    /// stored commodity, an unguarded `GROUP BY p.commodity` returns the NULL
    /// group, `COALESCE` yields NULL, and tier 3 fires on an account that has
    /// stored commodities — dropping every concrete posting as "non-default".
    #[sqlx::test(migrations = "./migrations")]
    async fn stored_commodity_wins_over_more_numerous_elided_legs(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        // Bank holds one stored USD leg and three elided legs whose siblings are
        // in AUD, so the NULL group (3) outnumbers the USD group (1).
        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_s0', '2026-01-01', 'Stored', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_s', 'tx_s0', ?, '-20.00', 'USD', 0)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert stored leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_s', 'tx_s0', ?, '20.00', 'USD', 1)")
            .bind(food.to_string()).execute(&pool).await.expect("insert stored sibling");

        for (i, tx) in ["tx_s1", "tx_s2", "tx_s3"].into_iter().enumerate() {
            sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES (?, '2026-01-02', 'Groceries', 'unreconciled', '2026-01-02T00:00:00Z')")
                .bind(tx).execute(&pool).await.expect("insert transaction");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, '50.00', 'AUD', 0)")
                .bind(format!("p_food_{i}")).bind(tx).bind(food.to_string())
                .execute(&pool).await.expect("insert concrete leg");
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, NULL, NULL, 1)")
                .bind(format!("p_bank_{i}")).bind(tx).bind(bank.to_string())
                .execute(&pool).await.expect("insert elided leg");
        }

        let engine = Engine::new(pool.clone());

        assert_eq!(
            engine
                .default_commodity_for(&bank)
                .await
                .expect("commodity"),
            Some("USD".to_owned()),
            "a stored commodity must beat the elided legs' NULL group",
        );

        let balances = engine.default_balances().await.expect("default balances");
        assert_eq!(
            balances
                .get(&bank)
                .map(|a| (a.value(), a.commodity().as_str().to_owned())),
            Some((dec!(-20.00), "USD".to_owned())),
            "the stored USD leg must not be dropped as a non-default commodity",
        );
    }

    /// An archived account whose only postings are elided must not leak into
    /// `default_balances` via the residual fallback: the leg stream has no
    /// `archived_at` filter, so the fallback must intersect against the
    /// active-account set explicitly.
    #[sqlx::test(migrations = "./migrations")]
    async fn default_balances_excludes_archived_account_via_residual(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Bank");
        let food = acct_svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Food");

        sqlx::query("INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES ('tx_a1', '2026-01-01', 'Groceries', 'unreconciled', '2026-01-01T00:00:00Z')")
            .execute(&pool).await.expect("insert transaction");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_food_a', 'tx_a1', ?, '50.00', 'AUD', 0)")
            .bind(food.to_string()).execute(&pool).await.expect("insert concrete leg");
        sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES ('p_bank_a', 'tx_a1', ?, NULL, NULL, 1)")
            .bind(bank.to_string()).execute(&pool).await.expect("insert elided leg");

        acct_svc
            .archive(&bank, Cascade::Reject)
            .await
            .expect("archive Bank");

        let engine = Engine::new(pool.clone());
        let balances = engine.default_balances().await.expect("default balances");

        assert_eq!(
            balances.get(&bank),
            None,
            "archived account must not appear even though its only leg is elided"
        );
        assert_eq!(
            balances.get(&food).expect("food must appear").value(),
            dec!(50.00)
        );
    }

    /// Equal concrete-leg counts in two commodities pick the lower code.
    #[sqlx::test(migrations = "./migrations")]
    async fn tier_two_tie_picks_the_alphabetically_first_commodity(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        let other = make_account(&pool, "Other", AccountType::Income).await;
        for (tx, code) in [("tx_1", "AUD"), ("tx_2", "USD")] {
            insert_tx(&pool, tx, "2026-01-01").await;
            insert_posting(
                &pool,
                &format!("{tx}_a"),
                tx,
                &wallet.to_string(),
                Some("10.00"),
                Some(code),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("{tx}_b"),
                tx,
                &other.to_string(),
                Some("-10.00"),
                Some(code),
                1,
            )
            .await;
        }

        let balances = Engine::new(pool)
            .default_balances()
            .await
            .expect("default balances");

        assert_eq!(
            balances
                .get(&wallet)
                .map(|a| a.commodity().as_str().to_owned()),
            Some("AUD".to_owned())
        );
    }

    /// Sorts an account's balances into a comparable list.
    fn sorted_balances(balances: &Balances) -> Vec<(String, Decimal)> {
        let mut v: Vec<(String, Decimal)> =
            balances.iter().map(|(c, d)| (c.to_owned(), d)).collect();
        v.sort();
        v
    }

    /// The streamed per-account totals equal an independent fold over the
    /// generated legs with `residual_of` on their weights, on a ledger mixing
    /// concrete, elided, multi-commodity, priced and ambiguous transactions.
    ///
    /// The oracle works from the generated legs, because the raw ids written
    /// here are not valid model ids for `transaction::Service::list`.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        clippy::integer_division_remainder_used,
        reason = "a bounded LCG generator indexes a fixed account list"
    )]
    async fn account_totals_matches_a_leg_oracle(pool: sqlx::SqlitePool) {
        // (account, amount, price) per leg.
        type Leg<'a> = (
            &'a AccountId,
            Option<(&'a str, &'a str)>,
            Option<(&'a str, &'a str)>,
        );
        let mut accounts = Vec::new();
        for (name, ty) in [
            ("Bank", AccountType::Asset),
            ("Card", AccountType::Liability),
            ("Food", AccountType::Expense),
            ("Rent", AccountType::Expense),
            ("Salary", AccountType::Income),
        ] {
            accounts.push(make_account(&pool, name, ty).await);
        }
        let mut oracle: HashMap<String, Balances> = HashMap::new();
        let mut seed: u64 = 42;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            (seed >> 33_u32) % n
        };
        // One write transaction keeps the 300 inserts from paying a commit each.
        let mut db = pool.begin().await.expect("begin");
        for i in 0..300_u64 {
            let tx = format!("tx_{i:04}");
            sqlx::query(
                "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
                 VALUES (?, ?, 'Gen', 'unreconciled', '2026-01-01T00:00:00Z')",
            )
            .bind(&tx)
            .bind(format!("2026-{:02}-{:02}", 1 + i % 12, 1 + i % 28))
            .execute(&mut *db)
            .await
            .expect("insert tx");
            let a = &accounts[usize::try_from(next(5)).expect("index")];
            let b = &accounts[usize::try_from(next(5)).expect("index")];
            let value = format!("{}.{:02}", next(500), next(100));
            let neg = format!("-{value}");
            let legs: Vec<Leg<'_>> = match next(5) {
                0 => vec![
                    (a, Some((value.as_str(), "AUD")), None),
                    (b, Some((neg.as_str(), "AUD")), None),
                ],
                1 => vec![(a, Some((value.as_str(), "AUD")), None), (b, None, None)],
                2 => vec![
                    (a, Some((value.as_str(), "AUD")), None),
                    (a, Some((value.as_str(), "USD")), None),
                    (b, None, None),
                ],
                3 => vec![
                    (a, Some((value.as_str(), "USD")), Some(("1.50", "AUD"))),
                    (b, None, None),
                ],
                _ => vec![
                    (a, Some((value.as_str(), "AUD")), None),
                    (b, None, None),
                    (a, None, None),
                ],
            };
            // (account, stored amount, weight) per leg; a unit price weighs
            // the leg in the price commodity.
            let mut weighed: Vec<(String, Option<Amount>, Option<Amount>)> = Vec::new();
            for (position, (acct, amount, price)) in legs.into_iter().enumerate() {
                let stored = amount.map(|(v, c)| Amount::new(v.parse().expect("decimal"), c));
                let weight = match (stored.as_ref(), price) {
                    (Some(base), Some((unit, code))) => Some(Amount::new(
                        base.value()
                            .checked_mul(unit.parse::<Decimal>().expect("decimal"))
                            .expect("weight"),
                        code,
                    )),
                    (base, _) => base.cloned(),
                };
                weighed.push((acct.to_string(), stored, weight));
                sqlx::query(
                    "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, \
                     price_value, price_commodity, price_kind, position) \
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
                )
                .bind(format!("{tx}_{position}"))
                .bind(&tx)
                .bind(acct.to_string())
                .bind(amount.map(|(v, _)| v))
                .bind(amount.map(|(_, c)| c))
                .bind(price.map(|(v, _)| v))
                .bind(price.map(|(_, c)| c))
                .bind(price.map(|_| "unit"))
                .bind(i64::try_from(position).expect("position"))
                .execute(&mut *db)
                .await
                .expect("insert posting");
            }
            let residual = crate::residual::residual_of(weighed.iter().map(|(_, _, w)| w.as_ref()))
                .expect("residual");
            for (account, stored, _) in &weighed {
                let entry = oracle.entry(account.clone()).or_default();
                match (stored, &residual) {
                    (Some(amount), _) => entry.try_add(amount).expect("add"),
                    (None, crate::residual::Residual::Attributable(attributed)) => {
                        for (code, share) in attributed.iter() {
                            entry.try_add(&Amount::new(share, code)).expect("add");
                        }
                    }
                    (None, _) => {}
                }
            }
        }
        db.commit().await.expect("commit");
        let expected: BTreeMap<String, Vec<(String, Decimal)>> = oracle
            .iter()
            .map(|(id, b)| (id.clone(), sorted_balances(b)))
            .filter(|(_, v)| !v.is_empty())
            .collect();

        let totals = Engine::new(pool).account_totals().await.expect("totals");
        let actual: BTreeMap<String, Vec<(String, Decimal)>> = totals
            .holdings_all_for_test()
            .map(|(id, b)| (id.to_owned(), sorted_balances(b)))
            .filter(|(_, v)| !v.is_empty())
            .collect();

        assert!(!expected.is_empty(), "the generated ledger holds balances");
        assert_eq!(actual, expected);
    }

    /// A leg on an account id absent from `accounts` contributes to no view.
    #[sqlx::test(migrations = "./migrations")]
    async fn legs_on_an_unknown_account_are_skipped(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        // Foreign keys are per connection, so the pragma and both inserts share
        // one connection.
        let mut conn = pool.acquire().await.expect("acquire");
        sqlx::query("PRAGMA foreign_keys = OFF")
            .execute(&mut *conn)
            .await
            .expect("pragma");
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES ('tx_1', '2026-01-01', 'Test', 'unreconciled', '2026-01-01T00:00:00Z')",
        )
        .execute(&mut *conn)
        .await
        .expect("insert tx");
        for (id, account, amount, position) in [
            ("p_1", "acc_missing".to_owned(), "5.00", 0_i64),
            ("p_2", bank.to_string(), "-5.00", 1),
        ] {
            sqlx::query(
                "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
                 VALUES (?, 'tx_1', ?, ?, 'AUD', ?)",
            )
            .bind(id)
            .bind(account)
            .bind(amount)
            .bind(position)
            .execute(&mut *conn)
            .await
            .expect("insert posting");
        }
        drop(conn);

        let totals = Engine::new(pool).account_totals().await.expect("totals");

        let defaults: Vec<(AccountId, Amount)> = totals.defaults().collect();
        assert_eq!(
            defaults,
            vec![(bank.clone(), Amount::new(dec!(-5.00), "AUD"))]
        );
        let rollups: Vec<AccountId> = totals
            .rollups()
            .expect("rollups")
            .map(|(id, _)| id)
            .collect();
        assert_eq!(rollups, vec![bank.clone()]);
        let holdings: Vec<&str> = totals.holdings().map(|(id, _)| id).collect();
        assert_eq!(holdings, vec![bank.to_string().as_str()]);
    }

    /// An all-elided account defaults to its first residual commodity in
    /// ascending transaction id order. `tx_1` is dated after `tx_2`, so date
    /// order would pick AUD.
    #[sqlx::test(migrations = "./migrations")]
    async fn defaults_take_the_first_residual_commodity_in_transaction_id_order(
        pool: sqlx::SqlitePool,
    ) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (tx, code, date) in [("tx_1", "USD", "2026-02-01"), ("tx_2", "AUD", "2026-01-01")] {
            insert_tx(&pool, tx, date).await;
            insert_posting(
                &pool,
                &format!("{tx}_f"),
                tx,
                &food.to_string(),
                Some("5.00"),
                Some(code),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("{tx}_b"),
                tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let totals = Engine::new(pool).account_totals().await.expect("totals");

        let defaults: HashMap<AccountId, Amount> = totals.defaults().collect();
        assert_eq!(defaults.get(&bank), Some(&Amount::new(dec!(-5.00), "USD")));
    }

    /// Inserts a transaction with the given id and date.
    async fn insert_tx(pool: &sqlx::SqlitePool, id: &str, date: &str) {
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
             VALUES (?, ?, 'Test', 'unreconciled', '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(date)
        .execute(pool)
        .await
        .expect("insert transaction");
    }

    /// Inserts a posting; `amount`/`commodity` are `None` for an elided leg.
    async fn insert_posting(
        pool: &sqlx::SqlitePool,
        id: &str,
        tx_id: &str,
        account_id: &str,
        amount: Option<&str>,
        commodity: Option<&str>,
        position: i64,
    ) {
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(tx_id)
        .bind(account_id)
        .bind(amount)
        .bind(commodity)
        .bind(position)
        .execute(pool)
        .await
        .expect("insert posting");
    }

    /// Creates an account and returns its id.
    async fn make_account(
        pool: &sqlx::SqlitePool,
        name: &str,
        account_type: AccountType,
    ) -> bc_models::AccountId {
        crate::account::Service::new(pool.clone())
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
    }

    /// D1: inserting a posting copies its transaction's date.
    #[sqlx::test(migrations = "./migrations")]
    async fn posting_date_is_populated_on_insert(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        insert_tx(&pool, "tx_d1", "2026-01-15").await;
        insert_posting(
            &pool,
            "p_d1",
            "tx_d1",
            &wallet.to_string(),
            Some("10.00"),
            Some("AUD"),
            0,
        )
        .await;

        let (date,): (Option<String>,) =
            sqlx::query_as("SELECT date FROM postings WHERE id = 'p_d1'")
                .fetch_one(&pool)
                .await
                .expect("select date");

        assert_eq!(date, Some("2026-01-15".to_owned()));
    }

    /// D2: amending a transaction's date moves every one of its postings.
    #[sqlx::test(migrations = "./migrations")]
    async fn amending_a_transaction_date_moves_its_postings(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        insert_tx(&pool, "tx_d2", "2026-01-15").await;
        insert_posting(
            &pool,
            "p_d2",
            "tx_d2",
            &wallet.to_string(),
            Some("10.00"),
            Some("AUD"),
            0,
        )
        .await;

        sqlx::query("UPDATE transactions SET date = '2026-03-01' WHERE id = 'tx_d2'")
            .execute(&pool)
            .await
            .expect("amend date");

        let (date,): (Option<String>,) =
            sqlx::query_as("SELECT date FROM postings WHERE id = 'p_d2'")
                .fetch_one(&pool)
                .await
                .expect("select date");

        assert_eq!(date, Some("2026-03-01".to_owned()));
    }

    /// D3: re-pointing a posting at another transaction resyncs its date.
    #[sqlx::test(migrations = "./migrations")]
    async fn reparenting_a_posting_resyncs_its_date(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        insert_tx(&pool, "tx_a", "2026-01-15").await;
        insert_tx(&pool, "tx_b", "2026-06-30").await;
        insert_posting(
            &pool,
            "p_d3",
            "tx_a",
            &wallet.to_string(),
            Some("10.00"),
            Some("AUD"),
            0,
        )
        .await;

        sqlx::query("UPDATE postings SET transaction_id = 'tx_b' WHERE id = 'p_d3'")
            .execute(&pool)
            .await
            .expect("reparent");

        let (date,): (Option<String>,) =
            sqlx::query_as("SELECT date FROM postings WHERE id = 'p_d3'")
                .fetch_one(&pool)
                .await
                .expect("select date");

        assert_eq!(date, Some("2026-06-30".to_owned()));
    }

    /// D4: no posting's denormalised date may diverge from its transaction's.
    ///
    /// `postings.date` cannot be `NOT NULL`, because an `AFTER INSERT` trigger populates
    /// it after the row already exists. This assertion is the guard in its place: a write
    /// path the triggers miss leaves a NULL date, which would silently vanish from every
    /// windowed query rather than failing.
    #[sqlx::test(migrations = "./migrations")]
    async fn every_posting_date_matches_its_transaction(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_i1", "2026-01-15").await;
        insert_posting(
            &pool,
            "p_i1",
            "tx_i1",
            &wallet.to_string(),
            Some("-10.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(&pool, "p_i2", "tx_i1", &food.to_string(), None, None, 1).await;
        sqlx::query("UPDATE transactions SET date = '2026-02-20' WHERE id = 'tx_i1'")
            .execute(&pool)
            .await
            .expect("amend date");

        let (divergent,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM postings p
             JOIN transactions t ON t.id = p.transaction_id
             WHERE p.date IS NOT t.date",
        )
        .fetch_one(&pool)
        .await
        .expect("integrity check");

        assert_eq!(divergent, 0);
    }

    /// A transaction whose in-scope legs net to zero still counts once.
    #[sqlx::test(migrations = "./migrations")]
    async fn period_tx_count_includes_a_zero_residual_transaction(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_1", "2026-01-10").await;
        insert_posting(
            &pool,
            "p_in",
            "tx_1",
            &food.to_string(),
            Some("5.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_out",
            "tx_1",
            &food.to_string(),
            Some("-5.00"),
            Some("AUD"),
            1,
        )
        .await;
        insert_posting(&pool, "p_bank", "tx_1", &bank.to_string(), None, None, 2).await;

        let stats = Engine::new(pool)
            .account_period_stats(&bank, "AUD", date(2026, 1, 1), date(2026, 2, 1))
            .await
            .expect("stats");

        assert_eq!(stats.tx_count, 1);
        assert_eq!(stats.closing.value(), Decimal::ZERO);
    }

    /// A transaction with multiple postings to the same account must count
    /// once, and boundary transactions on `from`/`to` must land on the correct side.
    ///
    /// Double-counting the multi-posting transaction below, or an off-by-one in the
    /// boundary comparison, would fail this test.
    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_count_deduplicates_multi_posting_transactions(pool: sqlx::SqlitePool) {
        let wallet = make_account(&pool, "Wallet", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;

        // Two postings to `wallet` in the same transaction, inside the window — a
        // missing DISTINCT would count this transaction twice.
        insert_tx(&pool, "tx_multi", "2026-04-15").await;
        insert_posting(
            &pool,
            "p_multi_a",
            "tx_multi",
            &wallet.to_string(),
            Some("-40.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_multi_b",
            "tx_multi",
            &wallet.to_string(),
            Some("-10.00"),
            Some("AUD"),
            1,
        )
        .await;
        insert_posting(
            &pool,
            "p_multi_c",
            "tx_multi",
            &food.to_string(),
            Some("50.00"),
            Some("AUD"),
            2,
        )
        .await;

        // On the inclusive lower boundary.
        insert_tx(&pool, "tx_lo", "2026-04-01").await;
        insert_posting(
            &pool,
            "p_lo",
            "tx_lo",
            &wallet.to_string(),
            Some("1.00"),
            Some("AUD"),
            0,
        )
        .await;

        // On the exclusive upper boundary — must be excluded.
        insert_tx(&pool, "tx_hi", "2026-05-01").await;
        insert_posting(
            &pool,
            "p_hi",
            "tx_hi",
            &wallet.to_string(),
            Some("1.00"),
            Some("AUD"),
            0,
        )
        .await;

        let engine = Engine::new(pool.clone());
        let stats = engine
            .account_period_stats(&wallet, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("stats");

        // tx_multi counts once despite two postings, plus tx_lo; tx_hi is excluded.
        assert_eq!(stats.tx_count, 2);
    }

    /// C1: splitting a window anywhere must not change the total.
    ///
    /// The single strongest invariant here: it fails if the opening query's upper bound
    /// and the in-window query's lower bound ever disagree.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal sums in a test assertion, not production arithmetic"
    )]
    async fn period_net_is_additive_across_a_split(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, day, amount) in [
            ("1", "2026-01-05", "10.00"),
            ("2", "2026-02-20", "20.00"),
            ("3", "2026-04-10", "30.00"),
            ("4", "2026-05-25", "40.00"),
        ] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let engine = Engine::new(pool.clone());
        let from = date(2026, 1, 1);
        let until = date(2026, 6, 1);
        for split in [date(2026, 1, 1), date(2026, 3, 15), date(2026, 6, 1)] {
            let whole = engine
                .account_period_stats(&bank, "AUD", from, until)
                .await
                .expect("whole");
            let left = engine
                .account_period_stats(&bank, "AUD", from, split)
                .await
                .expect("left");
            let right = engine
                .account_period_stats(&bank, "AUD", split, until)
                .await
                .expect("right");

            assert_eq!(
                whole.net.value(),
                left.net.value() + right.net.value(),
                "net is not additive across {split}"
            );
            assert_eq!(
                right.opening.value(),
                left.opening.value() + left.net.value(),
                "opening/net disagree at {split}",
            );
            assert_eq!(
                whole.closing.value(),
                right.closing.value(),
                "closing disagrees across {split}"
            );
        }
    }

    /// C2: opening plus net equals closing.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal sum in a test assertion, not production arithmetic"
    )]
    async fn opening_plus_net_equals_closing(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, day, amount) in [("1", "2025-11-01", "100.00"), ("2", "2026-02-20", "20.00")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let stats = Engine::new(pool.clone())
            .account_period_stats(&bank, "AUD", date(2026, 1, 1), date(2026, 6, 1))
            .await
            .expect("stats");

        assert_eq!(stats.opening.value(), dec!(-100.00));
        assert_eq!(stats.net.value(), dec!(-20.00));
        assert_eq!(
            stats.closing.value(),
            stats.opening.value() + stats.net.value()
        );
    }

    /// C3: the windowed path agrees with the unwindowed one.
    ///
    /// `balance_for` is untouched by this change, so it is a clean oracle.
    #[sqlx::test(migrations = "./migrations")]
    async fn full_span_closing_equals_balance_for(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        for (n, day, amount) in [("1", "2020-03-04", "12.34"), ("2", "2026-02-20", "56.78")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let engine = Engine::new(pool.clone());
        let stats = engine
            .account_period_stats(&bank, "AUD", jiff::civil::Date::MIN, jiff::civil::Date::MAX)
            .await
            .expect("stats");
        let direct = engine.balance_for(&bank, "AUD").await.expect("balance_for");

        assert_eq!(stats.closing.value(), direct.value());
    }

    /// C4: every account's period net sums to zero, per commodity, for any window.
    ///
    /// Catches a residual dropped for one account but not its counterparty — a shape the
    /// per-account tests cannot see. Includes a USD leg alongside the AUD ones so the "per
    /// commodity" claim is actually exercised: with only one commodity in the fixture, a bug
    /// that crossed commodities (e.g. summing a USD residual into an AUD total) could not
    /// show up here.
    #[sqlx::test(migrations = "./migrations")]
    #[expect(
        clippy::arithmetic_side_effects,
        reason = "decimal accumulation in a test assertion, not production arithmetic"
    )]
    async fn period_nets_sum_to_zero_across_all_accounts(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        let fun = make_account(&pool, "Fun", AccountType::Expense).await;
        for (n, acct, day, amount) in [
            ("1", &food, "2026-02-20", "20.00"),
            ("2", &fun, "2026-03-05", "35.00"),
        ] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_c_{n}"),
                &tx,
                &acct.to_string(),
                Some(amount),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        // A USD-denominated transaction, elided the same way, so the USD total is
        // exercised independently of the AUD total above.
        insert_tx(&pool, "tx_usd", "2026-04-10").await;
        insert_posting(
            &pool,
            "p_c_usd",
            "tx_usd",
            &fun.to_string(),
            Some("15.00"),
            Some("USD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_bank_usd",
            "tx_usd",
            &bank.to_string(),
            None,
            None,
            1,
        )
        .await;

        let engine = Engine::new(pool.clone());
        for commodity in ["AUD", "USD"] {
            let mut total = Decimal::ZERO;
            for acct in [&bank, &food, &fun] {
                let stats = engine
                    .account_period_stats(acct, commodity, date(2026, 1, 1), date(2026, 6, 1))
                    .await
                    .expect("stats");
                total += stats.net.value();
            }
            assert_eq!(
                total,
                Decimal::ZERO,
                "commodity {commodity} did not net to zero"
            );
        }
    }

    /// F: the genesis sentinel sorts below every real date.
    ///
    /// Dates are TEXT and compared lexicographically, so `Date::MIN` (-9999-01-01) has to
    /// stringify to something that sorts below a four-digit positive year. The test
    /// asserts the behaviour rather than pinning the format, so it survives a jiff
    /// formatting change while still catching a real ordering break.
    #[sqlx::test(migrations = "./migrations")]
    async fn opening_balance_includes_the_earliest_transactions(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        insert_tx(&pool, "tx_ancient", "0001-01-01").await;
        insert_posting(
            &pool,
            "p_food",
            "tx_ancient",
            &food.to_string(),
            Some("7.00"),
            Some("AUD"),
            0,
        )
        .await;
        insert_posting(
            &pool,
            "p_bank",
            "tx_ancient",
            &bank.to_string(),
            None,
            None,
            1,
        )
        .await;

        let stats = Engine::new(pool.clone())
            .account_period_stats(&bank, "AUD", date(2026, 1, 1), date(2026, 6, 1))
            .await
            .expect("stats");

        assert_eq!(stats.opening.value(), dec!(-7.00));
    }

    /// A3: the window is half-open on the elided path as well as the concrete one.
    ///
    /// `posting_flows_respects_date_boundary` covers only concrete legs. This is its
    /// elided twin, and it is what catches `<=` versus `<` drift between the driving
    /// elided query and the residual subquery.
    #[sqlx::test(migrations = "./migrations")]
    async fn elided_legs_respect_the_half_open_boundary(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        // One transaction on the inclusive lower bound, one on the exclusive upper bound.
        for (n, day) in [("lo", "2026-04-01"), ("hi", "2026-05-01")] {
            let tx = format!("tx_{n}");
            insert_tx(&pool, &tx, day).await;
            insert_posting(
                &pool,
                &format!("p_food_{n}"),
                &tx,
                &food.to_string(),
                Some("50.00"),
                Some("AUD"),
                0,
            )
            .await;
            insert_posting(
                &pool,
                &format!("p_bank_{n}"),
                &tx,
                &bank.to_string(),
                None,
                None,
                1,
            )
            .await;
        }

        let engine = Engine::new(pool.clone());
        let (inflow, outflow) = engine
            .posting_flows(&bank, "AUD", date(2026, 4, 1), date(2026, 5, 1))
            .await
            .expect("posting_flows");

        // Only the 2026-04-01 transaction is in window; its elided bank leg absorbs -50.
        assert_eq!(inflow.value(), dec!(0));
        assert_eq!(outflow.value(), dec!(50.00));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_group_account_contributes_zero_without_warning(pool: SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let group = accounts
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Group)
            .call()
            .await
            .expect("create the group account");

        let engine = Engine::new(pool.clone());
        let report = engine
            .net_worth("AUD", &crate::fx::NoopFxRateService)
            .await
            .expect("net worth");

        assert_eq!(
            report.total.value(),
            Decimal::ZERO,
            "a Group account holds no postings, so it contributes nothing"
        );
        assert_eq!(
            report
                .rows
                .iter()
                .map(|row| (row.balance.clone(), row.valuation.clone()))
                .collect::<Vec<_>>(),
            vec![(Amount::new(Decimal::ZERO, "AUD"), Valuation::Native)],
            "an account with no holdings still gets one zero row, so it is not mistaken for absent"
        );
        assert_eq!(
            engine
                .balance_for(&group, "AUD")
                .await
                .expect("balance")
                .value(),
            Decimal::ZERO
        );
    }

    /// Creates an active asset `DepositAccount` and an income counterpart.
    async fn wallet_and_income(pool: &sqlx::SqlitePool) -> (AccountId, AccountId) {
        let accounts = crate::account::Service::new(pool.clone());
        let wallet = accounts
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Wallet");
        let income = accounts
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create Income");
        (wallet, income)
    }

    /// Posts `amount` of `commodity` into `wallet` from `income` under `tx_id`.
    async fn fund_wallet(
        pool: &sqlx::SqlitePool,
        tx_id: &str,
        wallet: &AccountId,
        income: &AccountId,
        amount: &str,
        commodity: &str,
    ) {
        insert_tx(pool, tx_id, "2026-01-01").await;
        insert_posting(
            pool,
            &format!("{tx_id}_w"),
            tx_id,
            &wallet.to_string(),
            Some(amount),
            Some(commodity),
            0,
        )
        .await;
        insert_posting(
            pool,
            &format!("{tx_id}_i"),
            tx_id,
            &income.to_string(),
            Some(&format!("-{amount}")),
            Some(commodity),
            1,
        )
        .await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn net_worth_reports_a_foreign_holding_it_cannot_value(pool: sqlx::SqlitePool) {
        let (wallet, income) = wallet_and_income(&pool).await;
        fund_wallet(&pool, "tx_aud", &wallet, &income, "100.00", "AUD").await;
        fund_wallet(&pool, "tx_btc", &wallet, &income, "0.5", "BTC").await;

        let report = Engine::new(pool.clone())
            .net_worth("AUD", &crate::fx::NoopFxRateService)
            .await
            .expect("net worth");

        assert_eq!(report.total, Amount::new(dec!(100.00), "AUD"));
        assert_eq!(report.unvalued.get("BTC"), Some(dec!(0.5)));
        assert!(report.converted.is_empty());
        assert_eq!(
            report
                .rows
                .iter()
                .map(|row| (row.balance.clone(), row.valuation.clone()))
                .collect::<Vec<_>>(),
            vec![
                (Amount::new(dec!(100.00), "AUD"), Valuation::Native),
                (Amount::new(dec!(0.5), "BTC"), Valuation::Unvalued),
            ]
        );
    }

    /// Values every BTC at a flat 100 000 AUD; anything else is unavailable.
    struct FlatBtcRate;

    impl crate::fx::FxRateService for FlatBtcRate {
        fn convert(
            &self,
            amount: &Amount,
            to_commodity: &CommodityCode,
        ) -> Result<Amount, crate::fx::FxError> {
            if amount.commodity().as_str() == "BTC" && to_commodity.as_str() == "AUD" {
                let value = amount
                    .value()
                    .checked_mul(dec!(100_000))
                    .expect("test amounts are small");
                return Ok(Amount::new(value, "AUD"));
            }
            crate::fx::NoopFxRateService.convert(amount, to_commodity)
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn net_worth_converts_a_foreign_holding_when_a_rate_exists(pool: sqlx::SqlitePool) {
        let (wallet, income) = wallet_and_income(&pool).await;
        fund_wallet(&pool, "tx_aud", &wallet, &income, "100.00", "AUD").await;
        fund_wallet(&pool, "tx_btc", &wallet, &income, "0.5", "BTC").await;

        let report = Engine::new(pool.clone())
            .net_worth("AUD", &FlatBtcRate)
            .await
            .expect("net worth");

        assert_eq!(report.total, Amount::new(dec!(50100.00), "AUD"));
        assert_eq!(report.converted.get("BTC"), Some(dec!(0.5)));
        assert!(report.unvalued.is_empty());
        assert_eq!(
            report.rows.get(1).map(|row| row.valuation.clone()),
            Some(Valuation::Converted(Amount::new(dec!(50000.0), "AUD")))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn net_worth_reports_a_manual_asset_valued_in_another_commodity(pool: sqlx::SqlitePool) {
        use bc_models::ValuationSource;

        let house = crate::account::Service::new(pool.clone())
            .create()
            .name("House")
            .account_type(AccountType::Asset)
            .kind(AccountKind::ManualAsset)
            .call()
            .await
            .expect("create ManualAsset");
        let assets = crate::asset::Service::new(pool.clone());
        // An older AUD valuation superseded by a newer USD one: the newest wins
        // regardless of commodity, so the AUD figure must not resurface.
        assets
            .record_valuation(
                &house,
                dec!(400_000),
                "AUD",
                ValuationSource::ManualEstimate,
                date(2025, 1, 1),
                None,
            )
            .await
            .expect("record AUD valuation");
        assets
            .record_valuation(
                &house,
                dec!(300_000),
                "USD",
                ValuationSource::ProfessionalAppraisal,
                date(2026, 1, 1),
                None,
            )
            .await
            .expect("record USD valuation");

        let report = Engine::new(pool.clone())
            .net_worth("AUD", &crate::fx::NoopFxRateService)
            .await
            .expect("net worth");

        assert_eq!(report.total, Amount::new(Decimal::ZERO, "AUD"));
        assert_eq!(report.unvalued.get("USD"), Some(dec!(300_000)));
        assert_eq!(
            report
                .rows
                .iter()
                .map(|row| (row.balance.clone(), row.valuation.clone()))
                .collect::<Vec<_>>(),
            vec![(Amount::new(dec!(300_000), "USD"), Valuation::Unvalued)]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn net_worth_counts_an_elided_foreign_leg(pool: sqlx::SqlitePool) {
        let (wallet, income) = wallet_and_income(&pool).await;
        insert_tx(&pool, "tx_el", "2026-01-01").await;
        insert_posting(
            &pool,
            "p_el_i",
            "tx_el",
            &income.to_string(),
            Some("-0.25"),
            Some("BTC"),
            0,
        )
        .await;
        insert_posting(&pool, "p_el_w", "tx_el", &wallet.to_string(), None, None, 1).await;

        let report = Engine::new(pool.clone())
            .net_worth("AUD", &crate::fx::NoopFxRateService)
            .await
            .expect("net worth");

        assert_eq!(report.total, Amount::new(Decimal::ZERO, "AUD"));
        assert_eq!(report.unvalued.get("BTC"), Some(dec!(0.25)));
        assert_eq!(
            report
                .rows
                .iter()
                .map(|row| (row.balance.clone(), row.valuation.clone()))
                .collect::<Vec<_>>(),
            vec![(Amount::new(dec!(0.25), "BTC"), Valuation::Unvalued)]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollup_balances_folds_subtree_per_commodity(pool: sqlx::SqlitePool) {
        let acct_svc = crate::account::Service::new(pool.clone());
        let assets = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("assets");
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("bank");
        let savings = acct_svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&bank)
            .call()
            .await
            .expect("savings");
        let broker = acct_svc
            .create()
            .name("Broker")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("broker");
        let old = acct_svc
            .create()
            .name("Old")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("old");
        let income = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("income");
        sqlx::query("UPDATE accounts SET archived_at = '2026-01-01T00:00:00Z' WHERE id = ?")
            .bind(old.to_string())
            .execute(&pool)
            .await
            .expect("archive old");

        // tx_1: savings +100 AUD, income -100 AUD (concrete legs)
        // tx_2: broker +30 USD, income elided (residual -30 USD)
        // tx_3: bank +5 AUD, income -5 AUD (bank's own posting)
        // tx_4: old +999 AUD, income -999 AUD (archived; must not roll up)
        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES \
            ('tx_1', '2026-01-01', 'a', 'reconciled', '2026-01-01T00:00:00Z'), \
            ('tx_2', '2026-01-02', 'b', 'reconciled', '2026-01-01T00:00:00Z'), \
            ('tx_3', '2026-01-03', 'c', 'reconciled', '2026-01-01T00:00:00Z'), \
            ('tx_4', '2026-01-04', 'd', 'reconciled', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("txs");
        let posting = |id: &str,
                       tx: &str,
                       acct: &AccountId,
                       amt: Option<&str>,
                       com: Option<&str>,
                       pos: i32| {
            sqlx::query("INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES (?, ?, ?, ?, ?, ?)")
                .bind(id.to_owned())
                .bind(tx.to_owned())
                .bind(acct.to_string())
                .bind(amt.map(ToOwned::to_owned))
                .bind(com.map(ToOwned::to_owned))
                .bind(pos)
        };
        for q in [
            posting("p1", "tx_1", &savings, Some("100.00"), Some("AUD"), 0_i32),
            posting("p2", "tx_1", &income, Some("-100.00"), Some("AUD"), 1_i32),
            posting("p3", "tx_2", &broker, Some("30.00"), Some("USD"), 0_i32),
            posting("p4", "tx_2", &income, None, None, 1_i32),
            posting("p5", "tx_3", &bank, Some("5.00"), Some("AUD"), 0_i32),
            posting("p6", "tx_3", &income, Some("-5.00"), Some("AUD"), 1_i32),
            posting("p7", "tx_4", &old, Some("999.00"), Some("AUD"), 0_i32),
            posting("p8", "tx_4", &income, Some("-999.00"), Some("AUD"), 1_i32),
        ] {
            q.execute(&pool).await.expect("posting");
        }

        let engine = Engine::new(pool.clone());
        let rollup = engine.rollup_balances().await.expect("rollup");

        let assets_bal = rollup.get(&assets).expect("assets present");
        assert_eq!(assets_bal.get("AUD"), Some(dec!(105.00)));
        assert_eq!(assets_bal.get("USD"), Some(dec!(30.00)));
        assert_eq!(assets_bal.len(), 2);

        // `own` is a `BTreeMap` keyed by account id, so the fold visits
        // children in a fixed order and a multi-commodity parent's `iter()`
        // order is stable across calls, rather than varying with hash
        // iteration order as it would with a `HashMap`.
        let rollup_again = engine.rollup_balances().await.expect("second rollup");
        let assets_bal_again = rollup_again.get(&assets).expect("assets present again");
        assert_eq!(
            assets_bal.iter().collect::<Vec<_>>(),
            assets_bal_again.iter().collect::<Vec<_>>(),
            "a multi-commodity parent's roll-up order must be stable across calls"
        );

        let bank_bal = rollup.get(&bank).expect("bank present");
        assert_eq!(bank_bal.get("AUD"), Some(dec!(105.00)));
        assert_eq!(bank_bal.get("USD"), None);

        let savings_bal = rollup.get(&savings).expect("savings present");
        assert_eq!(savings_bal.get("AUD"), Some(dec!(100.00)));

        // Elided residual counts for the income leaf, in USD.
        let income_bal = rollup.get(&income).expect("income present");
        assert_eq!(income_bal.get("AUD"), Some(dec!(-1104.00)));
        assert_eq!(income_bal.get("USD"), Some(dec!(-30.00)));

        assert!(!rollup.contains_key(&old));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollup_balances_folds_past_an_archived_intermediate_account(pool: sqlx::SqlitePool) {
        // Assets (active) -> Bank (archived) -> Savings (active, has postings).
        // Bank sits between two active accounts but must not itself appear in
        // the result, and its archival must not sever the fold: Assets still
        // has to receive Savings' contribution through it.
        let acct_svc = crate::account::Service::new(pool.clone());
        let assets = acct_svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("assets");
        let bank = acct_svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("bank");
        let savings = acct_svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&bank)
            .call()
            .await
            .expect("savings");
        let income = acct_svc
            .create()
            .name("Income")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("income");

        sqlx::query("UPDATE accounts SET archived_at = '2024-06-30T00:00:00Z' WHERE id = ?")
            .bind(bank.to_string())
            .execute(&pool)
            .await
            .expect("archive bank");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES \
            ('tx_1', '2026-01-01', 'a', 'reconciled', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("txs");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES \
            ('p1', 'tx_1', ?, '100.00', 'AUD', 0), \
            ('p2', 'tx_1', ?, '-100.00', 'AUD', 1)",
        )
        .bind(savings.to_string())
        .bind(income.to_string())
        .execute(&pool)
        .await
        .expect("postings");

        let engine = Engine::new(pool.clone());
        let rollup = engine.rollup_balances().await.expect("rollup");

        let assets_bal = rollup.get(&assets).expect("assets present");
        assert_eq!(assets_bal.get("AUD"), Some(dec!(100.00)));

        let savings_bal = rollup.get(&savings).expect("savings present");
        assert_eq!(savings_bal.get("AUD"), Some(dec!(100.00)));

        assert!(!rollup.contains_key(&bank));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rollup_balances_returns_on_a_parent_id_cycle(pool: sqlx::SqlitePool) {
        // A corrupt `parent_id` cycle (A -> B -> A) should never hang the
        // ancestor walk; the `seen` guard must break out once an id repeats.
        let acct_svc = crate::account::Service::new(pool.clone());
        let a = acct_svc
            .create()
            .name("A")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("a");
        let b = acct_svc
            .create()
            .name("B")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&a)
            .call()
            .await
            .expect("b");
        sqlx::query("UPDATE accounts SET parent_id = ? WHERE id = ?")
            .bind(b.to_string())
            .bind(a.to_string())
            .execute(&pool)
            .await
            .expect("corrupt parent_id into a cycle");

        sqlx::query(
            "INSERT INTO transactions (id, date, description, reconciliation, created_at) VALUES \
            ('tx_1', '2026-01-01', 'a', 'reconciled', '2026-01-01T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .expect("tx");
        sqlx::query(
            "INSERT INTO postings (id, transaction_id, account_id, amount, commodity, position) VALUES \
            ('p1', 'tx_1', ?, '10.00', 'AUD', 0)",
        )
        .bind(a.to_string())
        .execute(&pool)
        .await
        .expect("posting");

        let engine = Engine::new(pool.clone());
        let rollup = engine
            .rollup_balances()
            .await
            .expect("rollup must return despite the parent_id cycle");

        assert_eq!(
            rollup.get(&a).and_then(|bal| bal.get("AUD")),
            Some(dec!(10.00))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn scope_ledger_runs_chronologically_with_residuals(pool: sqlx::SqlitePool) {
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let food = make_account(&pool, "Food", AccountType::Expense).await;
        let b = bank.to_string();
        let f = food.to_string();
        // Same date; tx_1 < tx_2 so display order (date DESC, id ASC) puts
        // tx_2 below tx_1, making tx_2 the earlier one chronologically.
        insert_tx(&pool, "tx_2", "2026-02-01").await;
        insert_posting(&pool, "p21", "tx_2", &b, Some("100.00"), Some("AUD"), 0).await;
        insert_posting(&pool, "p22", "tx_2", &f, Some("-100.00"), Some("AUD"), 1).await;
        insert_tx(&pool, "tx_1", "2026-02-01").await;
        insert_posting(&pool, "p11", "tx_1", &f, Some("30.00"), Some("AUD"), 0).await;
        insert_posting(&pool, "p12", "tx_1", &b, None, None, 1).await; // elided → -30
        insert_tx(&pool, "tx_c", "2026-01-15").await;
        insert_posting(&pool, "pc1", "tx_c", &b, Some("5.00"), Some("AUD"), 0).await;
        insert_posting(&pool, "pc2", "tx_c", &f, Some("-5.00"), Some("AUD"), 1).await;

        let ledger = Engine::new(pool.clone())
            .scope_ledger(&[bank])
            .await
            .expect("ledger");

        assert_eq!(ledger.balance_after("tx_c", "AUD"), dec!(5.00));
        assert_eq!(ledger.balance_after("tx_2", "AUD"), dec!(105.00));
        assert_eq!(ledger.balance_after("tx_1", "AUD"), dec!(75.00));
        assert_eq!(ledger.delta("tx_1", "AUD"), dec!(-30.00));
        assert_eq!(ledger.focal_commodity("tx_2"), Some("AUD"));
        assert_eq!(ledger.balance_after("missing", "AUD"), Decimal::ZERO);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn scope_ledger_focal_commodity_is_none_when_mixed(pool: sqlx::SqlitePool) {
        let broker = make_account(&pool, "Broker", AccountType::Asset).await;
        let bank = make_account(&pool, "Bank", AccountType::Asset).await;
        let k = broker.to_string();
        insert_tx(&pool, "tx_buy", "2026-03-01").await;
        insert_posting(&pool, "q1", "tx_buy", &k, Some("10"), Some("XYZ"), 0).await;
        insert_posting(&pool, "q2", "tx_buy", &k, Some("-500.00"), Some("AUD"), 1).await;
        insert_posting(
            &pool,
            "q3",
            "tx_buy",
            &bank.to_string(),
            Some("0.00"),
            Some("AUD"),
            2,
        )
        .await;

        let ledger = Engine::new(pool.clone())
            .scope_ledger(&[broker])
            .await
            .expect("ledger");

        assert_eq!(ledger.focal_commodity("tx_buy"), None);
        assert_eq!(ledger.balance_after("tx_buy", "XYZ"), dec!(10));
        assert_eq!(ledger.balance_after("tx_buy", "AUD"), dec!(-500.00));
    }

    #[rstest]
    #[case::plain_inflow(&[dec!(100)], dec!(100), dec!(0), dec!(0))]
    #[case::plain_outflow(&[dec!(-12.34)], dec!(0), dec!(12.34), dec!(0))]
    #[case::internal_transfer(&[dec!(-100), dec!(100)], dec!(0), dec!(0), dec!(100))]
    #[case::transfer_with_fee(&[dec!(-1005), dec!(1000)], dec!(0), dec!(5), dec!(1000))]
    #[case::pay_split(&[dec!(4000), dec!(1000)], dec!(5000), dec!(0), dec!(0))]
    #[case::same_leaf_both_directions(&[dec!(50), dec!(-30)], dec!(20), dec!(0), dec!(30))]
    #[case::empty(&[], dec!(0), dec!(0), dec!(0))]
    fn flow_totals_folds_one_transaction(
        #[case] legs: &[Decimal],
        #[case] inflow: Decimal,
        #[case] outflow: Decimal,
        #[case] internal: Decimal,
    ) {
        let mut totals = FlowTotals::default();
        totals.add_transaction(legs.iter().copied()).expect("fold");
        assert_eq!(
            totals,
            FlowTotals {
                inflow,
                outflow,
                internal
            }
        );
    }

    #[test]
    fn flow_totals_accumulates_across_transactions() {
        let mut totals = FlowTotals::default();
        totals.add_transaction([dec!(100)]).expect("first");
        totals.add_transaction([dec!(-30)]).expect("second");
        totals
            .add_transaction([dec!(-10), dec!(10)])
            .expect("third");
        assert_eq!(
            totals,
            FlowTotals {
                inflow: dec!(100),
                outflow: dec!(30),
                internal: dec!(10)
            }
        );
    }

    #[test]
    fn flow_totals_reports_overflow() {
        let mut totals = FlowTotals::default();
        let err = totals
            .add_transaction([Decimal::MAX, Decimal::MAX])
            .expect_err("overflow");
        assert!(matches!(err, BcError::BadData(_)), "got {err:?}");
    }
}
