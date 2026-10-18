//! Builds export snapshots from stored transactions.
//!
//! Account and tag ids become paths, and metadata splits into the entries the
//! export spells and the entries it omits. The archive level adds every
//! account, commodity and budget, and month-end balances.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;
use std::ops::Bound;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Budget;
use bc_models::BudgetRevision;
use bc_models::Commodity;
use bc_models::CommodityId;
use bc_models::DepreciationPolicy;
use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaValue;
use bc_models::Metadata;
use bc_models::Period;
use bc_models::Posting;
use bc_models::TagForest;
use bc_models::TagId;
use bc_models::TagPath;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::AccountResolver;
use crate::BcError;
use crate::BcResult;
use crate::RawMetaEntry;
use crate::export::AccountEntry;
use crate::export::BalanceEntry;
use crate::export::BudgetEntry;
use crate::export::CommodityEntry;
use crate::export::Level;
use crate::export::PostingEntry;
use crate::export::Snapshot;
use crate::export::TransactionEntry;
use crate::residual::Residual;
use crate::residual::residual_of_postings;

/// The metadata key that carries a handle.
const HANDLE_KEY: &str = "bc";

/// The prefix of every other key the round trip writes.
const RESERVED_PREFIX: &str = "bc-";

// MARK: Catalog

/// The ledger reference data a snapshot is rendered against.
#[derive(Debug)]
pub(super) struct Catalog {
    /// Every account, archived included.
    accounts: Vec<Account>,
    /// Account id to path.
    paths: AccountResolver,
    /// Every registered commodity.
    commodities: Vec<Commodity>,
    /// The tag hierarchy.
    tags: TagForest,
}

impl Catalog {
    /// Bundles the reference data.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Every account, archived included.
    /// * `paths` - A resolver over the same accounts.
    /// * `commodities` - Every registered commodity.
    /// * `tags` - The tag hierarchy.
    pub(super) const fn new(
        accounts: Vec<Account>,
        paths: AccountResolver,
        commodities: Vec<Commodity>,
        tags: TagForest,
    ) -> Self {
        Self {
            accounts,
            paths,
            commodities,
            tags,
        }
    }

    /// The path of `id`, or the id itself when the account is unknown.
    fn path(&self, id: &AccountId) -> String {
        account_path(&self.paths, id)
    }

    /// The paths of `ids` in order, skipping a tag the forest does not hold.
    fn tag_paths(&self, ids: &[TagId]) -> Vec<String> {
        ids.iter()
            .filter_map(|id| self.tags.path_of(id))
            .map(|path| tag_spelling(&path))
            .collect()
    }

    /// The code of the commodity `id` names.
    fn commodity_code(&self, id: &CommodityId) -> Option<&str> {
        self.commodities
            .iter()
            .find(|c| c.id() == id)
            .map(Commodity::code)
    }
}

/// Spells `path` for an export: its segments joined by `/`, which Beancount's
/// tag grammar accepts where it rejects `:`. Apply splits a stated tag on `/`.
///
/// A path with a segment holding `/` keeps its `:` form, which no `/`-split
/// can mistake for another path and which the Beancount writer reports as
/// unspellable.
fn tag_spelling(path: &TagPath) -> String {
    if path.segments().iter().any(|segment| segment.contains('/')) {
        path.to_string()
    } else {
        path.segments().join("/")
    }
}

/// The path of `id`, or the id itself when `paths` does not hold it.
fn account_path(paths: &AccountResolver, id: &AccountId) -> String {
    paths
        .path_of(id)
        .map_or_else(|| id.to_string(), ToOwned::to_owned)
}

// MARK: Metadata

/// Whether the round trip reserves `key`: `bc`, or a key starting `bc-`.
pub(super) fn is_reserved(key: &MetaKey) -> bool {
    let name = key.as_str();
    name == HANDLE_KEY || name.starts_with(RESERVED_PREFIX)
}

/// Splits `metadata` into the entries an export spells and the entries it
/// omits.
///
/// An entry is spelled when it is the first under its key, its value fits its
/// key's type, and the round trip does not reserve its key. Every other entry
/// is omitted: Beancount cannot repeat a key, and a mismatched value has no
/// typed spelling. Both lists keep display order. Apply restores the omitted
/// entries onto the transaction or posting they came from.
///
/// # Arguments
///
/// * `metadata` - The stored metadata.
/// * `accounts` - Renders an account-valued entry as its path.
///
/// # Returns
///
/// The spelled entries, with account values as paths, and the omitted
/// entries as stored.
#[must_use]
#[inline]
pub fn spellable_metadata(
    metadata: &Metadata,
    accounts: &AccountResolver,
) -> (Vec<RawMetaEntry>, Metadata) {
    let mut spelled = Vec::new();
    let mut omitted = Vec::new();
    for (entry, spells) in metadata.iter().zip(spelled_mask(metadata)) {
        if spells {
            spelled.push(raw_entry(entry, accounts));
        } else {
            omitted.push(entry.clone());
        }
    }
    (spelled, Metadata::new(omitted))
}

/// Marks, per entry of `metadata` in display order, whether
/// [`spellable_metadata`] spells it.
pub(super) fn spelled_mask(metadata: &Metadata) -> Vec<bool> {
    let mut seen: HashSet<&MetaKey> = HashSet::new();
    metadata
        .iter()
        .map(|entry| {
            let first = seen.insert(entry.key());
            first && !entry.mismatched() && !is_reserved(entry.key())
        })
        .collect()
}

/// Renders one stored entry for a plugin, naming an account by path.
fn raw_entry(entry: &MetaEntry, accounts: &AccountResolver) -> RawMetaEntry {
    match *entry.value() {
        MetaValue::Account(ref id) => {
            RawMetaEntry::account_path(entry.key().clone(), account_path(accounts, id))
        }
        MetaValue::Text(_)
        | MetaValue::Number(_)
        | MetaValue::Boolean(_)
        | MetaValue::Date(_)
        | MetaValue::Timestamp(_)
        | MetaValue::Amount(_) => {
            RawMetaEntry::resolved(entry.key().clone(), entry.value().clone())
        }
    }
}

/// Fails when a transaction or one of its postings carries a key the round
/// trip reserves.
///
/// # Errors
///
/// Returns [`BcError::InvalidInput`] naming every offending transaction and
/// key.
pub(super) fn refuse_reserved_keys(transactions: &[Transaction]) -> BcResult<()> {
    let offences: Vec<String> = transactions
        .iter()
        .flat_map(|tx| {
            let keys: BTreeSet<&str> = tx
                .metadata()
                .iter()
                .chain(tx.postings().iter().flat_map(|p| p.metadata().iter()))
                .map(MetaEntry::key)
                .filter(|key| is_reserved(key))
                .map(MetaKey::as_str)
                .collect();
            keys.into_iter()
                .map(move |key| format!("transaction {} uses `{key}`", tx.id()))
        })
        .collect();
    if offences.is_empty() {
        return Ok(());
    }
    Err(BcError::InvalidInput(format!(
        "metadata keys `{HANDLE_KEY}` and `{RESERVED_PREFIX}*` are reserved for export: {}",
        offences.join("; ")
    )))
}

// MARK: Levels

/// Builds a working-level snapshot: the transactions under their short
/// handles, and only the accounts and commodities they reference.
///
/// # Arguments
///
/// * `transactions` - The selected transactions.
/// * `handles` - Each transaction's short handle.
/// * `catalog` - The reference data.
/// * `narrowed` - Whether a window or query selected `transactions`.
///
/// # Errors
///
/// Returns [`BcError`] if a rendered field fails to encode.
pub(super) fn working(
    transactions: &[Transaction],
    handles: &HashMap<TransactionId, String>,
    catalog: &Catalog,
    narrowed: bool,
) -> BcResult<Snapshot> {
    let (account_ids, mut codes) = referenced(transactions);
    let accounts = account_entries(
        catalog
            .accounts
            .iter()
            .filter(|account| account_ids.contains(account.id())),
        catalog,
    )?;
    codes.extend(
        accounts
            .iter()
            .flat_map(|account| account.commodities.iter().cloned()),
    );
    let entries = transactions
        .iter()
        .map(|tx| {
            let handle = handles
                .get(tx.id())
                .cloned()
                .unwrap_or_else(|| tx.id().to_string());
            transaction_entry(tx, handle, Level::Working, catalog)
        })
        .collect();
    Ok(Snapshot {
        level: Level::Working,
        accounts,
        commodities: commodity_entries(&codes, catalog)?,
        transactions: entries,
        budgets: Vec::new(),
        balances: Vec::new(),
        narrowed,
    })
}

/// Builds an archive-level snapshot: every account, commodity and given
/// budget, the transactions under their full IDs, and month-end balances.
///
/// # Arguments
///
/// * `transactions` - The selected transactions.
/// * `budgets` - The budgets to export, each with its revisions.
/// * `catalog` - The reference data.
/// * `narrowed` - Whether a window or query selected `transactions`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if a balance overflows, and [`BcError`] if a
/// rendered field fails to encode.
pub(super) fn archive(
    transactions: &[Transaction],
    budgets: &[(Budget, Vec<BudgetRevision>)],
    catalog: &Catalog,
    narrowed: bool,
) -> BcResult<Snapshot> {
    let (_, mut codes) = referenced(transactions);
    codes.extend(
        catalog
            .commodities
            .iter()
            .map(|commodity| commodity.code().to_owned()),
    );
    let mut budget_entries = Vec::new();
    for (budget, revisions) in budgets {
        let account = catalog.path(budget.account_id());
        for revision in revisions {
            budget_entries.push(budget_entry(budget, revision, &account, catalog)?);
        }
    }
    Ok(Snapshot {
        level: Level::Archive,
        accounts: account_entries(catalog.accounts.iter(), catalog)?,
        commodities: commodity_entries(&codes, catalog)?,
        transactions: transactions
            .iter()
            .map(|tx| transaction_entry(tx, tx.id().to_string(), Level::Archive, catalog))
            .collect(),
        budgets: budget_entries,
        balances: month_end_balances(transactions, catalog)?,
        narrowed,
    })
}

/// The accounts and commodity codes `transactions` reference through their
/// legs and metadata.
fn referenced(transactions: &[Transaction]) -> (HashSet<&AccountId>, BTreeSet<String>) {
    let mut accounts = HashSet::new();
    let mut codes = BTreeSet::new();
    for tx in transactions {
        let legs_meta = tx.postings().iter().flat_map(|p| p.metadata().iter());
        for entry in tx.metadata().iter().chain(legs_meta) {
            match *entry.value() {
                MetaValue::Account(ref id) => {
                    accounts.insert(id);
                }
                MetaValue::Amount(ref amount) => {
                    codes.insert(amount.commodity().as_str().to_owned());
                }
                MetaValue::Text(_)
                | MetaValue::Number(_)
                | MetaValue::Boolean(_)
                | MetaValue::Date(_)
                | MetaValue::Timestamp(_) => {}
            }
        }
        for posting in tx.postings() {
            accounts.insert(posting.account_id());
            let quotes = posting
                .price()
                .into_iter()
                .chain(posting.cost().map(bc_models::Cost::basis));
            codes.extend(
                posting
                    .amount()
                    .into_iter()
                    .chain(quotes.map(bc_models::Quote::amount))
                    .map(|amount| amount.commodity().as_str().to_owned()),
            );
        }
    }
    (accounts, codes)
}

// MARK: Transactions

/// Renders one transaction. Archive-level postings carry their IDs as
/// handles; working-level postings carry none.
///
/// The entry is marked unbalanced at both levels; the plugin decides what to
/// do with the mark.
fn transaction_entry(
    tx: &Transaction,
    handle: String,
    level: Level,
    catalog: &Catalog,
) -> TransactionEntry {
    let postings = tx
        .postings()
        .iter()
        .map(|posting| PostingEntry {
            handle: (level == Level::Archive).then(|| posting.id().to_string()),
            account: catalog.path(posting.account_id()),
            amount: posting.amount().cloned(),
            price: posting.price().cloned(),
            cost: posting.cost().cloned(),
            tags: catalog.tag_paths(posting.tag_ids()),
            metadata: spellable_metadata(posting.metadata(), &catalog.paths).0,
            spread_from: posting.spread_from(),
            spread_until: posting.spread_until(),
        })
        .collect();
    TransactionEntry {
        handle,
        date: tx.date(),
        description: tx.description().to_owned(),
        reconciliation: tx.reconciliation(),
        tags: catalog.tag_paths(tx.tag_ids()),
        metadata: spellable_metadata(tx.metadata(), &catalog.paths).0,
        postings,
        unbalanced: is_unbalanced(tx),
    }
}

/// Reports whether `tx` has no elided leg and its weights sum to non-zero in
/// some commodity: the residual a single added elided leg would absorb.
fn is_unbalanced(tx: &Transaction) -> bool {
    tx.elided_count() == 0 && !tx.balanced()
}

// MARK: Accounts and commodities

/// Renders `accounts`, sorted by path.
fn account_entries<'a>(
    accounts: impl Iterator<Item = &'a Account>,
    catalog: &Catalog,
) -> BcResult<Vec<AccountEntry>> {
    let mut entries = accounts
        .map(|account| account_entry(account, catalog))
        .collect::<BcResult<Vec<_>>>()?;
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

/// Renders one account, with its non-path fields as `bc-` extras.
fn account_entry(account: &Account, catalog: &Catalog) -> BcResult<AccountEntry> {
    let mut extra = Extra::default();
    extra.text("account-type", variant_name(&account.account_type())?);
    extra.text("kind", variant_name(&account.kind())?);
    if let Some(description) = account.description() {
        extra.text("description", description);
    }
    let tags = catalog.tag_paths(account.tag_ids());
    if !tags.is_empty() {
        extra.text("tags", tags.join(" "));
    }
    if let Some(day) = account.acquisition_date() {
        extra.push("acquisition-date", MetaValue::Date(day));
    }
    if let Some(cost) = account.acquisition_cost() {
        extra.push("acquisition-cost", MetaValue::Number(cost));
    }
    if let Some(policy) = account.depreciation_policy() {
        extra.text("depreciation-policy", variant_name(policy)?);
        if let DepreciationPolicy::StraightLine { annual_rate }
        | DepreciationPolicy::DecliningBalance { annual_rate } = *policy
        {
            extra.push("annual-rate", MetaValue::Number(annual_rate));
        }
    }
    if let Some(at) = account.archived_at() {
        extra.push("archived-at", MetaValue::Timestamp(*at));
    }
    Ok(AccountEntry {
        path: catalog.path(account.id()),
        opened_on: account.opened_on(),
        closed_on: account.closed_on(),
        commodities: account
            .commodities()
            .iter()
            .filter_map(|id| catalog.commodity_code(id))
            .map(ToOwned::to_owned)
            .collect(),
        extra: extra.finish()?,
    })
}

/// Renders one entry per code, in code order. A code with no registered
/// commodity has no extras.
fn commodity_entries(codes: &BTreeSet<String>, catalog: &Catalog) -> BcResult<Vec<CommodityEntry>> {
    codes
        .iter()
        .map(|code| {
            let extra = catalog
                .commodities
                .iter()
                .find(|c| c.code() == code)
                .map_or_else(|| Ok(Vec::new()), commodity_extra)?;
            Ok(CommodityEntry {
                code: code.clone(),
                extra,
            })
        })
        .collect()
}

/// A commodity's details as `bc-` extras.
fn commodity_extra(commodity: &Commodity) -> BcResult<Vec<RawMetaEntry>> {
    let mut extra = Extra::default();
    let texts = [
        ("exchange", commodity.exchange()),
        ("name", commodity.name()),
        ("description", commodity.description()),
        ("symbol", commodity.symbol()),
    ];
    for (field, value) in texts {
        if let Some(text) = value {
            extra.text(field, text);
        }
    }
    if !commodity.aliases().is_empty() {
        extra.text("aliases", commodity.aliases().join(" "));
    }
    extra.push(
        "decimals",
        MetaValue::Number(Decimal::from(commodity.decimals())),
    );
    extra.push("is-iso", MetaValue::Boolean(commodity.is_iso()));
    extra.push("symbol-after", MetaValue::Boolean(commodity.symbol_after()));
    if let Some(day) = commodity.active_from() {
        extra.push("active-from", MetaValue::Date(day));
    }
    if let Some(day) = commodity.active_until() {
        extra.push("active-until", MetaValue::Date(day));
    }
    extra.finish()
}

// MARK: Budgets

/// Renders one budget revision.
fn budget_entry(
    budget: &Budget,
    revision: &BudgetRevision,
    account: &str,
    catalog: &Catalog,
) -> BcResult<BudgetEntry> {
    let mut extra = Extra::default();
    if let Some(name) = revision.name() {
        extra.text("name", name);
    }
    extra.text("rollover", variant_name(&revision.rollover())?);
    extra.text("intent", variant_name(&revision.intent())?);
    if let Some(path) = revision
        .tag_filter()
        .and_then(|tag| catalog.tags.path_of(tag))
    {
        extra.text("tag-filter", tag_spelling(&path));
    }
    let period = period_spelling(revision.period(), &mut extra)?;
    if let Some(at) = budget.archived_at() {
        extra.push("archived-at", MetaValue::Timestamp(*at));
    }
    Ok(BudgetEntry {
        account: account.to_owned(),
        effective_from: revision.effective_from(),
        period,
        target: revision.target().cloned(),
        expression: revision.target_expr().map(ToOwned::to_owned),
        extra: extra.finish()?,
    })
}

/// The period word for `period`, pushing its parameters onto `extra`.
///
/// A period Fava names exactly takes Fava's word. Any other takes the model's
/// variant name, with each parameter as a `bc-` extra.
fn period_spelling(period: &Period, extra: &mut Extra) -> BcResult<String> {
    let word = match *period {
        Period::Daily => "daily",
        Period::Weekly => "weekly",
        Period::Monthly => "monthly",
        Period::Quarterly => "quarterly",
        Period::CalendarYear => "yearly",
        Period::Fortnightly { anchor } => {
            extra.push("anchor", MetaValue::Date(anchor));
            "fortnightly"
        }
        Period::FinancialYear {
            start_month,
            start_day,
        }
        | Period::FinancialQuarter {
            start_month,
            start_day,
        } => {
            extra.push("start-month", MetaValue::Number(start_month.into()));
            extra.push("start-day", MetaValue::Number(start_day.into()));
            return variant_name(period);
        }
        Period::Custom {
            days,
            weeks,
            months,
        } => {
            for (field, count) in [("days", days), ("weeks", weeks), ("months", months)] {
                if let Some(n) = count {
                    extra.push(field, MetaValue::Number(n.into()));
                }
            }
            "custom"
        }
        ref other => return variant_name(other),
    };
    Ok(word.to_owned())
}

// MARK: Balances

/// Per-commodity deltas keyed by month end.
type MonthDeltas = BTreeMap<Date, BTreeMap<String, Decimal>>;

/// A leg and the per-commodity units it moves.
type LegUnits<'a> = (&'a Posting, Vec<(String, Decimal)>);

/// Each account's balance at the end of every month it has a leg in.
///
/// A balance covers the account and its descendants, as a Beancount `balance`
/// assertion does. An elided leg counts as its transaction's residual. Every
/// commodity the subtree has held is stated, zero included. A month whose
/// balance would be written after the account's close states none.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if a weight or a running total overflows.
fn month_end_balances(
    transactions: &[Transaction],
    catalog: &Catalog,
) -> BcResult<Vec<BalanceEntry>> {
    let mut own: BTreeMap<String, MonthDeltas> = BTreeMap::new();
    for tx in transactions {
        let month_end = tx.date().last_of_month();
        for (posting, units) in leg_units(tx)? {
            let deltas = own
                .entry(catalog.path(posting.account_id()))
                .or_default()
                .entry(month_end)
                .or_default();
            for (code, value) in units {
                add(deltas.entry(code).or_insert(Decimal::ZERO), value)?;
            }
        }
    }

    let closes: HashMap<String, Date> = catalog
        .accounts
        .iter()
        .filter_map(|account| Some((catalog.path(account.id()), account.closed_on()?)))
        .collect();
    let mut balances = Vec::new();
    for (path, active) in &own {
        let child_prefix = format!("{path}:");
        let mut subtree = MonthDeltas::new();
        let family = own
            .range::<str, _>((Bound::Included(path.as_str()), Bound::Unbounded))
            .take_while(|&(other, _)| other.starts_with(path.as_str()))
            .filter(|&(other, _)| other == path || other.starts_with(&child_prefix));
        for (_, months) in family {
            for (month_end, deltas) in months {
                let merged = subtree.entry(*month_end).or_default();
                for (code, value) in deltas {
                    add(merged.entry(code.clone()).or_insert(Decimal::ZERO), *value)?;
                }
            }
        }
        let mut running: BTreeMap<String, Decimal> = BTreeMap::new();
        for (month_end, deltas) in &subtree {
            for (code, value) in deltas {
                add(running.entry(code.clone()).or_insert(Decimal::ZERO), *value)?;
            }
            if active.contains_key(month_end)
                && !closed_before_written(*month_end, closes.get(path))
            {
                balances.extend(running.iter().map(|(code, value)| BalanceEntry {
                    date: *month_end,
                    account: path.clone(),
                    amount: Amount::new(*value, code.as_str()),
                }));
            }
        }
    }
    balances.sort_by(|a, b| {
        (a.date, &a.account, a.amount.commodity().as_str()).cmp(&(
            b.date,
            &b.account,
            b.amount.commodity().as_str(),
        ))
    });
    Ok(balances)
}

/// Whether an account closed on `closed_on` is already closed on the day a
/// balance stated at `month_end` is written: the writer asserts it on the next
/// day, and Beancount refuses an assertion on an account after its close.
fn closed_before_written(month_end: Date, closed_on: Option<&Date>) -> bool {
    closed_on.is_some_and(|closed| month_end.tomorrow().is_ok_and(|written| written > *closed))
}

/// Each leg of `tx` with the units it moves: its amount, or for an elided leg
/// the transaction's residual.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if a weight overflows.
fn leg_units(tx: &Transaction) -> BcResult<Vec<LegUnits<'_>>> {
    let residual = residual_of_postings(tx.postings())
        .map_err(|e| BcError::BadData(format!("transaction {}: {e}", tx.id())))?;
    Ok(tx
        .postings()
        .iter()
        .map(|posting| {
            let units = match posting.amount() {
                Some(amount) => vec![(amount.commodity().as_str().to_owned(), amount.value())],
                None => match residual {
                    Residual::Attributable(ref balances) => balances
                        .iter()
                        .map(|(code, value)| (code.to_owned(), value))
                        .collect(),
                    Residual::NotElided | Residual::Ambiguous => Vec::new(),
                },
            };
            (posting, units)
        })
        .collect())
}

/// Adds `delta` to `total`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] on overflow.
fn add(total: &mut Decimal, delta: Decimal) -> BcResult<()> {
    *total = total
        .checked_add(delta)
        .ok_or_else(|| BcError::BadData("a balance exceeds the decimal range".to_owned()))?;
    Ok(())
}

// MARK: Extras

/// Host-rendered `bc-` fields, keyed by model field name in kebab case.
#[derive(Debug, Default)]
struct Extra(Vec<(&'static str, MetaValue)>);

impl Extra {
    /// Appends `value` under `bc-<field>`.
    fn push(&mut self, field: &'static str, value: MetaValue) {
        self.0.push((field, value));
    }

    /// Appends text under `bc-<field>`.
    fn text(&mut self, field: &'static str, value: impl Into<String>) {
        self.push(field, MetaValue::Text(value.into()));
    }

    /// The fields as metadata entries, in push order.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if a field name makes an invalid key.
    fn finish(self) -> BcResult<Vec<RawMetaEntry>> {
        self.0
            .into_iter()
            .map(|(field, value)| {
                let key = MetaKey::new(format!("{RESERVED_PREFIX}{field}"))
                    .map_err(|e| BcError::BadData(format!("export field `{field}`: {e}")))?;
                Ok(RawMetaEntry::resolved(key, value))
            })
            .collect()
    }
}

/// The kebab-case name of an enum variant, read from its serde form: a unit
/// variant's string, or an internally tagged variant's `type`.
///
/// # Errors
///
/// Returns [`BcError::Serialisation`] if `value` does not serialise, and
/// [`BcError::BadData`] if its form carries no name.
fn variant_name<T>(value: &T) -> BcResult<String>
where
    T: serde::Serialize,
{
    let form = serde_json::to_value(value)?;
    let name = form
        .as_str()
        .or_else(|| form.get("type").and_then(serde_json::Value::as_str))
        .ok_or_else(|| BcError::BadData(format!("no variant name in {form}")))?;
    Ok(name.replace('_', "-"))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashSet;

    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::BudgetIntent;
    use bc_models::MetaEntry;
    use bc_models::MetaKey;
    use bc_models::MetaValue;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::RolloverPolicy;
    use bc_models::TagPath;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    use super::*;
    use crate::AccountPath;
    use crate::AccountService;
    use crate::BudgetService;
    use crate::Cascade;
    use crate::CommodityService;
    use crate::ExportBalanceEntry;
    use crate::ExportBudgetEntry;
    use crate::ExportLevel;
    use crate::PathSpec;
    use crate::RawMetaValue;
    use crate::TagService;
    use crate::TransactionService;
    use crate::roundtrip::RoundTripService;
    use crate::roundtrip::abbreviate;
    use crate::roundtrip::database_id;
    use crate::search::TransactionQuery;

    // MARK: Fixtures

    /// Creates the account at `path`, with any missing ancestors.
    async fn account(pool: &SqlitePool, path: &str) -> AccountId {
        AccountService::new(pool.clone())
            .create_path(
                &PathSpec::builder()
                    .path(AccountPath::parse(path).expect("path"))
                    .build(),
            )
            .await
            .expect("account")
    }

    fn key(name: &str) -> MetaKey {
        MetaKey::new(name).expect("valid key")
    }

    fn text(name: &str, value: &str) -> MetaEntry {
        MetaEntry::new(key(name), MetaValue::Text(value.to_owned()))
    }

    fn aud(value: Decimal) -> Amount {
        Amount::new(value, "AUD")
    }

    fn leg(account: &AccountId, amount: Option<Amount>) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(account.clone())
            .maybe_amount(amount)
            .build()
    }

    fn tx(
        day: Date,
        description: &str,
        postings: Vec<Posting>,
        metadata: Vec<MetaEntry>,
    ) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(day)
            .description(description)
            .postings(postings)
            .metadata(Metadata::new(metadata))
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    async fn store(pool: &SqlitePool, transaction: Transaction) -> TransactionId {
        TransactionService::new(pool.clone())
            .create(transaction)
            .await
            .expect("create")
            .value
    }

    async fn export(pool: &SqlitePool, level: ExportLevel) -> BcResult<crate::roundtrip::Export> {
        RoundTripService::new(pool.clone())
            .export(&TransactionQuery::default(), level)
            .await
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_window_marks_the_snapshot_narrowed(pool: SqlitePool) {
        let cases = [
            (TransactionQuery::default(), false),
            (
                TransactionQuery::new(None, Some(date(2026, 1, 1)), None),
                true,
            ),
            (
                TransactionQuery::new(None, None, Some(date(2026, 2, 1))),
                true,
            ),
        ];
        for (query, narrowed) in cases {
            for level in [ExportLevel::Working, ExportLevel::Archive] {
                let out = RoundTripService::new(pool.clone())
                    .export(&query, level)
                    .await
                    .expect("export");
                assert_eq!(out.snapshot.narrowed, narrowed, "{query:?} at {level:?}");
            }
        }
    }

    // MARK: Working level

    #[sqlx::test(migrations = "./migrations")]
    async fn working_snapshot_has_only_referenced_accounts(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let food = account(&pool, "Expenses:Food").await;
        account(&pool, "Expenses:Travel").await;
        store(
            &pool,
            tx(
                date(2026, 1, 5),
                "Woolworths",
                vec![leg(&food, Some(aud(dec!(12.50)))), leg(&checking, None)],
                vec![],
            ),
        )
        .await;

        let out = export(&pool, ExportLevel::Working).await.expect("export");

        let paths: Vec<&str> = out
            .snapshot
            .accounts
            .iter()
            .map(|a| a.path.as_str())
            .collect();
        assert_eq!(paths, vec!["Assets:Bank:Checking", "Expenses:Food"]);
        let codes: Vec<&str> = out
            .snapshot
            .commodities
            .iter()
            .map(|c| c.code.as_str())
            .collect();
        assert_eq!(codes, vec!["AUD"]);
        assert_eq!(out.snapshot.level, ExportLevel::Working);
        assert_eq!(out.snapshot.budgets, vec![]);
        assert_eq!(out.snapshot.balances, vec![]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn working_handles_match_sidecar(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let food = account(&pool, "Expenses:Food").await;
        let mut ids = Vec::new();
        for (day, value) in [(5, dec!(12.50)), (6, dec!(7.25))] {
            ids.push(
                store(
                    &pool,
                    tx(
                        date(2026, 1, day),
                        "Coles",
                        vec![leg(&food, Some(aud(value))), leg(&checking, None)],
                        vec![],
                    ),
                )
                .await,
            );
        }

        let out = export(&pool, ExportLevel::Working).await.expect("export");
        let sidecar = out.sidecar.expect("a working export has a sidecar");

        let expected = abbreviate(&ids);
        assert_eq!(sidecar.entries.len(), 2);
        assert_eq!(out.snapshot.transactions.len(), 2);
        for entry in &out.snapshot.transactions {
            let recorded = sidecar.entry(&entry.handle).expect("handle in sidecar");
            assert_eq!(expected.get(recorded.transaction.id()), Some(&entry.handle));
            assert!(entry.postings.iter().all(|p| p.handle.is_none()));
        }
        assert_eq!(sidecar.database_id, database_id(&pool).await.expect("id"));
    }

    // MARK: Unbalanced

    #[rstest]
    #[case::unbalanced_pair(Some(dec!(-7.00)), true)]
    #[case::elided_second_leg(None, false)]
    #[case::zero_sum_pair(Some(dec!(-10.00)), false)]
    fn unbalanced_marks_only_a_non_zero_residual_with_no_elided_leg(
        #[case] second: Option<Decimal>,
        #[case] expected: bool,
    ) {
        let bank = AccountId::new();
        let food = AccountId::new();
        let transaction = tx(
            date(2026, 1, 5),
            "Coles",
            vec![
                leg(&bank, Some(aud(dec!(10.00)))),
                leg(&food, second.map(aud)),
            ],
            vec![],
        );

        assert_eq!(is_unbalanced(&transaction), expected);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_single_leg_transaction_is_marked_at_both_levels(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let food = account(&pool, "Expenses:Food").await;
        store(
            &pool,
            tx(
                date(2026, 1, 5),
                "Statement line",
                vec![leg(&checking, Some(aud(dec!(-12.34))))],
                vec![],
            ),
        )
        .await;
        store(
            &pool,
            tx(
                date(2026, 1, 6),
                "Woolworths",
                vec![leg(&food, Some(aud(dec!(12.50)))), leg(&checking, None)],
                vec![],
            ),
        )
        .await;

        for level in [ExportLevel::Working, ExportLevel::Archive] {
            let out = export(&pool, level).await.expect("export");

            let mut marks: Vec<(&str, bool)> = out
                .snapshot
                .transactions
                .iter()
                .map(|t| (t.description.as_str(), t.unbalanced))
                .collect();
            marks.sort_unstable();
            assert_eq!(
                marks,
                vec![("Statement line", true), ("Woolworths", false)],
                "{level:?}"
            );
        }
    }

    // MARK: Tags

    #[rstest]
    #[case::flat(&["trip"], "trip")]
    #[case::nested(&["home", "garden"], "home/garden")]
    #[case::segment_with_slash(&["kids", "a/b"], "kids:a/b")]
    fn tag_paths_join_segments_with_a_slash(#[case] segments: &[&str], #[case] spelled: &str) {
        let path = TagPath::new(segments.iter().copied()).expect("valid path");
        assert_eq!(tag_spelling(&path), spelled);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn nested_tags_reach_the_snapshot_slash_separated(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let food = account(&pool, "Expenses:Food").await;
        let tags = TagService::new(pool.clone());
        let garden = tags
            .create_path(&"home:garden".parse().expect("path"))
            .await
            .expect("tag");
        let tagged = Posting::builder()
            .id(PostingId::new())
            .account_id(food)
            .amount(aud(dec!(12.50)))
            .tag_ids(vec![garden.clone()])
            .build();
        let transaction = Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 1, 5))
            .description("Bunnings")
            .postings(vec![tagged, leg(&checking, None)])
            .reconciliation(Reconciliation::Unreconciled)
            .tag_ids(vec![garden])
            .created_at(Timestamp::now())
            .build();
        store(&pool, transaction).await;

        let out = export(&pool, ExportLevel::Working).await.expect("export");
        let entry = out.snapshot.transactions.first().expect("one transaction");
        assert_eq!(entry.tags, vec!["home/garden".to_owned()]);
        assert_eq!(
            entry.postings.first().map(|p| p.tags.clone()),
            Some(vec!["home/garden".to_owned()])
        );
    }

    // MARK: Metadata

    #[sqlx::test(migrations = "./migrations")]
    async fn repeated_and_mismatched_metadata_are_omitted(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let paths = AccountResolver::load(&AccountService::new(pool.clone()))
            .await
            .expect("resolver");
        let metadata = Metadata::new(vec![
            text("payee", "Woolworths"),
            text("note", "first"),
            MetaEntry::mismatch(key("invoice"), "INV-123"),
            text("note", "second"),
            text("bc", "x1"),
            text("bc-delete", "yes"),
            text("bcc", "kept"),
            MetaEntry::new(key("owner"), MetaValue::Account(checking)),
        ]);

        let (spelled, omitted) = spellable_metadata(&metadata, &paths);

        assert_eq!(
            spelled,
            vec![
                RawMetaEntry::resolved(key("payee"), MetaValue::Text("Woolworths".to_owned())),
                RawMetaEntry::resolved(key("note"), MetaValue::Text("first".to_owned())),
                RawMetaEntry::resolved(key("bcc"), MetaValue::Text("kept".to_owned())),
                RawMetaEntry {
                    key: key("owner"),
                    value: RawMetaValue::AccountPath("Assets:Bank:Checking".to_owned()),
                },
            ]
        );
        assert_eq!(
            omitted,
            Metadata::new(vec![
                MetaEntry::mismatch(key("invoice"), "INV-123"),
                text("note", "second"),
                text("bc", "x1"),
                text("bc-delete", "yes"),
            ])
        );
    }

    #[rstest]
    #[case::bare_on_transaction("bc", false, true)]
    #[case::prefixed_on_transaction("bc-delete", false, true)]
    #[case::prefixed_on_posting("bc-tags", true, true)]
    #[case::ordinary("bcc", true, false)]
    fn user_bc_key_is_refused(#[case] name: &str, #[case] on_posting: bool, #[case] refused: bool) {
        let account = AccountId::new();
        let posting_meta = if on_posting {
            vec![text(name, "value")]
        } else {
            vec![]
        };
        let tx_meta = if on_posting {
            vec![]
        } else {
            vec![text(name, "value")]
        };
        let posting = Posting::builder()
            .id(PostingId::new())
            .account_id(account)
            .amount(aud(dec!(3.00)))
            .metadata(Metadata::new(posting_meta))
            .build();
        let transaction = tx(date(2026, 1, 5), "Bakers Delight", vec![posting], tx_meta);

        let result = refuse_reserved_keys(std::slice::from_ref(&transaction));

        if refused {
            let Err(BcError::InvalidInput(message)) = result else {
                panic!("expected InvalidInput, got {result:?}");
            };
            assert!(message.contains(&transaction.id().to_string()), "{message}");
            assert!(message.contains(&format!("`{name}`")), "{message}");
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn export_refuses_a_user_bc_key(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let food = account(&pool, "Expenses:Food").await;
        store(
            &pool,
            tx(
                date(2026, 1, 5),
                "Bakers Delight",
                vec![leg(&food, Some(aud(dec!(3.00)))), leg(&checking, None)],
                vec![text("bc", "abc")],
            ),
        )
        .await;

        let result = export(&pool, ExportLevel::Working).await;

        let Err(BcError::InvalidInput(message)) = result else {
            panic!("expected InvalidInput, got {result:?}");
        };
        assert!(message.contains("uses `bc`"), "{message}");
    }

    // MARK: Archive level

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_balances_are_month_end_and_decimal_exact(pool: SqlitePool) {
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let food = account(&pool, "Expenses:Food").await;
        account(&pool, "Expenses:Travel").await;
        for (day, value) in [
            (date(2026, 1, 5), dec!(0.10)),
            (date(2026, 1, 20), dec!(0.20)),
            (date(2026, 2, 3), dec!(1.00)),
        ] {
            store(
                &pool,
                tx(
                    day,
                    "IGA",
                    vec![leg(&food, Some(aud(value))), leg(&checking, None)],
                    vec![],
                ),
            )
            .await;
        }

        let out = export(&pool, ExportLevel::Archive).await.expect("export");

        let balance = |day: Date, path: &str, value: Decimal| ExportBalanceEntry {
            date: day,
            account: path.to_owned(),
            amount: aud(value),
        };
        assert_eq!(
            out.snapshot.balances,
            vec![
                balance(date(2026, 1, 31), "Assets:Bank:Checking", dec!(-0.30)),
                balance(date(2026, 1, 31), "Expenses:Food", dec!(0.30)),
                balance(date(2026, 2, 28), "Assets:Bank:Checking", dec!(-1.30)),
                balance(date(2026, 2, 28), "Expenses:Food", dec!(1.30)),
            ]
        );
        let food_january = out
            .snapshot
            .balances
            .get(1)
            .expect("a January food balance");
        assert_eq!(food_january.amount.value().to_string(), "0.30");
        assert!(out.sidecar.is_none());
        let paths: HashSet<&str> = out
            .snapshot
            .accounts
            .iter()
            .map(|a| a.path.as_str())
            .collect();
        assert!(paths.contains("Expenses:Travel"));
        assert!(paths.contains("Assets"));
        for entry in &out.snapshot.transactions {
            assert!(entry.handle.starts_with("transaction_"));
            assert!(entry.postings.iter().all(|p| {
                p.handle
                    .as_deref()
                    .is_some_and(|h| h.starts_with("posting_"))
            }));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_balance_includes_descendant_accounts(pool: SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let wallet = accounts
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("wallet");
        let coins = accounts
            .create()
            .name("Coins")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&wallet)
            .call()
            .await
            .expect("coins");
        let food = account(&pool, "Expenses:Food").await;
        store(
            &pool,
            tx(
                date(2026, 3, 2),
                "Bakery",
                vec![
                    leg(&food, Some(aud(dec!(5.00)))),
                    leg(&wallet, Some(aud(dec!(-3.00)))),
                    leg(&coins, Some(aud(dec!(-2.00)))),
                ],
                vec![],
            ),
        )
        .await;

        let out = export(&pool, ExportLevel::Archive).await.expect("export");

        let wallet_balance: Vec<&Amount> = out
            .snapshot
            .balances
            .iter()
            .filter(|b| b.account == "Wallet")
            .map(|b| &b.amount)
            .collect();
        assert_eq!(wallet_balance, vec![&aud(dec!(-5.00))]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_states_no_balance_after_an_account_closes(pool: SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let checking = account(&pool, "Assets:Bank:Checking").await;
        let mid_month = account(&pool, "Assets:Bank:Mid").await;
        let month_end = account(&pool, "Assets:Bank:End").await;
        for closing in [&mid_month, &month_end] {
            store(
                &pool,
                tx(
                    date(2026, 1, 5),
                    "Transfer",
                    vec![leg(closing, Some(aud(dec!(4.00)))), leg(&checking, None)],
                    vec![],
                ),
            )
            .await;
            store(
                &pool,
                tx(
                    date(2026, 1, 10),
                    "Transfer",
                    vec![leg(closing, Some(aud(dec!(-4.00)))), leg(&checking, None)],
                    vec![],
                ),
            )
            .await;
        }
        accounts
            .close(&mid_month, date(2026, 1, 15), Cascade::Reject)
            .await
            .expect("close mid-month");
        accounts
            .close(&month_end, date(2026, 1, 31), Cascade::Reject)
            .await
            .expect("close at month end");

        let out = export(&pool, ExportLevel::Archive).await.expect("export");

        let stated: Vec<&str> = out
            .snapshot
            .balances
            .iter()
            .map(|b| b.account.as_str())
            .collect();
        assert_eq!(stated, vec!["Assets:Bank:Checking"]);
    }

    #[rstest]
    #[case::open(None, false)]
    #[case::closed_mid_month(Some(date(2026, 1, 15)), true)]
    #[case::closed_at_month_end(Some(date(2026, 1, 31)), true)]
    #[case::closed_on_the_written_day(Some(date(2026, 2, 1)), false)]
    fn balance_is_dropped_when_written_after_the_close(
        #[case] closed_on: Option<Date>,
        #[case] dropped: bool,
    ) {
        assert_eq!(
            closed_before_written(date(2026, 1, 31), closed_on.as_ref()),
            dropped
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_exports_an_archived_budget(pool: SqlitePool) {
        let food = account(&pool, "Expenses:Food").await;
        let budgets = BudgetService::new(pool.clone());
        let (budget, _) = budgets
            .create()
            .account_id(food)
            .effective_from(date(2026, 1, 1))
            .target(aud(dec!(500)))
            .intent(BudgetIntent::Limit)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("budget")
            .value;
        budgets.archive(budget.id()).await.expect("archive");
        let archived_at = *budgets
            .list_all()
            .await
            .expect("list")
            .first()
            .and_then(Budget::archived_at)
            .expect("archived");

        let out = export(&pool, ExportLevel::Archive).await.expect("export");

        let entry = out.snapshot.budgets.first().expect("the archived budget");
        assert_eq!(out.snapshot.budgets.len(), 1);
        assert_eq!(
            entry.extra.last(),
            Some(&RawMetaEntry::resolved(
                key("bc-archived-at"),
                MetaValue::Timestamp(archived_at)
            ))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_budget_skips_a_tag_filter_missing_from_the_forest(pool: SqlitePool) {
        let food = account(&pool, "Expenses:Food").await;
        let budget = Budget::builder()
            .account_id(food.clone())
            .created_at(Timestamp::now())
            .build();
        let revision = BudgetRevision::builder()
            .budget_id(budget.id().clone())
            .effective_from(date(2026, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .tag_filter(TagId::new())
            .created_at(Timestamp::now())
            .build();
        let catalog = Catalog::new(
            Vec::new(),
            AccountResolver::load(&AccountService::new(pool.clone()))
                .await
                .expect("resolver"),
            Vec::new(),
            TagForest::new(Vec::new()),
        );

        let entry = budget_entry(&budget, &revision, "Expenses:Food", &catalog).expect("entry");

        assert!(
            entry
                .extra
                .iter()
                .all(|e| e.key.as_str() != "bc-tag-filter"),
            "{:?}",
            entry.extra
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_accounts_and_commodities_carry_model_fields(pool: SqlitePool) {
        let aud_commodity = CommodityService::new(pool.clone())
            .create(
                &Commodity::builder()
                    .code("AUD")
                    .name("Australian dollar")
                    .decimals(2)
                    .build(),
            )
            .await
            .expect("commodity");
        AccountService::new(pool.clone())
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .description("Account 123456789")
            .commodity_ids(core::slice::from_ref(aud_commodity.id()))
            .call()
            .await
            .expect("account");

        let out = export(&pool, ExportLevel::Archive).await.expect("export");

        let text_entry = |name: &str, value: &str| {
            RawMetaEntry::resolved(key(name), MetaValue::Text(value.to_owned()))
        };
        let savings = out.snapshot.accounts.first().expect("one account");
        assert_eq!(savings.path, "Savings");
        assert_eq!(savings.commodities, vec!["AUD".to_owned()]);
        assert_eq!(
            savings.extra,
            vec![
                text_entry("bc-account-type", "asset"),
                text_entry("bc-kind", "deposit-account"),
                text_entry("bc-description", "Account 123456789"),
            ]
        );
        assert_eq!(
            out.snapshot
                .commodities
                .first()
                .expect("one commodity")
                .extra,
            vec![
                text_entry("bc-name", "Australian dollar"),
                RawMetaEntry::resolved(key("bc-decimals"), MetaValue::Number(dec!(2))),
                RawMetaEntry::resolved(key("bc-is-iso"), MetaValue::Boolean(true)),
                RawMetaEntry::resolved(key("bc-symbol-after"), MetaValue::Boolean(false)),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_budget_keeps_expression(pool: SqlitePool) {
        let food = account(&pool, "Expenses:Food").await;
        BudgetService::new(pool.clone())
            .create()
            .account_id(food)
            .effective_from(date(2026, 1, 1))
            .target(aud(dec!(1)))
            .target_expr("6 * 100")
            .intent(BudgetIntent::Limit)
            .period(Period::Monthly)
            .rollover(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("budget");

        let out = export(&pool, ExportLevel::Archive).await.expect("export");

        assert_eq!(
            out.snapshot.budgets,
            vec![ExportBudgetEntry {
                account: "Expenses:Food".to_owned(),
                effective_from: date(2026, 1, 1),
                period: "monthly".to_owned(),
                target: Some(aud(dec!(600))),
                expression: Some("6 * 100".to_owned()),
                extra: vec![
                    RawMetaEntry::resolved(
                        key("bc-rollover"),
                        MetaValue::Text("carry-forward".to_owned())
                    ),
                    RawMetaEntry::resolved(key("bc-intent"), MetaValue::Text("limit".to_owned())),
                ],
            }]
        );
    }

    #[rstest]
    #[case::monthly(Period::Monthly, "monthly", vec![])]
    #[case::calendar_year(Period::CalendarYear, "yearly", vec![])]
    #[case::fortnightly(
        Period::Fortnightly { anchor: date(2026, 1, 2) },
        "fortnightly",
        vec![("bc-anchor", MetaValue::Date(date(2026, 1, 2)))]
    )]
    #[case::financial_year(
        Period::FinancialYear { start_month: 7, start_day: 1 },
        "financial-year",
        vec![
            ("bc-start-month", MetaValue::Number(dec!(7))),
            ("bc-start-day", MetaValue::Number(dec!(1))),
        ]
    )]
    #[case::custom(
        Period::Custom { days: None, weeks: Some(3), months: None },
        "custom",
        vec![("bc-weeks", MetaValue::Number(dec!(3)))]
    )]
    fn period_takes_favas_word_or_the_variant_name(
        #[case] period: Period,
        #[case] word: &str,
        #[case] params: Vec<(&str, MetaValue)>,
    ) {
        let mut extra = Extra::default();

        let spelled = period_spelling(&period, &mut extra).expect("spelled");

        assert_eq!(spelled, word);
        let expected: Vec<RawMetaEntry> = params
            .into_iter()
            .map(|(name, value)| RawMetaEntry::resolved(key(name), value))
            .collect();
        assert_eq!(extra.finish().expect("keys"), expected);
    }
}
