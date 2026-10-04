//! Budget tree assembly: builds the budget tree for one display window from
//! the active budgets and the partition of their postings.
//!
//! Rows follow the account tree from each type root down to every budgeted
//! account. Envelopes gain an `↳ unallocated` row for the postings they own,
//! and accounts under `Income` and `Expense` roots gain an `↳ unbudgeted` row
//! for postings no budget matches. A row's children sum to the row, except
//! where a posting counts in two budgets neither of whose rows nests the
//! other (`double_counted`).

use std::collections::BTreeMap;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::BudgetIntent;
use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::Period;
use bc_models::Verdict;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::budget::BudgetService;
use crate::budget::BudgetStatusEngine;
use crate::budget::PostingKey;
use crate::budget::ValuedPosting;
use crate::budget_partition::Owner;
use crate::budget_partition::Scope;

/// Label of the row holding the postings an envelope owns itself.
const UNALLOCATED_LABEL: &str = "↳ unallocated";

/// Label of the row holding the postings no budget matches.
const UNBUDGETED_LABEL: &str = "↳ unbudgeted";

// MARK: RowKind

/// What a tree row stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub enum RowKind {
    /// A budget, merged with its account's row when it is that row's only
    /// budget and unfiltered.
    Budget,
    /// An account without a budget of its own, aggregating the rows beneath.
    Account,
    /// The postings an envelope matches and none of its sub-budgets do.
    Unallocated,
    /// The postings under an account that no budget matches.
    Unbudgeted,
}

impl RowKind {
    /// Sort rank among siblings: leftover rows last, unallocated before unbudgeted.
    const fn rank(self) -> u8 {
        match self {
            Self::Budget | Self::Account => 0,
            Self::Unallocated => 1,
            Self::Unbudgeted => 2,
        }
    }
}

// MARK: BudgetTreeItem

/// One row of the budget tree for one display window.
#[derive(Debug, Clone)]
#[non_exhaustive]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent row flags, each shown as its own pill"
)]
pub struct BudgetTreeItem {
    /// Stable row identity: the budget id, or `acct:{account_id}`,
    /// `unalloc:{budget_id}` or `unbud:{account_id}`.
    pub id: String,
    /// What the row stands for.
    pub kind: RowKind,
    /// The account the row is anchored to.
    pub account: bc_models::Account,
    /// The row's budget; for an unallocated row, its envelope's budget.
    pub budget: Option<bc_models::Budget>,
    /// The revision governing the display window start, for budget rows.
    pub governing: Option<bc_models::BudgetRevision>,
    /// Row label. A budget row shows its revision name, else its account
    /// path relative to its parent row's account followed by `#` and its
    /// tag path relative to the parent's tag (each part omitted when it adds
    /// nothing; a lone tag shows without `#`). An account row shows the
    /// account's leaf name; leftover rows show `↳ unallocated` or
    /// `↳ unbudgeted`.
    pub label: String,
    /// ID and colon-joined path of a filtered budget's tag. The pair comes from
    /// the governing revision, else the first overlapping one, else the first,
    /// so it can be set while `governing` is `None`.
    pub tag_filter: Option<(bc_models::TagId, String)>,
    /// Row total, in one commodity. `None` when the rows beneath span
    /// commodities (`mixed`) or no commodity is known.
    pub actual: Option<Amount>,
    /// Window-effective target. `None` for tracking-only budgets, unbudgeted
    /// rows and account rows whose budgets disagree.
    pub target: Option<Amount>,
    /// The budget's intent; an unallocated row takes its envelope's. `None`
    /// for account and unbudgeted rows.
    pub intent: Option<BudgetIntent>,
    /// Bar segment: spend claimed by sub-budgets, or by the budget itself.
    pub claimed: Decimal,
    /// Bar segment: envelope-owned spend under this row.
    pub unallocated: Decimal,
    /// Bar segment: unbudgeted spend under this row.
    pub unbudgeted: Decimal,
    /// Traffic light against the paced target; `None` is neutral.
    pub verdict: Option<Verdict>,
    /// `actual ÷ paced reference`, as the verdict uses it; `None` without a
    /// usable reference.
    pub ratio: Option<Decimal>,
    /// Worst verdict among every row beneath this one.
    pub worst_descendant: Option<Verdict>,
    /// The aggregate rule did not apply: the budgets beneath differ in
    /// intent, target sign or commodity, or the actuals span commodities.
    pub mixed: bool,
    /// A posting beneath counts in two of this row's children.
    pub double_counted: bool,
    /// The envelope's sub-budget targets exceed its own.
    pub over_allocated: bool,
    /// A revision of this budget flips sign against a neighbour.
    pub sign_flip: bool,
    /// `true` when the budget's native period differs from the display period.
    pub has_mixed_period: bool,
    /// Native amounts that fed no total, by commodity (see
    /// [`crate::BudgetStatus::unvalued`]).
    pub unvalued: bc_models::Balances,
    /// Every posting behind the row, with its bucket label: the owning
    /// budget's label, `↳ unallocated` for an envelope's own postings, or
    /// `None` when a budget without sub-budgets or a leftover row owns it.
    pub postings: Vec<RowPosting>,
    /// Rows nested under this one.
    pub children: Vec<BudgetTreeItem>,
}

// MARK: RowPosting

/// One posting behind a budget row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RowPosting {
    /// The posting (or elided-leg component).
    pub key: PostingKey,
    /// The bucket label: the owning budget's label, `↳ unallocated` for an
    /// envelope's own postings, or `None` when a budget without sub-budgets
    /// or a leftover row owns it.
    pub bucket: Option<String>,
    /// The posting's value in the row's commodity; `None` when the row has
    /// no commodity or the posting is unconvertible. An account row over
    /// children in different commodities keeps each child's commodity.
    pub value: Option<Amount>,
    /// The native amount.
    pub amount: Amount,
}

// MARK: BudgetTreeSummary

/// Header figures for the budget overview.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BudgetTreeSummary {
    /// Budget and unallocated rows with a red verdict.
    pub red: u32,
    /// Budget and unallocated rows with a warn verdict.
    pub warn: u32,
    /// Budget and unallocated rows with a green verdict.
    pub green: u32,
    /// Unbudgeted total per type root with a non-zero total: the root's id,
    /// its account name, and the total.
    pub unbudgeted: Vec<(AccountId, String, Amount)>,
    /// `true` when any row has a non-empty `unvalued`. A flag rather than a
    /// sum: rows can target different commodities.
    pub has_unvalued: bool,
}

// MARK: BudgetOverview

/// The complete budget page data for one display window.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BudgetOverview {
    /// Header figures.
    pub summary: BudgetTreeSummary,
    /// Type root rows.
    pub nodes: Vec<BudgetTreeItem>,
    /// Fraction of the window elapsed by the end of `today`; `None` for a
    /// window that has not started.
    pub elapsed_fraction: Option<Decimal>,
}

// MARK: BudgetTreeService

/// Assembles a `BudgetOverview` for a given display period and window start.
#[derive(Clone)]
pub struct BudgetTreeService {
    /// The SQLite connection pool.
    pool: SqlitePool,
    /// Foreign exchange rate service for cross-commodity conversion.
    fx: Arc<dyn crate::fx::FxRateService>,
}

impl core::fmt::Debug for BudgetTreeService {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("BudgetTreeService")
            .field("pool", &self.pool)
            .finish_non_exhaustive()
    }
}

impl BudgetTreeService {
    /// Creates a new [`BudgetTreeService`].
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool, fx: Arc<dyn crate::fx::FxRateService>) -> Self {
        Self { pool, fx }
    }

    /// Builds the budget overview for the display period starting on `display_start`.
    ///
    /// # Arguments
    ///
    /// * `display_period` - The display period type.
    /// * `display_start` - A date inside the display window.
    /// * `query` - Optional global filter; it narrows budget and unbudgeted
    ///   postings alike.
    /// * `today` - The day verdicts are paced to.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn get_overview(
        &self,
        display_period: &Period,
        display_start: Date,
        query: Option<&crate::search::TransactionQuery>,
        today: Date,
    ) -> crate::BcResult<BudgetOverview> {
        Ok(self
            .assemble(display_period, display_start, query, today)
            .await?
            .0)
    }

    /// Lists the postings behind one row of the overview.
    ///
    /// # Arguments
    ///
    /// * `row_id` - The row's [`BudgetTreeItem::id`].
    /// * `display_period`, `display_start`, `query`, `today` - As for
    ///   [`Self::get_overview`].
    ///
    /// # Returns
    ///
    /// Each posting (as [`BudgetTreeItem::postings`]) and whether it counts in
    /// two budgets where neither row nests the other.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] when no row has `row_id`, or
    /// [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn row_postings(
        &self,
        row_id: &str,
        display_period: &Period,
        display_start: Date,
        query: Option<&crate::search::TransactionQuery>,
        today: Date,
    ) -> crate::BcResult<Vec<(RowPosting, bool)>> {
        let (overview, double_counted) = self
            .assemble(display_period, display_start, query, today)
            .await?;
        let row = find_row(&overview.nodes, row_id)
            .ok_or_else(|| crate::BcError::NotFound(format!("budget row {row_id}")))?;
        Ok(row
            .postings
            .iter()
            .map(|p| (p.clone(), double_counted.contains(&p.key)))
            .collect())
    }

    /// Runs every assembly step and returns the overview with the postings
    /// counted in two budgets where neither row nests the other.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    async fn assemble(
        &self,
        display_period: &Period,
        display_start: Date,
        query: Option<&crate::search::TransactionQuery>,
        today: Date,
    ) -> crate::BcResult<(BudgetOverview, HashSet<PostingKey>)> {
        let (start, end) = display_period.range_containing(display_start);
        let window = Window { start, end, today };

        let (loaded, accounts) = self.load(display_period, window, query).await?;
        let (matches, owners, parents) = partition(&loaded);
        let unmatched = self
            .unmatched(&loaded, &accounts, &matches, window, query)
            .await?;

        let mut skeleton = Skeleton::new(&loaded, &parents);
        skeleton.merge_unfiltered(&loaded);
        skeleton.prune();
        skeleton.add_leftovers(&parents, &unmatched, &accounts);

        let (double_rows, double_keys) = skeleton.double_counted(&matches);
        let assembler = Assembler::new(
            &loaded,
            &accounts,
            &owners,
            &parents,
            &skeleton,
            double_rows,
            window,
        );
        let nodes = assembler.roots();
        let summary = summarise(&nodes, &unmatched, &accounts);
        let elapsed_fraction = bc_models::elapsed_fraction(start, end, today);
        Ok((
            BudgetOverview {
                summary,
                nodes,
                elapsed_fraction,
            },
            double_keys,
        ))
    }

    /// Loads every active budget and values its postings in the window.
    ///
    /// Each budget gets its governing revision, window-effective target,
    /// `unvalued` and `has_mixed_period` from its status, its valued
    /// postings, and its [`Scope`]: the account chain from the type root and
    /// the filter tag's chain from the tag root.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::NotFound`] when a budget's account is not
    /// active, or [`crate::BcError`] on database or data parse failure.
    async fn load(
        &self,
        display_period: &Period,
        window: Window,
        query: Option<&crate::search::TransactionQuery>,
    ) -> crate::BcResult<(Vec<Loaded>, HashMap<AccountId, bc_models::Account>)> {
        let budget_svc = BudgetService::new(self.pool.clone());
        let engine = BudgetStatusEngine::new(self.pool.clone(), Arc::clone(&self.fx));
        let accounts: HashMap<AccountId, bc_models::Account> =
            crate::account::Service::new(self.pool.clone())
                .list_active()
                .await?
                .into_iter()
                .map(|a| (a.id().clone(), a))
                .collect();
        let forest = crate::tag::Service::new(self.pool.clone()).forest().await?;
        let display_window =
            bc_models::BudgetWindow::custom(window.start, window.end, "display".to_owned());

        let mut loaded = Vec::new();
        for budget in budget_svc.list().await? {
            let revs = budget_svc.revisions(budget.id()).await?;
            let governing = bc_models::governing_revision(&revs, window.start).cloned();
            let periods = bc_models::periods_overlapping(&revs, window.start, window.end);
            let config = governing
                .as_ref()
                .or_else(|| periods.first().map(|p| p.revision))
                .or_else(|| revs.first());

            let target_value = crate::budget::prorated_target(&revs, window.start, window.end)?;
            let target_commodity = periods
                .iter()
                .find_map(|p| p.revision.target())
                .map(|t| t.commodity().clone());
            let target = target_value
                .zip(target_commodity)
                .map(|(value, commodity)| Amount::new(value, commodity));

            let status = engine
                .status_for_window(&budget, display_window.clone(), query)
                .await?;
            let valuation = engine
                .window_postings(&budget, &display_window, query)
                .await?;

            let account = accounts
                .get(budget.account_id())
                .ok_or_else(|| crate::BcError::NotFound(budget.account_id().to_string()))?
                .clone();

            let has_mixed_period = {
                let distinct_revs: HashSet<_> = periods.iter().map(|p| p.revision.id()).collect();
                distinct_revs.len() > 1
                    || !governing
                        .as_ref()
                        .map(bc_models::BudgetRevision::period)
                        .is_none_or(|p| periods_equivalent(display_period, p))
            };

            let tag = config.and_then(bc_models::BudgetRevision::tag_filter);
            let (tag_chain, tag_names) = tag.map_or_else(
                || (None, Vec::new()),
                |t| {
                    let mut ancestors: Vec<&bc_models::Tag> = forest.ancestors_of(t).collect();
                    ancestors.reverse();
                    if ancestors.is_empty() {
                        (Some(vec![t.clone()]), vec![t.to_string()])
                    } else {
                        (
                            Some(ancestors.iter().map(|a| a.id().clone()).collect()),
                            ancestors.iter().map(|a| a.name().to_owned()).collect(),
                        )
                    }
                },
            );
            let tag_filter = tag.map(|t| {
                let path = forest
                    .path_of(t)
                    .map_or_else(|| t.to_string(), |p| p.to_string());
                (t.clone(), path)
            });
            let name = config
                .and_then(bc_models::BudgetRevision::name)
                .map(ToOwned::to_owned);
            let intent = config.map_or_else(
                || BudgetIntent::default_for(account.account_type()),
                bc_models::BudgetRevision::intent,
            );

            loaded.push(Loaded {
                scope: Scope {
                    account_chain: account_chain(account.id(), &accounts),
                    tag_chain,
                },
                sign_flip: !crate::budget::sign_flips(&revs).is_empty(),
                budget,
                account,
                governing,
                name,
                tag_names,
                tag_filter,
                target,
                intent,
                postings: valuation.postings,
                commodity: valuation.commodity,
                unvalued: status.unvalued,
                has_mixed_period,
            });
        }
        Ok((loaded, accounts))
    }

    /// Lists, per `Income` or `Expense` type root holding a budget, the
    /// postings no budget matches, valued in the root's report commodity.
    ///
    /// `matches` holds every key a budget matched, unvaluable ones included,
    /// so a matched posting never counts here as well.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    async fn unmatched(
        &self,
        loaded: &[Loaded],
        accounts: &HashMap<AccountId, bc_models::Account>,
        matches: &HashMap<PostingKey, Vec<usize>>,
        window: Window,
        query: Option<&crate::search::TransactionQuery>,
    ) -> crate::BcResult<Vec<Unmatched>> {
        let engine = BudgetStatusEngine::new(self.pool.clone(), Arc::clone(&self.fx));
        let display_window =
            bc_models::BudgetWindow::custom(window.start, window.end, "display".to_owned());
        let mut roots: Vec<&AccountId> = Vec::new();
        for root in loaded.iter().filter_map(|l| l.scope.account_chain.first()) {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }

        let mut out = Vec::new();
        for root in roots {
            let tracks_flow = accounts.get(root).is_some_and(|a| {
                matches!(
                    a.account_type(),
                    bc_models::AccountType::Income | bc_models::AccountType::Expense
                )
            });
            if !tracks_flow {
                continue;
            }
            let commodity = report_commodity(
                loaded
                    .iter()
                    .filter(|l| l.scope.account_chain.first() == Some(root)),
            );
            let postings = engine
                .subtree_postings(root, &display_window, query)
                .await?
                .into_iter()
                .filter(|(key, _, _)| !matches.contains_key(key))
                .map(|(key, account_id, amount)| {
                    let value = commodity
                        .as_ref()
                        .and_then(|c| self.fx.convert(&amount, c).ok())
                        .map(|a| a.value());
                    UnmatchedPosting {
                        key,
                        account_id,
                        value,
                        amount,
                    }
                })
                .collect();
            out.push(Unmatched {
                root: root.clone(),
                commodity,
                postings,
            });
        }
        Ok(out)
    }

    /// Returns native period breakdown for one budget within `[display_start, display_end)`.
    ///
    /// `query` is the same global transaction filter applied to the tree overview
    /// ([`Self::get_overview`]); passing it here keeps the native-period drill-down
    /// consistent with whatever the tree counted. `today` paces open sub-periods
    /// for their verdict.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn native_periods(
        &self,
        budget: &bc_models::Budget,
        display_start: Date,
        display_end: Date,
        query: Option<&crate::search::TransactionQuery>,
        today: Date,
    ) -> crate::BcResult<Vec<NativePeriodStatus>> {
        let budget_svc = BudgetService::new(self.pool.clone());
        let status_engine = BudgetStatusEngine::new(self.pool.clone(), Arc::clone(&self.fx));

        let revs = budget_svc.revisions(budget.id()).await?;

        // Value the whole display window once; each sub-row slices that valuation by date, so
        // the rows share one commodity and their actuals sum to the window's.
        let window =
            bc_models::BudgetWindow::custom(display_start, display_end, display_start.to_string());
        let valuation = status_engine
            .window_postings(budget, &window, query)
            .await?;

        // `bc_models::periods_overlapping` tiles each revision's grid from its `effective_from`,
        // returning the natural period boundaries (`rp.start..rp.end`), which are clipped to
        // `[display_start, display_end)` to get the overlap span.
        let mut result = Vec::new();
        for rp in bc_models::periods_overlapping(&revs, display_start, display_end) {
            let overlap = crate::period_overlap::PeriodOverlap {
                native_start: rp.start,
                native_end: rp.end,
                overlap_start: rp.start.max(display_start),
                overlap_end: rp.end.min(display_end),
            };
            let mut actuals = Decimal::ZERO;
            let mut unvalued = bc_models::Balances::new();
            for p in valuation
                .postings
                .iter()
                .filter(|p| (overlap.overlap_start..overlap.overlap_end).contains(&p.date))
            {
                match p.value {
                    Some(v) => {
                        actuals = actuals
                            .checked_add(v)
                            .ok_or_else(|| crate::BcError::BadData("actuals overflow".into()))?;
                    }
                    None => crate::budget::add_unvalued(&mut unvalued, &p.amount)?,
                }
            }
            let effective_target =
                crate::budget::prorated_target(&revs, overlap.overlap_start, overlap.overlap_end)?;
            let commodity = valuation.commodity.clone();
            let actual = commodity
                .clone()
                .map(|c| bc_models::Amount::new(actuals, c));
            let target = effective_target
                .zip(commodity.clone())
                .map(|(v, c)| bc_models::Amount::new(v, c));
            let judgement = bc_models::judge(
                rp.revision.intent(),
                actual.as_ref(),
                target.as_ref(),
                overlap.overlap_start,
                overlap.overlap_end,
                today,
            );
            result.push(NativePeriodStatus {
                effective_target,
                overlap,
                actuals,
                commodity,
                unvalued,
                verdict: judgement.verdict,
                ratio: judgement.ratio,
            });
        }

        Ok(result)
    }
}

// MARK: NativePeriodStatus

/// Status for one native period overlapping the display window.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct NativePeriodStatus {
    /// The period overlap (native range and overlap range with the display window).
    pub overlap: crate::period_overlap::PeriodOverlap,
    /// Pro-rated effective target for this overlap. `None` = tracking-only.
    pub effective_target: Option<Decimal>,
    /// Actuals within the overlap.
    pub actuals: Decimal,
    /// Commodity of the actuals.
    pub commodity: Option<bc_models::CommodityCode>,
    /// Native amounts in the overlap that fed no total, by commodity.
    pub unvalued: bc_models::Balances,
    /// Core's verdict for this overlap, paced to today. `None` when it has no verdict.
    pub verdict: Option<bc_models::Verdict>,
    /// `actual ÷ paced reference`.
    pub ratio: Option<Decimal>,
}

// MARK: Helpers

/// Returns `true` when `a` and `b` are the same [`Period`] variant.
fn periods_equivalent(a: &Period, b: &Period) -> bool {
    core::mem::discriminant(a) == core::mem::discriminant(b)
}

// MARK: Loading

/// The display window and the day verdicts are paced to.
#[derive(Clone, Copy, Debug)]
struct Window {
    /// Inclusive start.
    start: Date,
    /// Exclusive end.
    end: Date,
    /// The day verdicts are paced to.
    today: Date,
}

/// One active budget, loaded and valued for the display window.
#[derive(Debug)]
struct Loaded {
    /// The budget anchor.
    budget: bc_models::Budget,
    /// The budget's account.
    account: bc_models::Account,
    /// The revision governing the window start.
    governing: Option<bc_models::BudgetRevision>,
    /// The postings the budget can match.
    scope: Scope,
    /// The governing revision's name, which overrides the derived label.
    name: Option<String>,
    /// Tag names along `scope.tag_chain`, root first.
    tag_names: Vec<String>,
    /// ID and path of the filter's tag, if any.
    tag_filter: Option<(bc_models::TagId, String)>,
    /// Window-effective target.
    target: Option<Amount>,
    /// What the target is for.
    intent: BudgetIntent,
    /// Every posting the budget matched in the window.
    postings: Vec<ValuedPosting>,
    /// Commodity of the posting values.
    commodity: Option<CommodityCode>,
    /// Native amounts the status could not value.
    unvalued: bc_models::Balances,
    /// `true` when the native period differs from the display period.
    has_mixed_period: bool,
    /// `true` when a revision flips sign against a neighbour.
    sign_flip: bool,
}

impl Loaded {
    /// Sum of the valued postings.
    fn total(&self) -> Decimal {
        self.postings.iter().filter_map(|p| p.value).sum()
    }

    /// The row's actual: the valued total in the budget's commodity.
    fn actual(&self) -> Option<Amount> {
        self.commodity.clone().map(|c| Amount::new(self.total(), c))
    }
}

/// A posting no budget matches.
#[derive(Debug, Clone)]
struct UnmatchedPosting {
    /// The posting (or elided-leg component).
    key: PostingKey,
    /// The account the posting is on.
    account_id: AccountId,
    /// The amount in the root's report commodity; `None` when unconvertible.
    value: Option<Decimal>,
    /// The native amount.
    amount: Amount,
}

/// The postings no budget matches under one `Income` or `Expense` type root.
#[derive(Debug)]
struct Unmatched {
    /// The type root.
    root: AccountId,
    /// The commodity the postings are valued in.
    commodity: Option<CommodityCode>,
    /// The postings.
    postings: Vec<UnmatchedPosting>,
}

/// Walks `account`'s parent chain and returns it root-first.
///
/// The walk stops at an account missing from `accounts` or at a cycle.
fn account_chain(
    account: &AccountId,
    accounts: &HashMap<AccountId, bc_models::Account>,
) -> Vec<AccountId> {
    let mut chain = vec![account.clone()];
    let mut seen: HashSet<AccountId> = HashSet::from([account.clone()]);
    let mut current = accounts.get(account).and_then(|a| a.parent_id()).cloned();
    while let Some(id) = current {
        if !seen.insert(id.clone()) {
            tracing::warn!(account_id = %id, "cycle detected in account parent chain");
            break;
        }
        let Some(parent) = accounts.get(&id) else {
            break;
        };
        current = parent.parent_id().cloned();
        chain.push(id);
    }
    chain.reverse();
    chain
}

/// The commodity most of `budgets` value in (their target's, else their
/// actuals'), ties going to the lowest code.
fn report_commodity<'a>(budgets: impl Iterator<Item = &'a Loaded>) -> Option<CommodityCode> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for l in budgets {
        let code = l
            .target
            .as_ref()
            .map(|t| t.commodity().clone())
            .or_else(|| l.commodity.clone());
        if let Some(c) = code {
            let n = counts.entry(c.as_str().to_owned()).or_default();
            *n = n.saturating_add(1);
        }
    }
    let mut best: Option<(&String, usize)> = None;
    for (code, &n) in &counts {
        if best.is_none_or(|(_, m)| n > m) {
            best = Some((code, n));
        }
    }
    best.map(|(code, _)| CommodityCode::new(code.clone()))
}

// MARK: Ownership

/// Every budget each posting matched, each posting's owner, and each
/// budget's envelope parent.
type Partition = (
    HashMap<PostingKey, Vec<usize>>,
    HashMap<PostingKey, Owner>,
    Vec<Option<usize>>,
);

/// Partitions every matched posting among the budgets that matched it.
fn partition(loaded: &[Loaded]) -> Partition {
    let scopes: Vec<Scope> = loaded.iter().map(|l| l.scope.clone()).collect();
    let mut matches: HashMap<PostingKey, Vec<usize>> = HashMap::new();
    for (i, l) in loaded.iter().enumerate() {
        for p in &l.postings {
            matches.entry(p.key.clone()).or_default().push(i);
        }
    }
    let owners = crate::budget_partition::owners(&scopes, &matches);
    let parents = crate::budget_partition::envelope_parents(&scopes);
    (matches, owners, parents)
}

// MARK: Skeleton

/// A row under construction.
#[derive(Debug)]
struct Draft {
    /// What the row stands for.
    kind: RowKind,
    /// The anchoring account.
    account: AccountId,
    /// The budget (budget rows) or envelope (unallocated rows).
    budget: Option<usize>,
    /// The parent row.
    parent: Option<usize>,
    /// `false` once merged away or pruned.
    alive: bool,
    /// The unmatched postings of an unbudgeted row.
    unmatched: Vec<UnmatchedPosting>,
    /// The commodity of an unbudgeted row's values.
    commodity: Option<CommodityCode>,
}

impl Draft {
    /// A live row with no unmatched postings.
    const fn new(
        kind: RowKind,
        account: AccountId,
        budget: Option<usize>,
        parent: Option<usize>,
    ) -> Self {
        Self {
            kind,
            account,
            budget,
            parent,
            alive: true,
            unmatched: Vec::new(),
            commodity: None,
        }
    }
}

/// The tree's shape, as rows pointing at their parents.
#[derive(Debug)]
struct Skeleton {
    /// Every row, dead ones included.
    rows: Vec<Draft>,
    /// The row standing for each row account: its account row, or the
    /// budget row merged into it.
    by_account: HashMap<AccountId, usize>,
    /// Each budget's row.
    by_budget: Vec<usize>,
}

impl Skeleton {
    /// Builds account rows for every account on a budget's chain and places
    /// each budget under its envelope, or else under its account's row.
    fn new(loaded: &[Loaded], parents: &[Option<usize>]) -> Self {
        let mut skeleton = Self {
            rows: Vec::new(),
            by_account: HashMap::new(),
            by_budget: Vec::with_capacity(loaded.len()),
        };
        for l in loaded {
            let mut parent = None;
            for account in &l.scope.account_chain {
                let row = if let Some(&existing) = skeleton.by_account.get(account) {
                    existing
                } else {
                    let added =
                        skeleton.push(Draft::new(RowKind::Account, account.clone(), None, parent));
                    skeleton.by_account.insert(account.clone(), added);
                    added
                };
                parent = Some(row);
            }
        }
        for (i, l) in loaded.iter().enumerate() {
            let row = skeleton.push(Draft::new(
                RowKind::Budget,
                l.account.id().clone(),
                Some(i),
                None,
            ));
            skeleton.by_budget.push(row);
        }
        for (i, l) in loaded.iter().enumerate() {
            let parent = match parents.get(i).copied().flatten() {
                Some(envelope) => skeleton.by_budget.get(envelope).copied(),
                None => skeleton.by_account.get(l.account.id()).copied(),
            };
            if let Some(draft) = skeleton
                .by_budget
                .get(i)
                .and_then(|&row| skeleton.rows.get_mut(row))
            {
                draft.parent = parent;
            }
        }
        skeleton
    }

    /// Appends `draft` and returns its index.
    fn push(&mut self, draft: Draft) -> usize {
        self.rows.push(draft);
        self.rows.len().saturating_sub(1)
    }

    /// Merges an unfiltered budget into its account's row when it is the
    /// only budget placed directly under that row.
    ///
    /// Every budget on the same or a deeper account is more specific than an
    /// unfiltered one, so it nests under the unfiltered budget. An
    /// unfiltered budget alone under its account row therefore covers
    /// everything the row would.
    fn merge_unfiltered(&mut self, loaded: &[Loaded]) {
        let mut direct: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (row, draft) in self.rows.iter().enumerate() {
            let under_account = draft
                .parent
                .and_then(|p| self.rows.get(p))
                .is_some_and(|p| p.kind == RowKind::Account);
            if draft.kind == RowKind::Budget
                && under_account
                && let Some(parent) = draft.parent
            {
                direct.entry(parent).or_default().push(row);
            }
        }
        for (account_row, budget_rows) in direct {
            let [budget_row] = budget_rows.as_slice() else {
                continue;
            };
            let unfiltered = self
                .rows
                .get(*budget_row)
                .and_then(|d| d.budget)
                .and_then(|i| loaded.get(i))
                .is_some_and(|l| l.scope.tag_chain.is_none());
            if unfiltered {
                self.merge(account_row, *budget_row);
            }
        }
    }

    /// Replaces `account_row` with `budget_row` in the tree.
    fn merge(&mut self, account_row: usize, budget_row: usize) {
        let Some((parent, account)) = self
            .rows
            .get(account_row)
            .map(|d| (d.parent, d.account.clone()))
        else {
            return;
        };
        for (row, draft) in self.rows.iter_mut().enumerate() {
            if row == budget_row {
                draft.parent = parent;
            } else if draft.parent == Some(account_row) {
                draft.parent = Some(budget_row);
            } else if row == account_row {
                draft.alive = false;
            }
        }
        self.by_account.insert(account, budget_row);
    }

    /// Drops, bottom up, every account row left with no live children.
    fn prune(&mut self) {
        loop {
            let parents: HashSet<usize> = self
                .rows
                .iter()
                .filter(|d| d.alive)
                .filter_map(|d| d.parent)
                .collect();
            let mut changed = false;
            for (row, draft) in self.rows.iter_mut().enumerate() {
                if draft.alive && draft.kind == RowKind::Account && !parents.contains(&row) {
                    draft.alive = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
    }

    /// The live row standing for `account`, if any.
    fn live_row(&self, account: &AccountId) -> Option<usize> {
        self.by_account
            .get(account)
            .copied()
            .filter(|&row| self.rows.get(row).is_some_and(|d| d.alive))
    }

    /// The row an unmatched posting on `account` falls under: the live
    /// account row of `account` or its nearest ancestor, else the nearest
    /// live row of any kind.
    ///
    /// A merged budget row sums only its budget's postings, so a posting it
    /// leaves unmatched (one dated before its first revision) belongs to the
    /// account row above it.
    fn unbudgeted_parent(
        &self,
        account: &AccountId,
        accounts: &HashMap<AccountId, bc_models::Account>,
    ) -> Option<usize> {
        let mut seen: HashSet<&AccountId> = HashSet::new();
        let mut fallback = None;
        let mut current = Some(account);
        while let Some(id) = current {
            if !seen.insert(id) {
                break;
            }
            if let Some(row) = self.live_row(id) {
                if self
                    .rows
                    .get(row)
                    .is_some_and(|d| d.kind == RowKind::Account)
                {
                    return Some(row);
                }
                fallback = fallback.or(Some(row));
            }
            current = accounts.get(id).and_then(|a| a.parent_id());
        }
        fallback
    }

    /// Adds an `↳ unallocated` row under every envelope and an
    /// `↳ unbudgeted` row under each live account row whose unmatched
    /// postings are non-zero or partly unvaluable (see
    /// [`Self::unbudgeted_parent`]).
    ///
    /// Runs after pruning, so a posting under a pruned account lands in the
    /// nearest surviving ancestor's row.
    fn add_leftovers(
        &mut self,
        parents: &[Option<usize>],
        unmatched: &[Unmatched],
        accounts: &HashMap<AccountId, bc_models::Account>,
    ) {
        let envelopes: std::collections::BTreeSet<usize> =
            parents.iter().flatten().copied().collect();
        for envelope in envelopes {
            let Some(&row) = self.by_budget.get(envelope) else {
                continue;
            };
            let Some(account) = self.rows.get(row).map(|d| d.account.clone()) else {
                continue;
            };
            self.push(Draft::new(
                RowKind::Unallocated,
                account,
                Some(envelope),
                Some(row),
            ));
        }

        for group in unmatched {
            let mut by_row: BTreeMap<usize, Vec<UnmatchedPosting>> = BTreeMap::new();
            for posting in &group.postings {
                if let Some(row) = self.unbudgeted_parent(&posting.account_id, accounts) {
                    by_row.entry(row).or_default().push(posting.clone());
                } else {
                    tracing::warn!(
                        account_id = %posting.account_id,
                        "unbudgeted posting has no row above it"
                    );
                }
            }
            for (row, postings) in by_row {
                let total: Decimal = postings.iter().filter_map(|p| p.value).sum();
                let unvaluable = postings.iter().any(|p| p.value.is_none());
                if total.is_zero() && !unvaluable {
                    continue;
                }
                let Some(account) = self.rows.get(row).map(|d| d.account.clone()) else {
                    continue;
                };
                let mut draft = Draft::new(RowKind::Unbudgeted, account, None, Some(row));
                draft.unmatched = postings;
                draft.commodity.clone_from(&group.commodity);
                self.push(draft);
            }
        }
    }

    /// Live rows grouped by parent; roots under `None`.
    fn children(&self) -> HashMap<Option<usize>, Vec<usize>> {
        let mut out: HashMap<Option<usize>, Vec<usize>> = HashMap::new();
        for (row, draft) in self.rows.iter().enumerate() {
            if draft.alive {
                out.entry(draft.parent).or_default().push(row);
            }
        }
        out
    }

    /// Row indices from the root down to `row`.
    fn path(&self, row: usize) -> Vec<usize> {
        let mut path = vec![row];
        let mut current = self.rows.get(row).and_then(|d| d.parent);
        while let Some(parent) = current {
            if path.contains(&parent) {
                break;
            }
            path.push(parent);
            current = self.rows.get(parent).and_then(|d| d.parent);
        }
        path.reverse();
        path
    }

    /// Double counting read from the full match sets.
    ///
    /// For each posting, any two matching budget rows where neither is an
    /// ancestor of the other both count it; the row where their paths meet
    /// sums it twice.
    ///
    /// # Returns
    ///
    /// The rows that sum a posting twice, and the postings counted twice.
    fn double_counted(
        &self,
        matches: &HashMap<PostingKey, Vec<usize>>,
    ) -> (HashSet<usize>, HashSet<PostingKey>) {
        let mut rows = HashSet::new();
        let mut keys = HashSet::new();
        #[expect(
            clippy::iter_over_hash_type,
            reason = "the results are sets, so visiting order does not matter"
        )]
        for (key, budgets) in matches {
            let paths: Vec<Vec<usize>> = budgets
                .iter()
                .filter_map(|&i| self.by_budget.get(i))
                .map(|&row| self.path(row))
                .collect();
            for (n, a) in paths.iter().enumerate() {
                for b in paths.iter().skip(n.saturating_add(1)) {
                    let common = a.iter().zip(b).take_while(|(x, y)| x == y).count();
                    if common < a.len() && common < b.len() {
                        keys.insert(key.clone());
                        if let Some(&lca) = common.checked_sub(1).and_then(|k| a.get(k)) {
                            rows.insert(lca);
                        }
                    }
                }
            }
        }
        (rows, keys)
    }

    /// Budget `i`'s label relative to its parent row (see
    /// [`BudgetTreeItem::label`]).
    fn label(
        &self,
        loaded: &[Loaded],
        accounts: &HashMap<AccountId, bc_models::Account>,
        i: usize,
    ) -> String {
        let Some(l) = loaded.get(i) else {
            return String::new();
        };
        if let Some(name) = &l.name {
            return name.clone();
        }
        let parent = self
            .by_budget
            .get(i)
            .and_then(|&row| self.rows.get(row))
            .and_then(|d| d.parent)
            .and_then(|p| self.rows.get(p));

        let chain = &l.scope.account_chain;
        let skip = parent
            .and_then(|p| chain.iter().position(|a| *a == p.account))
            .map_or(0, |k| k.saturating_add(1));
        let account_part = chain
            .iter()
            .skip(skip)
            .filter_map(|a| accounts.get(a).map(bc_models::Account::name))
            .collect::<Vec<_>>()
            .join(":");

        let parent_tags: &[bc_models::TagId] = parent
            .filter(|p| p.kind == RowKind::Budget)
            .and_then(|p| p.budget)
            .and_then(|j| loaded.get(j))
            .and_then(|pl| pl.scope.tag_chain.as_deref())
            .unwrap_or(&[]);
        let tag_part = match &l.scope.tag_chain {
            Some(tags) => {
                let shared = if tags.starts_with(parent_tags) {
                    parent_tags.len()
                } else {
                    0
                };
                l.tag_names
                    .iter()
                    .skip(shared)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(":")
            }
            None => String::new(),
        };

        match (account_part.is_empty(), tag_part.is_empty()) {
            (false, true) => account_part,
            (true, false) => tag_part,
            (false, false) => format!("{account_part} #{tag_part}"),
            (true, true) => l.account.name().to_owned(),
        }
    }
}

// MARK: Figures

/// A budget's intent and target, as an aggregate row sees it.
#[derive(Debug, Clone)]
struct Outer {
    /// The budget's intent.
    intent: BudgetIntent,
    /// The budget's window-effective target.
    target: Option<Amount>,
}

impl Outer {
    /// What must agree across the budgets beneath an aggregate row.
    fn agreement(&self) -> (BudgetIntent, Option<(bool, CommodityCode)>) {
        (
            self.intent,
            self.target
                .as_ref()
                .map(|t| (t.value().is_sign_negative(), t.commodity().clone())),
        )
    }
}

/// A finished row, with what its parent needs to aggregate it.
#[derive(Debug)]
struct Built {
    /// The row.
    item: BudgetTreeItem,
    /// The outermost budgets at or beneath the row.
    outer: Vec<Outer>,
    /// The actuals at or beneath the row span commodities.
    actual_mixed: bool,
}

/// Computes each row's figures, bottom up.
struct Assembler<'a> {
    /// The budgets.
    loaded: &'a [Loaded],
    /// Active accounts by id.
    accounts: &'a HashMap<AccountId, bc_models::Account>,
    /// Each matched posting's owner.
    owners: &'a HashMap<PostingKey, Owner>,
    /// Each budget's envelope parent.
    parents: &'a [Option<usize>],
    /// Budgets with at least one sub-budget.
    envelopes: HashSet<usize>,
    /// The tree's shape.
    skeleton: &'a Skeleton,
    /// Live rows grouped by parent.
    children: HashMap<Option<usize>, Vec<usize>>,
    /// Rows flagged double-counted.
    double_counted: HashSet<usize>,
    /// Each budget row's label.
    labels: Vec<String>,
    /// The display window.
    window: Window,
}

impl<'a> Assembler<'a> {
    /// Prepares the lookups the figures need.
    fn new(
        loaded: &'a [Loaded],
        accounts: &'a HashMap<AccountId, bc_models::Account>,
        owners: &'a HashMap<PostingKey, Owner>,
        parents: &'a [Option<usize>],
        skeleton: &'a Skeleton,
        double_counted: HashSet<usize>,
        window: Window,
    ) -> Self {
        Self {
            loaded,
            accounts,
            owners,
            parents,
            envelopes: parents.iter().flatten().copied().collect(),
            skeleton,
            children: skeleton.children(),
            double_counted,
            labels: (0..loaded.len())
                .map(|i| skeleton.label(loaded, accounts, i))
                .collect(),
            window,
        }
    }

    /// The finished root rows, sorted.
    fn roots(&self) -> Vec<BudgetTreeItem> {
        self.build_all(None).into_iter().map(|b| b.item).collect()
    }

    /// Builds and sorts the live children of `parent`.
    fn build_all(&self, parent: Option<usize>) -> Vec<Built> {
        let mut built: Vec<Built> = self
            .children
            .get(&parent)
            .map(|rows| rows.iter().filter_map(|&row| self.build(row)).collect())
            .unwrap_or_default();
        built.sort_by(|a, b| {
            (a.item.kind.rank(), &a.item.label, &a.item.id).cmp(&(
                b.item.kind.rank(),
                &b.item.label,
                &b.item.id,
            ))
        });
        built
    }

    /// Builds `row` and everything beneath it.
    fn build(&self, row: usize) -> Option<Built> {
        let draft = self.skeleton.rows.get(row)?;
        let children = self.build_all(Some(row));
        let mut built = match (draft.kind, draft.budget) {
            (RowKind::Budget, Some(i)) => self.budget_row(i)?,
            (RowKind::Unallocated, Some(i)) => self.unallocated_row(i)?,
            (RowKind::Unbudgeted, _) => self.unbudgeted_row(draft, row)?,
            _ => self.account_row(draft, &children)?,
        };
        built.item.double_counted = self.double_counted.contains(&row);
        built.item.worst_descendant = children
            .iter()
            .flat_map(|c| [c.item.verdict, c.item.worst_descendant])
            .flatten()
            .max();
        built.item.children = children.into_iter().map(|c| c.item).collect();
        Some(built)
    }

    /// The label of `key`'s bucket as seen from budget `me`:
    /// `↳ unallocated` when envelope `me` owns it, `None` when `me` owns it
    /// and has no sub-budgets.
    fn owner_label(&self, key: &PostingKey, me: usize) -> Option<String> {
        let label = |i: usize| self.labels.get(i).cloned();
        match self.owners.get(key)? {
            Owner::Budget(i) if *i == me => self
                .envelopes
                .contains(&me)
                .then(|| UNALLOCATED_LABEL.to_owned()),
            Owner::Budget(i) => label(*i),
            Owner::Shared(all) if all.contains(&me) => None,
            Owner::Shared(all) => Some(
                all.iter()
                    .filter_map(|&i| label(i))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        }
    }

    /// The postings budget `i` owns outright.
    fn owned(&self, i: usize) -> impl Iterator<Item = &ValuedPosting> {
        self.loaded
            .get(i)
            .into_iter()
            .flat_map(|l| &l.postings)
            .filter(move |p| self.owners.get(&p.key) == Some(&Owner::Budget(i)))
    }

    /// The envelope's own sum; zero for a budget without sub-budgets.
    fn own_unallocated(&self, i: usize) -> Decimal {
        if self.envelopes.contains(&i) {
            self.owned(i).filter_map(|p| p.value).sum()
        } else {
            Decimal::ZERO
        }
    }

    /// Envelope `i`'s target less its direct sub-budgets' targets, and
    /// whether a sub-budget targets another commodity.
    fn unallocated_target(&self, i: usize) -> (Option<Amount>, bool) {
        let Some(target) = self.loaded.get(i).and_then(|l| l.target.as_ref()) else {
            return (None, false);
        };
        let mut value = target.value();
        for (j, parent) in self.parents.iter().enumerate() {
            if *parent != Some(i) {
                continue;
            }
            match self.loaded.get(j).and_then(|l| l.target.as_ref()) {
                None => {}
                Some(sub) if sub.commodity() == target.commodity() => {
                    value = value.saturating_sub(sub.value());
                }
                Some(_) => return (None, true),
            }
        }
        (Some(Amount::new(value, target.commodity().clone())), false)
    }

    /// Whether envelope `i`'s sub-budget targets exceed its own: the
    /// unallocated target is non-zero and opposes the envelope target's sign.
    fn over_allocated(&self, i: usize) -> bool {
        self.envelopes.contains(&i)
            && self
                .loaded
                .get(i)
                .and_then(|l| l.target.as_ref())
                .is_some_and(|envelope| {
                    self.unallocated_target(i).0.is_some_and(|rest| {
                        !rest.value().is_zero()
                            && rest.value().is_sign_negative()
                                != envelope.value().is_sign_negative()
                    })
                })
    }

    /// Verdict and ratio of `actual` against `target` paced through the window.
    fn judge(
        &self,
        intent: BudgetIntent,
        actual: Option<&Amount>,
        target: Option<&Amount>,
    ) -> (Option<Verdict>, Option<Decimal>) {
        let w = self.window;
        let j = bc_models::judge(intent, actual, target, w.start, w.end, w.today);
        (j.verdict, j.ratio)
    }

    /// A budget row: the budget's own inclusive figures.
    fn budget_row(&self, i: usize) -> Option<Built> {
        let l = self.loaded.get(i)?;
        let actual = l.actual();
        let own = self.own_unallocated(i);
        let (verdict, ratio) = self.judge(l.intent, actual.as_ref(), l.target.as_ref());
        let over_allocated = self.over_allocated(i);
        let item = BudgetTreeItem {
            id: l.budget.id().to_string(),
            kind: RowKind::Budget,
            account: l.account.clone(),
            budget: Some(l.budget.clone()),
            governing: l.governing.clone(),
            label: self.labels.get(i).cloned().unwrap_or_default(),
            tag_filter: l.tag_filter.clone(),
            claimed: actual
                .as_ref()
                .map_or(Decimal::ZERO, Amount::value)
                .saturating_sub(own),
            unallocated: own,
            unbudgeted: Decimal::ZERO,
            actual,
            target: l.target.clone(),
            intent: Some(l.intent),
            verdict,
            ratio,
            worst_descendant: None,
            mixed: false,
            double_counted: false,
            over_allocated,
            sign_flip: l.sign_flip,
            has_mixed_period: l.has_mixed_period,
            unvalued: l.unvalued.clone(),
            postings: l
                .postings
                .iter()
                .map(|p| RowPosting {
                    key: p.key.clone(),
                    bucket: self.owner_label(&p.key, i),
                    value: valued(p.value, l.commodity.as_ref()),
                    amount: p.amount.clone(),
                })
                .collect(),
            children: Vec::new(),
        };
        Some(Built {
            item,
            outer: vec![Outer {
                intent: l.intent,
                target: l.target.clone(),
            }],
            actual_mixed: false,
        })
    }

    /// Envelope `i`'s `↳ unallocated` row: the postings it owns outright.
    ///
    /// An over-allocated envelope's row is red whatever its spend: its
    /// target has the wrong sign, so any ratio against it misleads. A future
    /// window still has no verdict.
    fn unallocated_row(&self, i: usize) -> Option<Built> {
        let l = self.loaded.get(i)?;
        let own = self.own_unallocated(i);
        let actual = l.commodity.clone().map(|c| Amount::new(own, c));
        let (target, mixed) = self.unallocated_target(i);
        let (judged, ratio) = self.judge(l.intent, actual.as_ref(), target.as_ref());
        let started = self.window.today >= self.window.start;
        let verdict = if started && self.over_allocated(i) {
            Some(Verdict::Bad)
        } else {
            judged
        };
        let mut unvalued = bc_models::Balances::new();
        for p in self.owned(i).filter(|p| p.value.is_none()) {
            if let Err(e) = unvalued.try_add(&p.amount) {
                tracing::warn!(budget = %l.budget.id(), error = %e, "unallocated unvalued overflow");
            }
        }
        let item = BudgetTreeItem {
            id: format!("unalloc:{}", l.budget.id()),
            kind: RowKind::Unallocated,
            account: l.account.clone(),
            budget: Some(l.budget.clone()),
            governing: None,
            label: UNALLOCATED_LABEL.to_owned(),
            tag_filter: None,
            actual,
            target,
            intent: Some(l.intent),
            claimed: Decimal::ZERO,
            unallocated: own,
            unbudgeted: Decimal::ZERO,
            verdict,
            ratio,
            worst_descendant: None,
            mixed,
            double_counted: false,
            over_allocated: false,
            sign_flip: false,
            has_mixed_period: false,
            unvalued,
            postings: self
                .owned(i)
                .map(|p| RowPosting {
                    key: p.key.clone(),
                    bucket: None,
                    value: valued(p.value, l.commodity.as_ref()),
                    amount: p.amount.clone(),
                })
                .collect(),
            children: Vec::new(),
        };
        Some(Built {
            item,
            outer: Vec::new(),
            actual_mixed: false,
        })
    }

    /// An `↳ unbudgeted` row: postings under its parent no budget matches.
    fn unbudgeted_row(&self, draft: &Draft, row: usize) -> Option<Built> {
        let account = self.row_account(draft)?;
        let total: Decimal = draft.unmatched.iter().filter_map(|p| p.value).sum();
        let mut unvalued = bc_models::Balances::new();
        for p in draft.unmatched.iter().filter(|p| p.value.is_none()) {
            if let Err(e) = unvalued.try_add(&p.amount) {
                tracing::warn!(row, error = %e, "unbudgeted unvalued overflow");
            }
        }
        let item = BudgetTreeItem {
            id: format!("unbud:{}", draft.account),
            kind: RowKind::Unbudgeted,
            account,
            budget: None,
            governing: None,
            label: UNBUDGETED_LABEL.to_owned(),
            tag_filter: None,
            actual: draft.commodity.clone().map(|c| Amount::new(total, c)),
            target: None,
            intent: None,
            claimed: Decimal::ZERO,
            unallocated: Decimal::ZERO,
            unbudgeted: total,
            verdict: None,
            ratio: None,
            worst_descendant: None,
            mixed: false,
            double_counted: false,
            over_allocated: false,
            sign_flip: false,
            has_mixed_period: false,
            unvalued,
            postings: draft
                .unmatched
                .iter()
                .map(|p| RowPosting {
                    key: p.key.clone(),
                    bucket: None,
                    value: valued(p.value, draft.commodity.as_ref()),
                    amount: p.amount.clone(),
                })
                .collect(),
            children: Vec::new(),
        };
        Some(Built {
            item,
            outer: Vec::new(),
            actual_mixed: false,
        })
    }

    /// An account row: sums its children, and takes a target and verdict
    /// when the outermost budgets beneath agree on intent, target sign and
    /// commodity.
    fn account_row(&self, draft: &Draft, children: &[Built]) -> Option<Built> {
        let account = self.row_account(draft)?;
        let mut actual_mixed = children.iter().any(|c| c.actual_mixed);
        let mut commodity: Option<&CommodityCode> = None;
        let mut total = Decimal::ZERO;
        let (mut claimed, mut unallocated, mut unbudgeted) =
            (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
        let mut unvalued = bc_models::Balances::new();
        let mut postings: Vec<RowPosting> = Vec::new();
        let mut seen: HashSet<&PostingKey> = HashSet::new();
        for c in children {
            if let Some(a) = &c.item.actual {
                match commodity {
                    None => commodity = Some(a.commodity()),
                    Some(k) if k == a.commodity() => {}
                    Some(_) => actual_mixed = true,
                }
                total = total.saturating_add(a.value());
            }
            claimed = claimed.saturating_add(c.item.claimed);
            unallocated = unallocated.saturating_add(c.item.unallocated);
            unbudgeted = unbudgeted.saturating_add(c.item.unbudgeted);
            for (code, value) in c.item.unvalued.iter() {
                if let Err(e) = unvalued.try_add(&Amount::new(value, code)) {
                    tracing::warn!(error = %e, "account row unvalued overflow");
                }
            }
            for p in &c.item.postings {
                if seen.insert(&p.key) {
                    // An envelope's own postings name the envelope here, as
                    // several envelopes can sit under one account row.
                    let bucket = p
                        .bucket
                        .as_deref()
                        .filter(|l| *l != UNALLOCATED_LABEL)
                        .map_or_else(|| c.item.label.clone(), ToOwned::to_owned);
                    postings.push(RowPosting {
                        bucket: Some(bucket),
                        ..p.clone()
                    });
                }
            }
        }
        let actual = if actual_mixed {
            None
        } else {
            commodity.map(|c| Amount::new(total, c.clone()))
        };

        let outer: Vec<Outer> = children.iter().flat_map(|c| c.outer.clone()).collect();
        let agreed = outer
            .first()
            .map(Outer::agreement)
            .filter(|first| outer.iter().all(|o| o.agreement() == *first));
        let (target, verdict, ratio, disagree) = match agreed {
            Some((intent, Some((_, code)))) => {
                let sum: Decimal = outer
                    .iter()
                    .filter_map(|o| o.target.as_ref().map(Amount::value))
                    .sum();
                let target = Amount::new(sum, code);
                let (verdict, ratio) = self.judge(intent, actual.as_ref(), Some(&target));
                (Some(target), verdict, ratio, false)
            }
            Some((_, None)) => (None, None, None, false),
            None => (None, None, None, !outer.is_empty()),
        };

        let item = BudgetTreeItem {
            id: format!("acct:{}", draft.account),
            kind: RowKind::Account,
            label: account.name().to_owned(),
            account,
            budget: None,
            governing: None,
            tag_filter: None,
            actual,
            target,
            intent: None,
            claimed,
            unallocated,
            unbudgeted,
            verdict,
            ratio,
            worst_descendant: None,
            mixed: actual_mixed || disagree,
            double_counted: false,
            over_allocated: false,
            sign_flip: false,
            has_mixed_period: false,
            unvalued,
            postings,
            children: Vec::new(),
        };
        Some(Built {
            item,
            outer,
            actual_mixed,
        })
    }

    /// The account a row is anchored to.
    fn row_account(&self, draft: &Draft) -> Option<bc_models::Account> {
        self.accounts.get(&draft.account).cloned()
    }
}

// MARK: Summary

/// Counts verdicts over budget and unallocated rows, and totals each type
/// root's unbudgeted postings.
fn summarise(
    nodes: &[BudgetTreeItem],
    unmatched: &[Unmatched],
    accounts: &HashMap<AccountId, bc_models::Account>,
) -> BudgetTreeSummary {
    /// Folds one row and its descendants into the summary.
    fn visit(item: &BudgetTreeItem, summary: &mut BudgetTreeSummary) {
        summary.has_unvalued |= !item.unvalued.is_empty();
        if matches!(item.kind, RowKind::Budget | RowKind::Unallocated) {
            let count = match item.verdict {
                Some(Verdict::Bad) => Some(&mut summary.red),
                Some(Verdict::Warn) => Some(&mut summary.warn),
                Some(Verdict::Good) => Some(&mut summary.green),
                _ => None,
            };
            if let Some(n) = count {
                *n = n.saturating_add(1);
            }
        }
        for child in &item.children {
            visit(child, summary);
        }
    }

    let mut summary = BudgetTreeSummary {
        red: 0,
        warn: 0,
        green: 0,
        unbudgeted: Vec::new(),
        has_unvalued: false,
    };
    for node in nodes {
        visit(node, &mut summary);
    }
    for group in unmatched {
        let total: Decimal = group.postings.iter().filter_map(|p| p.value).sum();
        let (Some(commodity), Some(root)) = (&group.commodity, accounts.get(&group.root)) else {
            continue;
        };
        if !total.is_zero() {
            summary.unbudgeted.push((
                group.root.clone(),
                root.name().to_owned(),
                Amount::new(total, commodity.clone()),
            ));
        }
    }
    summary.unbudgeted.sort_by(|a, b| a.1.cmp(&b.1));
    summary
}

/// `value` in `commodity`, when both are known.
fn valued(value: Option<Decimal>, commodity: Option<&CommodityCode>) -> Option<Amount> {
    value.zip(commodity).map(|(v, c)| Amount::new(v, c.clone()))
}

/// Finds the row with `id` anywhere in `nodes`.
fn find_row<'a>(nodes: &'a [BudgetTreeItem], id: &str) -> Option<&'a BudgetTreeItem> {
    nodes.iter().find_map(|n| {
        if n.id == id {
            Some(n)
        } else {
            find_row(&n.children, id)
        }
    })
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;
    use std::fmt::Write as _;

    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::BudgetIntent;
    use bc_models::CommodityCode;
    use bc_models::Decimal;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::RolloverPolicy;
    use bc_models::TagId;
    use bc_models::TagPath;
    use bc_models::Transaction;
    use bc_models::Verdict;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    use super::BudgetOverview;
    use super::BudgetTreeItem;
    use super::BudgetTreeService;
    use super::RowKind;
    use crate::account::Service as AccountService;
    use crate::budget::BudgetService;
    use crate::budget::BudgetStatusEngine;
    use crate::fx::noop_fx;
    use crate::search::AmountQuery;
    use crate::search::TransactionQuery;
    use crate::tag::Service as TagService;
    use crate::transaction::Service as TransactionService;

    /// Start of the June display window the ported tests use.
    const JUNE: Date = Date::constant(2026, 6, 1);
    /// A `today` after June has closed.
    const JUNE_CLOSED: Date = Date::constant(2026, 7, 1);
    /// Start of the September display window.
    const SEPTEMBER: Date = Date::constant(2026, 9, 1);
    /// A `today` after September has closed.
    const SEPTEMBER_CLOSED: Date = Date::constant(2026, 10, 2);
    /// Label of an unallocated row.
    const UNALLOCATED: &str = "↳ unallocated";
    /// Label of an unbudgeted row.
    const UNBUDGETED: &str = "↳ unbudgeted";

    fn aud(value: Decimal) -> Amount {
        Amount::new(value, CommodityCode::new("AUD"))
    }

    // MARK: Fixture

    /// Fake accounts, tags, budgets and September transactions.
    struct Ledger {
        pool: SqlitePool,
        accounts: HashMap<String, AccountId>,
        tags: HashMap<String, TagId>,
        next: u32,
    }

    impl Ledger {
        /// A ledger holding only `Assets:Bank:Everyday`, the counter account.
        async fn new(pool: &SqlitePool) -> Self {
            let mut ledger = Self {
                pool: pool.clone(),
                accounts: HashMap::new(),
                tags: HashMap::new(),
                next: 0,
            };
            ledger.account("Assets:Bank:Everyday").await;
            ledger
        }

        /// The account at `path`, creating any missing segment. The root
        /// segment picks the type: `Income`, `Assets`, else `Expense`.
        async fn account(&mut self, path: &str) -> AccountId {
            let account_type = match path.split(':').next() {
                Some("Income") => AccountType::Income,
                Some("Assets") => AccountType::Asset,
                _ => AccountType::Expense,
            };
            let mut parent: Option<AccountId> = None;
            let mut prefix = String::new();
            for segment in path.split(':') {
                if !prefix.is_empty() {
                    prefix.push(':');
                }
                prefix.push_str(segment);
                let id = if let Some(existing) = self.accounts.get(&prefix) {
                    existing.clone()
                } else {
                    let created = AccountService::new(self.pool.clone())
                        .create()
                        .name(segment)
                        .account_type(account_type)
                        .kind(AccountKind::DepositAccount)
                        .maybe_parent_id(parent.as_ref())
                        .call()
                        .await
                        .expect("create account");
                    self.accounts.insert(prefix.clone(), created.clone());
                    created
                };
                parent = Some(id);
            }
            parent.expect("non-empty account path")
        }

        /// The tag at `path`, creating it if missing.
        async fn tag(&mut self, path: &str) -> TagId {
            if let Some(existing) = self.tags.get(path) {
                return existing.clone();
            }
            let id = TagService::new(self.pool.clone())
                .create_path(&path.parse::<TagPath>().expect("tag path"))
                .await
                .expect("create tag");
            self.tags.insert(path.to_owned(), id.clone());
            id
        }

        /// A monthly budget from 2026-01-01; returns its id.
        async fn budget(
            &mut self,
            path: &str,
            tag: Option<&str>,
            target: Option<Amount>,
            intent: BudgetIntent,
        ) -> String {
            let account = self.account(path).await;
            let tag_filter = match tag {
                Some(t) => Some(self.tag(t).await),
                None => None,
            };
            let (budget, _) = BudgetService::new(self.pool.clone())
                .create()
                .account_id(account)
                .effective_from(Date::constant(2026, 1, 1))
                .maybe_tag_filter(tag_filter)
                .maybe_target(target)
                .period(Period::Monthly)
                .rollover(RolloverPolicy::ResetToZero)
                .intent(intent)
                .call()
                .await
                .expect("create budget")
                .value;
            budget.id().to_string()
        }

        /// A Limit budget of `target` AUD.
        async fn limit(&mut self, path: &str, tag: Option<&str>, target: Decimal) -> String {
            self.budget(path, tag, Some(aud(target)), BudgetIntent::Limit)
                .await
        }

        /// Posts `value` AUD to `path` against the everyday account on
        /// 2026-09-10, tagging the posting with `tags`; returns its id.
        async fn post(&mut self, path: &str, value: Decimal, tags: &[&str]) -> String {
            self.post_in(path, value, "AUD", tags).await
        }

        /// As [`Self::post`], in `commodity`.
        async fn post_in(
            &mut self,
            path: &str,
            value: Decimal,
            commodity: &str,
            tags: &[&str],
        ) -> String {
            self.post_dated(path, value, commodity, tags, "2026-09-10")
                .await
        }

        /// As [`Self::post_in`], dated `date` (`YYYY-MM-DD`).
        async fn post_dated(
            &mut self,
            path: &str,
            value: Decimal,
            commodity: &str,
            tags: &[&str],
            date: &str,
        ) -> String {
            let account = self.account(path).await;
            let counter = self.account("Assets:Bank:Everyday").await;
            self.next = self.next.saturating_add(1);
            // Valid TypeIDs, so the query engine can hydrate these rows, and
            // deterministic, so ids sort in creation order.
            let tx = format!("transaction_{:026}", self.next);
            let posting = format!("posting_{:025}p", self.next);
            let counter_posting = format!("posting_{:025}c", self.next);
            sqlx::query(
                "INSERT INTO transactions (id, date, description, reconciliation, created_at) \
                 VALUES (?, ?, 'fixture', 'unreconciled', '2026-01-01T00:00:00Z')",
            )
            .bind(&tx)
            .bind(date)
            .execute(&self.pool)
            .await
            .expect("insert transaction");
            let legs = [
                (&posting, &account, value),
                (
                    &counter_posting,
                    &counter,
                    Decimal::ZERO.saturating_sub(value),
                ),
            ];
            for (position, (id, account_id, amount)) in legs.into_iter().enumerate() {
                sqlx::query(
                    "INSERT INTO postings \
                     (id, transaction_id, account_id, amount, commodity, position) \
                     VALUES (?, ?, ?, ?, ?, ?)",
                )
                .bind(id)
                .bind(&tx)
                .bind(account_id.to_string())
                .bind(amount.to_string())
                .bind(commodity)
                .bind(i64::try_from(position).expect("position fits i64"))
                .execute(&self.pool)
                .await
                .expect("insert posting");
            }
            for tag_path in tags {
                let tag = self.tag(tag_path).await;
                sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
                    .bind(&posting)
                    .bind(tag.to_string())
                    .execute(&self.pool)
                    .await
                    .expect("insert posting tag");
            }
            posting
        }

        /// The September overview as of `today`.
        async fn overview(&self, query: Option<&TransactionQuery>, today: Date) -> BudgetOverview {
            BudgetTreeService::new(self.pool.clone(), noop_fx())
                .get_overview(&Period::Monthly, SEPTEMBER, query, today)
                .await
                .expect("overview")
        }
    }

    // MARK: Walking

    /// Every row, depth first.
    fn every(nodes: &[BudgetTreeItem]) -> Vec<&BudgetTreeItem> {
        nodes
            .iter()
            .flat_map(|n| core::iter::once(n).chain(every(&n.children)))
            .collect()
    }

    /// The first row labelled `label`, depth first.
    fn find<'a>(nodes: &'a [BudgetTreeItem], label: &str) -> &'a BudgetTreeItem {
        every(nodes)
            .into_iter()
            .find(|n| n.label == label)
            .unwrap_or_else(|| panic!("no row labelled {label}"))
    }

    /// The child of `node` labelled `label`.
    fn child<'a>(node: &'a BudgetTreeItem, label: &str) -> &'a BudgetTreeItem {
        node.children
            .iter()
            .find(|n| n.label == label)
            .unwrap_or_else(|| panic!("{} has no child {label}", node.label))
    }

    /// The labels of `node`'s children, in order.
    fn labels(node: &BudgetTreeItem) -> Vec<&str> {
        node.children.iter().map(|c| c.label.as_str()).collect()
    }

    /// Sum of the children's actuals, all in one commodity.
    fn children_total(node: &BudgetTreeItem) -> Amount {
        let total: Decimal = node
            .children
            .iter()
            .map(|c| c.actual.as_ref().expect("child actual").value())
            .sum();
        aud(total)
    }

    /// An amount as `612 AUD`, or `-` when absent.
    fn show(amount: Option<&Amount>) -> String {
        amount.map_or_else(
            || "-".to_owned(),
            |a| format!("{} {}", a.value().normalize(), a.commodity()),
        )
    }

    /// One line per row: indent, label, kind, actual, target, intent,
    /// verdict, and the raised flags.
    fn render(nodes: &[BudgetTreeItem]) -> String {
        #[expect(
            clippy::use_debug,
            reason = "enum Debug names are the stable text the snapshot pins"
        )]
        fn walk(nodes: &[BudgetTreeItem], depth: usize, out: &mut String) {
            for n in nodes {
                let flags: Vec<&str> = [
                    (n.mixed, "mixed"),
                    (n.double_counted, "double-counted"),
                    (n.over_allocated, "over-allocated"),
                    (n.sign_flip, "sign-flip"),
                ]
                .into_iter()
                .filter_map(|(on, name)| on.then_some(name))
                .collect();
                writeln!(
                    out,
                    "{}{} [{:?}] actual={} target={} intent={:?} verdict={:?} flags={:?}",
                    "  ".repeat(depth),
                    n.label,
                    n.kind,
                    show(n.actual.as_ref()),
                    show(n.target.as_ref()),
                    n.intent,
                    n.verdict,
                    flags,
                )
                .expect("write to string");
                walk(&n.children, depth.saturating_add(1), out);
            }
        }
        let mut out = String::new();
        walk(nodes, 0, &mut out);
        out
    }

    // MARK: Tree shape

    #[sqlx::test(migrations = "./migrations")]
    async fn children_sum_to_parent(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(800)).await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(500))
            .await;
        ledger.limit("Expenses:Food:Dining", None, dec!(200)).await;
        let groceries_posting = ledger.post("Expenses:Food:Groceries", dec!(431), &[]).await;
        let dining_posting = ledger.post("Expenses:Food:Dining", dec!(150), &[]).await;
        let snacks_posting = ledger.post("Expenses:Food:Snacks", dec!(31), &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(food.kind, RowKind::Budget);
        assert_eq!(food.actual, Some(aud(dec!(612))));
        assert_eq!(labels(food), vec!["Dining", "Groceries", UNALLOCATED]);
        let unallocated = child(food, UNALLOCATED);
        assert_eq!(unallocated.kind, RowKind::Unallocated);
        assert_eq!(unallocated.actual, Some(aud(dec!(31))));
        assert_eq!(unallocated.target, Some(aud(dec!(100))));
        assert_eq!(unallocated.intent, Some(BudgetIntent::Limit));
        assert_eq!(children_total(food), aud(dec!(612)));
        assert_eq!(food.claimed, dec!(581));
        assert_eq!(food.unallocated, dec!(31));

        // Every envelope posting names its bucket, its own as unallocated.
        let mut buckets: Vec<(String, Option<String>, bool)> =
            BudgetTreeService::new(pool.clone(), noop_fx())
                .row_postings(
                    &food.id,
                    &Period::Monthly,
                    SEPTEMBER,
                    None,
                    SEPTEMBER_CLOSED,
                )
                .await
                .expect("row postings")
                .into_iter()
                .map(|(p, shared)| (p.key.posting_id, p.bucket, shared))
                .collect();
        buckets.sort();
        let mut expected = vec![
            (dining_posting, Some("Dining".to_owned()), false),
            (groceries_posting, Some("Groceries".to_owned()), false),
            (snacks_posting, Some(UNALLOCATED.to_owned()), false),
        ];
        expected.sort();
        assert_eq!(buckets, expected);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn tag_filter_pairs_id_and_path_without_a_governing_revision(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        let account = ledger.account("Expenses:Haircuts").await;
        let tag = ledger.tag("person:a").await;
        // The only revision starts after the window, so none governs it.
        BudgetService::new(pool.clone())
            .create()
            .account_id(account)
            .effective_from(Date::constant(2027, 1, 1))
            .tag_filter(tag.clone())
            .target(aud(dec!(30)))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget");

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let row = every(&overview.nodes)
            .into_iter()
            .find(|n| n.kind == RowKind::Budget)
            .expect("budget row");
        assert!(row.governing.is_none());
        assert_eq!(row.tag_filter, Some((tag, "person:a".to_owned())));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filtered_envelope_yields_both_leftovers(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        let envelope_id = ledger
            .limit("Expenses:Food", Some("household"), dec!(800))
            .await;
        ledger
            .limit("Expenses:Food:Groceries", Some("household"), dec!(500))
            .await;
        ledger
            .post("Expenses:Food:Groceries", dec!(100), &["household"])
            .await;
        let dining = ledger
            .post("Expenses:Food:Dining", dec!(40), &["household"])
            .await;
        ledger.post("Expenses:Food:Snacks", dec!(25), &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(food.kind, RowKind::Account);
        assert_eq!(labels(food), vec!["household", UNBUDGETED]);

        // The envelope labels its own posting unallocated; the account row
        // above names the envelope instead.
        let bucket_of = |row: &BudgetTreeItem| {
            row.postings
                .iter()
                .find(|p| p.key.posting_id == dining)
                .and_then(|p| p.bucket.clone())
        };
        assert_eq!(
            bucket_of(child(food, "household")),
            Some(UNALLOCATED.to_owned())
        );
        assert_eq!(bucket_of(food), Some("household".to_owned()));

        let envelope = child(food, "household");
        assert_eq!(envelope.id, envelope_id);
        assert_eq!(envelope.kind, RowKind::Budget);
        assert_eq!(
            envelope.tag_filter.as_ref().map(|(_, path)| path.as_str()),
            Some("household")
        );
        assert_eq!(envelope.actual, Some(aud(dec!(140))));
        assert_eq!(labels(envelope), vec!["Groceries", UNALLOCATED]);
        let groceries = child(envelope, "Groceries");
        assert_eq!(groceries.account.name(), "Groceries");
        assert_eq!(groceries.actual, Some(aud(dec!(100))));
        assert_eq!(child(envelope, UNALLOCATED).actual, Some(aud(dec!(40))));

        let unbudgeted = child(food, UNBUDGETED);
        assert_eq!(unbudgeted.actual, Some(aud(dec!(25))));
        assert_eq!(unbudgeted.target, None);
        assert_eq!(food.actual, Some(aud(dec!(165))));
        assert_eq!(children_total(food), aud(dec!(165)));

        assert!(
            every(&overview.nodes)
                .iter()
                .all(|n| !(n.kind == RowKind::Account && n.label == "Groceries")),
            "the Groceries account row is pruned"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn overlapping_pair_sharing_a_sub_budget_is_double_counted(pool: SqlitePool) {
        // `Food #household` and `Groceries` overlap; `Groceries #household`
        // nests under `Groceries`, so a `#household` grocery posting counts
        // in both `Food` children.
        let mut ledger = Ledger::new(&pool).await;
        ledger
            .limit("Expenses:Food", Some("household"), dec!(800))
            .await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(500))
            .await;
        ledger
            .limit("Expenses:Food:Groceries", Some("household"), dec!(300))
            .await;
        let posting = ledger
            .post("Expenses:Food:Groceries", dec!(50), &["household"])
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(food.kind, RowKind::Account);
        assert_eq!(labels(food), vec!["Groceries", "household"]);
        assert!(food.double_counted, "{}", render(&overview.nodes));
        assert_eq!(food.actual, Some(aud(dec!(100))));
        assert!(!find(&overview.nodes, "Expenses").double_counted);
        let groceries = child(food, "Groceries");
        assert!(
            !groceries.double_counted,
            "its children nest, so it sums once"
        );

        let flagged = BudgetTreeService::new(pool.clone(), noop_fx())
            .row_postings(
                &food.id,
                &Period::Monthly,
                SEPTEMBER,
                None,
                SEPTEMBER_CLOSED,
            )
            .await
            .expect("row postings");
        let flags: Vec<(String, bool)> = flagged
            .into_iter()
            .map(|(p, double_counted)| (p.key.posting_id, double_counted))
            .collect();
        assert_eq!(flags, vec![(posting, true)]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn labels_are_relative_to_the_parent_row(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(800)).await;
        ledger
            .limit("Expenses:Food:Groceries:Fruit", None, dec!(100))
            .await;
        ledger
            .limit("Expenses:Food:Dining", Some("household"), dec!(200))
            .await;
        ledger
            .limit("Expenses:Haircuts", Some("person"), dec!(90))
            .await;
        ledger
            .limit("Expenses:Haircuts", Some("person:a"), dec!(30))
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(
            labels(food),
            vec!["Dining #household", "Groceries:Fruit", UNALLOCATED]
        );
        let haircuts = find(&overview.nodes, "Haircuts");
        assert_eq!(labels(haircuts), vec!["person"]);
        let person = child(haircuts, "person");
        assert_eq!(labels(person), vec!["a", UNALLOCATED]);
        assert_eq!(
            child(person, "a")
                .tag_filter
                .as_ref()
                .map(|(_, path)| path.as_str()),
            Some("person:a")
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sibling_tags_flag_double_counting(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger
            .limit("Expenses:Haircuts", Some("person:a"), dec!(30))
            .await;
        ledger
            .limit("Expenses:Haircuts", Some("person:b"), dec!(60))
            .await;
        let shared = ledger
            .post("Expenses:Haircuts", dec!(20), &["person:a", "person:b"])
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let haircuts = find(&overview.nodes, "Haircuts");
        assert_eq!(haircuts.kind, RowKind::Account);
        assert!(haircuts.double_counted);
        assert_eq!(child(haircuts, "person:a").actual, Some(aud(dec!(20))));
        assert_eq!(child(haircuts, "person:b").actual, Some(aud(dec!(20))));
        assert!(!find(&overview.nodes, "Expenses").double_counted);

        let flagged = BudgetTreeService::new(pool.clone(), noop_fx())
            .row_postings(
                &haircuts.id,
                &Period::Monthly,
                SEPTEMBER,
                None,
                SEPTEMBER_CLOSED,
            )
            .await
            .expect("row postings");
        assert_eq!(flagged.len(), 1);
        let (posting, double_counted) = flagged.first().expect("one posting");
        assert_eq!(posting.key.posting_id, shared);
        assert!(*double_counted);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_budgets_on_one_account_both_appear(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        let a = ledger
            .limit("Expenses:Haircuts", Some("person:a"), dec!(30))
            .await;
        let b = ledger
            .limit("Expenses:Haircuts", Some("person:b"), dec!(60))
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let haircuts = find(&overview.nodes, "Haircuts");
        assert_eq!(labels(haircuts), vec!["person:a", "person:b"]);
        let ids: Vec<&str> = haircuts.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec![a.as_str(), b.as_str()]);
        assert_eq!(haircuts.target, Some(aud(dec!(90))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn assets_have_no_unbudgeted_row(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger
            .budget(
                "Assets:Bank:Savings",
                None,
                Some(aud(dec!(1000))),
                BudgetIntent::Goal,
            )
            .await;
        ledger.post("Assets:Bank:Savings", dec!(500), &[]).await;
        ledger.post("Expenses:Widgets", dec!(70), &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let roots: Vec<&str> = overview.nodes.iter().map(|n| n.label.as_str()).collect();
        assert_eq!(roots, vec!["Assets"]);
        assert!(
            every(&overview.nodes)
                .iter()
                .all(|n| n.kind != RowKind::Unbudgeted),
            "{}",
            render(&overview.nodes)
        );
        assert_eq!(
            find(&overview.nodes, "Savings").actual,
            Some(aud(dec!(500)))
        );
        assert_eq!(overview.summary.unbudgeted, []);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn over_allocated_envelope_is_flagged(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(500)).await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(400))
            .await;
        ledger.limit("Expenses:Food:Dining", None, dec!(200)).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert!(food.over_allocated);
        assert_eq!(child(food, UNALLOCATED).target, Some(aud(dec!(-100))));
        assert!(!find(&overview.nodes, "Groceries").over_allocated);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn over_allocation_follows_the_envelope_sign(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(-500)).await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(-400))
            .await;
        ledger.limit("Expenses:Food:Dining", None, dec!(-200)).await;
        ledger.limit("Expenses:Rent", None, dec!(-900)).await;
        ledger.limit("Expenses:Rent:Water", None, dec!(-100)).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(child(food, UNALLOCATED).target, Some(aud(dec!(100))));
        assert!(food.over_allocated);
        let rent = find(&overview.nodes, "Rent");
        assert_eq!(child(rent, UNALLOCATED).target, Some(aud(dec!(-800))));
        assert!(!rent.over_allocated);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn over_allocated_unallocated_row_is_red(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(500)).await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(400))
            .await;
        ledger.limit("Expenses:Food:Dining", None, dec!(200)).await;
        ledger.post("Expenses:Food:Snacks", dec!(10), &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        let unallocated = child(food, UNALLOCATED);
        assert_eq!(unallocated.target, Some(aud(dec!(-100))));
        assert_eq!(
            unallocated.verdict,
            Some(Verdict::Bad),
            "{}",
            render(&overview.nodes)
        );
        assert_eq!(food.worst_descendant, Some(Verdict::Bad));
        assert_eq!(
            (
                overview.summary.red,
                overview.summary.warn,
                overview.summary.green
            ),
            (1, 0, 3)
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn over_allocated_unallocated_row_has_no_verdict_in_a_future_window(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(500)).await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(400))
            .await;
        ledger.limit("Expenses:Food:Dining", None, dec!(200)).await;
        ledger.post("Expenses:Food:Snacks", dec!(10), &[]).await;

        let overview = ledger.overview(None, JUNE_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        let unallocated = child(food, UNALLOCATED);
        assert_eq!(unallocated.verdict, None, "{}", render(&overview.nodes));
        assert_eq!(overview.summary.red, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn spend_before_a_budget_starts_is_unbudgeted_at_the_account_row(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        let groceries = ledger.account("Expenses:Groceries").await;
        ledger.limit("Expenses:Rent", None, dec!(900)).await;
        BudgetService::new(pool.clone())
            .create()
            .account_id(groceries)
            .effective_from(Date::constant(2026, 9, 15))
            .target(aud(dec!(300)))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create budget");
        ledger
            .post_dated("Expenses:Groceries", dec!(40), "AUD", &[], "2026-09-10")
            .await;
        ledger
            .post_dated("Expenses:Groceries", dec!(60), "AUD", &[], "2026-09-20")
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let groceries_row = find(&overview.nodes, "Groceries");
        assert_eq!(groceries_row.kind, RowKind::Budget);
        assert_eq!(groceries_row.actual, Some(aud(dec!(60))));
        assert!(
            groceries_row.children.is_empty(),
            "{}",
            render(&overview.nodes)
        );
        let expenses = find(&overview.nodes, "Expenses");
        assert_eq!(child(expenses, UNBUDGETED).actual, Some(aud(dec!(40))));
        assert_eq!(expenses.actual, Some(aud(dec!(100))));
        assert_eq!(children_total(expenses), aud(dec!(100)));
    }

    // MARK: Global filter

    #[sqlx::test(migrations = "./migrations")]
    async fn global_filter_applies_to_unbudgeted(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(300)).await;
        ledger.post("Expenses:Gifts", dec!(15), &["keep"]).await;
        ledger.post("Expenses:Gifts", dec!(70), &[]).await;
        let keep = ledger.tag("keep").await;
        let query = TransactionQuery {
            tags: vec![keep],
            ..Default::default()
        };

        let overview = ledger.overview(Some(&query), SEPTEMBER_CLOSED).await;

        let expenses = find(&overview.nodes, "Expenses");
        assert_eq!(child(expenses, UNBUDGETED).actual, Some(aud(dec!(15))));
        let expenses_id = ledger.account("Expenses").await;
        assert_eq!(
            overview.summary.unbudgeted,
            vec![(expenses_id, "Expenses".to_owned(), aud(dec!(15)))]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn amount_filter_applies_to_unbudgeted(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(300)).await;
        let small_food = ledger.post("Expenses:Food", dec!(20), &[]).await;
        ledger.post("Expenses:Food", dec!(90), &[]).await;
        ledger.post("Expenses:Gifts", dec!(15), &[]).await;
        ledger.post("Expenses:Gifts", dec!(70), &[]).await;
        let query = TransactionQuery {
            amount: Some(AmountQuery {
                min: Some(dec!(50)),
                max: None,
                commodity: Some(CommodityCode::new("AUD")),
            }),
            ..Default::default()
        };

        let overview = ledger.overview(Some(&query), SEPTEMBER_CLOSED).await;

        assert_eq!(find(&overview.nodes, "Food").actual, Some(aud(dec!(90))));
        let expenses = find(&overview.nodes, "Expenses");
        assert_eq!(child(expenses, UNBUDGETED).actual, Some(aud(dec!(70))));
        assert!(
            every(&overview.nodes)
                .iter()
                .all(|n| n.postings.iter().all(|p| p.key.posting_id != small_food)),
            "the filtered-out posting appears in no row"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unvaluable_matched_posting_is_not_unbudgeted(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(300)).await;
        ledger.post_in("Expenses:Food", dec!(12), "XYZ", &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(food.unvalued.get("XYZ"), Some(dec!(12)));
        assert_eq!(food.actual, Some(aud(dec!(0))));
        assert!(
            every(&overview.nodes)
                .iter()
                .all(|n| n.kind != RowKind::Unbudgeted),
            "{}",
            render(&overview.nodes)
        );
        assert!(overview.summary.has_unvalued);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unallocated_row_reports_its_own_unvaluable_postings(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(300)).await;
        ledger.limit("Expenses:Food:Dining", None, dec!(200)).await;
        ledger
            .post_in("Expenses:Food:Snacks", dec!(12), "XYZ", &[])
            .await;
        ledger
            .post_in("Expenses:Food:Dining", dec!(5), "XYZ", &[])
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let food = find(&overview.nodes, "Food");
        assert_eq!(food.unvalued.get("XYZ"), Some(dec!(17)));
        let unallocated = child(food, UNALLOCATED);
        assert_eq!(unallocated.unvalued.get("XYZ"), Some(dec!(12)));
        assert_eq!(child(food, "Dining").unvalued.get("XYZ"), Some(dec!(5)));
    }

    // MARK: Verdicts

    #[sqlx::test(migrations = "./migrations")]
    async fn open_window_verdict_is_paced(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Groceries", None, dec!(300)).await;
        ledger.post("Expenses:Groceries", dec!(150), &[]).await;

        let month_end = ledger.overview(None, Date::constant(2026, 9, 30)).await;
        let at_month_end = find(&month_end.nodes, "Groceries");
        assert_eq!(at_month_end.verdict, Some(Verdict::Good));
        assert_eq!(at_month_end.ratio, Some(dec!(0.5)));

        let early = ledger.overview(None, Date::constant(2026, 9, 10)).await;
        let on_the_tenth = find(&early.nodes, "Groceries");
        assert_eq!(on_the_tenth.verdict, Some(Verdict::Bad));
        assert_eq!(on_the_tenth.ratio, Some(dec!(1.5)));
        assert_eq!(early.elapsed_fraction, dec!(10).checked_div(dec!(30)));
        assert_eq!(early.summary.red, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn aggregate_verdict_needs_agreement(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger
            .budget(
                "Income:Interest",
                None,
                Some(aud(dec!(-40))),
                BudgetIntent::Estimate,
            )
            .await;
        ledger
            .budget(
                "Income:Salary",
                None,
                Some(aud(dec!(-5000))),
                BudgetIntent::Goal,
            )
            .await;
        ledger.limit("Expenses:Food", None, dec!(300)).await;
        ledger.limit("Expenses:Rent", None, dec!(1000)).await;
        ledger.post("Expenses:Food", dec!(120), &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let income = find(&overview.nodes, "Income");
        assert_eq!(income.verdict, None);
        assert_eq!(income.target, None);
        assert!(income.mixed);
        let expenses = find(&overview.nodes, "Expenses");
        assert!(!expenses.mixed);
        assert_eq!(expenses.target, Some(aud(dec!(1300))));
        assert_eq!(expenses.actual, Some(aud(dec!(120))));
        assert_eq!(expenses.verdict, Some(Verdict::Good));

        ledger
            .budget(
                "Expenses:Travel",
                None,
                Some(Amount::new(dec!(100), CommodityCode::new("USD"))),
                BudgetIntent::Limit,
            )
            .await;
        let with_usd = ledger.overview(None, SEPTEMBER_CLOSED).await;
        let mixed_expenses = find(&with_usd.nodes, "Expenses");
        assert_eq!(mixed_expenses.actual, None);
        assert_eq!(mixed_expenses.verdict, None);
        assert!(mixed_expenses.mixed);
    }

    /// A budget row's postings carry their valued amounts, including spend on
    /// child accounts.
    #[sqlx::test(migrations = "./migrations")]
    async fn row_postings_carry_values_for_child_accounts(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        let budget = ledger.limit("Expenses:Food", None, dec!(200)).await;
        ledger.post("Expenses:Food:Groceries", dec!(40), &[]).await;

        let rows = BudgetTreeService::new(pool.clone(), noop_fx())
            .row_postings(&budget, &Period::Monthly, SEPTEMBER, None, SEPTEMBER_CLOSED)
            .await
            .expect("row postings");

        assert_eq!(rows.len(), 1);
        let (posting, _) = rows.first().expect("one posting");
        assert_eq!(posting.value, Some(aud(dec!(40))));
        assert_eq!(posting.amount, aud(dec!(40)));
    }

    /// An account row over children in two commodities leaves each posting's
    /// value in its child's commodity.
    #[sqlx::test(migrations = "./migrations")]
    async fn mixed_account_row_keeps_child_commodities(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(200)).await;
        ledger
            .budget(
                "Expenses:Travel",
                None,
                Some(Amount::new(dec!(200), CommodityCode::new("USD"))),
                BudgetIntent::Limit,
            )
            .await;
        ledger.post("Expenses:Food", dec!(40), &[]).await;
        ledger
            .post_in("Expenses:Travel", dec!(30), "USD", &[])
            .await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        let parent = find(&overview.nodes, "Expenses");
        assert_eq!(parent.actual, None);
        let mut codes: Vec<String> = parent
            .postings
            .iter()
            .filter_map(|p| p.value.as_ref().map(|v| v.commodity().as_str().to_owned()))
            .collect();
        codes.sort();
        assert_eq!(codes, vec!["AUD".to_owned(), "USD".to_owned()]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn overview_snapshot(pool: SqlitePool) {
        let mut ledger = Ledger::new(&pool).await;
        ledger.limit("Expenses:Food", None, dec!(800)).await;
        ledger
            .limit("Expenses:Food:Groceries", None, dec!(500))
            .await;
        ledger.limit("Expenses:Food:Dining", None, dec!(200)).await;
        ledger.limit("Expenses:Gifts", None, dec!(100)).await;
        ledger
            .limit("Expenses:Haircuts", Some("person:a"), dec!(30))
            .await;
        ledger
            .limit("Expenses:Haircuts", Some("person:b"), dec!(60))
            .await;
        ledger.limit("Expenses:Pets:Grooming", None, dec!(80)).await;
        ledger
            .budget(
                "Income:Interest",
                None,
                Some(aud(dec!(-40))),
                BudgetIntent::Estimate,
            )
            .await;
        ledger
            .budget(
                "Income:Salary",
                None,
                Some(aud(dec!(-5000))),
                BudgetIntent::Goal,
            )
            .await;

        ledger.post("Expenses:Food:Groceries", dec!(431), &[]).await;
        ledger.post("Expenses:Food:Dining", dec!(150), &[]).await;
        ledger.post("Expenses:Food:Snacks", dec!(31), &[]).await;
        ledger
            .post("Expenses:Haircuts", dec!(30), &["person:a"])
            .await;
        ledger
            .post("Expenses:Haircuts", dec!(45), &["person:b"])
            .await;
        ledger.post("Expenses:Haircuts", dec!(25), &[]).await;
        ledger.post("Expenses:Pets:Grooming", dec!(95), &[]).await;
        ledger.post("Expenses:Pets:Food", dec!(120), &[]).await;
        ledger.post("Income:Interest", dec!(-38), &[]).await;
        ledger.post("Income:Salary", dec!(-5000), &[]).await;

        let overview = ledger.overview(None, SEPTEMBER_CLOSED).await;

        assert_eq!(
            find(&overview.nodes, "Expenses").actual,
            Some(aud(dec!(927)))
        );
        assert_eq!(
            (
                overview.summary.red,
                overview.summary.warn,
                overview.summary.green
            ),
            (1, 2, 7)
        );
        insta::assert_snapshot!(render(&overview.nodes));
    }

    // MARK: Ported

    #[sqlx::test(migrations = "./migrations")]
    async fn single_monthly_budget_matches_actuals(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let restaurants = accounts
            .create()
            .name("Restaurants")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(restaurants.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(300), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("budget");

        let txns = TransactionService::new(pool.clone());
        txns.create(
            Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(Date::constant(2026, 6, 11))
                .description("Dinner")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(restaurants)
                        .amount(Amount::new(dec!(68), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking)
                        .amount(Amount::new(dec!(-68), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("create tx");

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, JUNE, None, JUNE_CLOSED)
            .await
            .expect("overview");

        assert_eq!(overview.nodes.len(), 1);
        let node = overview.nodes.first().expect("one node");
        assert_eq!(node.kind, RowKind::Budget);
        assert_eq!(node.actual, Some(aud(dec!(68))));
        assert_eq!(node.target, Some(aud(dec!(300))));
        assert_eq!(overview.summary.red, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn weekly_budget_in_monthly_view_pro_rates_target(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let gym = accounts
            .create()
            .name("Gym")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("gym");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(gym.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(30), CommodityCode::new("AUD")))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("budget");

        drop(checking);

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, JUNE, None, JUNE_CLOSED)
            .await
            .expect("overview");

        assert_eq!(overview.nodes.len(), 1);
        let node = overview.nodes.first().expect("one node");
        // June 2026 has 5 overlapping weeks (4 full + 1 partial of 2 days).
        // Effective target = 4 x $30 + (2/7 x $30) approx $128.57
        let target = node.target.as_ref().map(Amount::value).expect("has target");
        // Allow 1 cent tolerance for rounding.
        #[expect(
            clippy::arithmetic_side_effects,
            reason = "test arithmetic on bounded test values"
        )]
        let diff = (target - dec!(128.57)).abs();
        assert!(diff < dec!(0.01), "got {target}");
        assert!(node.has_mixed_period);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn nested_budget_appears_as_child_not_lost(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let food = accounts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food account");
        let restaurants = accounts
            .create()
            .name("Restaurants")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .parent_id(&food)
            .call()
            .await
            .expect("restaurants account");

        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(500), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("food budget");
        budgets
            .create()
            .account_id(restaurants.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(200), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("restaurant budget");

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, JUNE, None, JUNE_CLOSED)
            .await
            .expect("overview");

        // One root (Food) nesting Restaurants, beside Food's unallocated row.
        assert_eq!(overview.nodes.len(), 1, "expected one root node");
        let root = overview.nodes.first().expect("one node");
        assert_eq!(labels(root), vec!["Restaurants", "↳ unallocated"]);

        let restaurants_row = root.children.first().expect("one child");
        assert_eq!(restaurants_row.account.name(), "Restaurants");
        assert_eq!(restaurants_row.kind, RowKind::Budget);
        assert_eq!(restaurants_row.target, Some(aud(dec!(200))));
        assert_eq!(root.target, Some(aud(dec!(500))));
        assert_eq!(overview.summary.red, 0);

        drop(restaurants);
    }

    fn query_text(text: &str) -> crate::search::TransactionQuery {
        crate::search::TransactionQuery {
            text: Some(text.to_owned()),
            ..Default::default()
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_text_narrows_actuals(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let restaurants = accounts
            .create()
            .name("Restaurants")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(restaurants.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(300), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("budget");

        let txns = TransactionService::new(pool.clone());
        for (desc, amt) in [("Dinner at Cafe", dec!(40)), ("Groceries", dec!(60))] {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "negation of a bounded test amount"
            )]
            let neg_amt = -amt;
            txns.create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 6, 11))
                    .description(desc)
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(restaurants.clone())
                            .amount(Amount::new(amt, CommodityCode::new("AUD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(checking.clone())
                            .amount(Amount::new(neg_amt, CommodityCode::new("AUD")))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        }

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let q = query_text("cafe");
        let overview = svc
            .get_overview(&Period::Monthly, JUNE, Some(&q), JUNE_CLOSED)
            .await
            .expect("overview");
        let node = overview.nodes.first().expect("one node");
        // Only "Dinner at Cafe" (40) matches; "Groceries" (60) excluded.
        assert_eq!(node.actual, Some(aud(dec!(40))));

        // Empty query reproduces the unfiltered total (100).
        let unfiltered = svc
            .get_overview(&Period::Monthly, JUNE, None, JUNE_CLOSED)
            .await
            .expect("overview");
        let unfiltered_node = unfiltered.nodes.first().expect("one node");
        assert_eq!(unfiltered_node.actual, Some(aud(dec!(100))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn budget_tag_filter_flows_down_from_transaction(pool: sqlx::SqlitePool) {
        // A budget's own tag filter counts a posting when the *transaction*
        // carries the tag, even if that individual posting is untagged
        // (transaction tags flow down to every posting), and a descendant tag on
        // the posting counts via the tag subtree.
        let accounts = AccountService::new(pool.clone());
        let health = accounts
            .create()
            .name("Health")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("health");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");

        let wellness = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, 'wellness', ?)")
            .bind(wellness.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert wellness");
        let gym_tag = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, parent_id, created_at) VALUES (?, 'gym', ?, ?)")
            .bind(gym_tag.to_string())
            .bind(wellness.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert gym");

        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(health.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .tag_filter(wellness.clone())
            .target(Amount::new(dec!(300), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("budget");

        let txns = TransactionService::new(pool.clone());

        // (a) tag on the TRANSACTION, health posting untagged -> counts (40).
        txns.create(
            Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(Date::constant(2026, 6, 11))
                .description("Checkup")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(health.clone())
                        .amount(Amount::new(dec!(40), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-40), CommodityCode::new("AUD")))
                        .build(),
                ])
                .tag_ids(vec![wellness.clone()])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("tx a");

        // (b) descendant tag on the health POSTING -> counts via subtree (25).
        txns.create(
            Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(Date::constant(2026, 6, 12))
                .description("Gym")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(health.clone())
                        .amount(Amount::new(dec!(25), CommodityCode::new("AUD")))
                        .tag_ids(vec![gym_tag.clone()])
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-25), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("tx b");

        // (c) untagged transaction and untagged posting -> excluded (99).
        txns.create(
            Transaction::builder()
                .id(bc_models::TransactionId::new())
                .date(Date::constant(2026, 6, 13))
                .description("Snacks")
                .postings(vec![
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(health.clone())
                        .amount(Amount::new(dec!(99), CommodityCode::new("AUD")))
                        .build(),
                    Posting::builder()
                        .id(PostingId::new())
                        .account_id(checking.clone())
                        .amount(Amount::new(dec!(-99), CommodityCode::new("AUD")))
                        .build(),
                ])
                .reconciliation(Reconciliation::Reconciled)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("tx c");

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, JUNE, None, JUNE_CLOSED)
            .await
            .expect("overview");
        // A filtered budget sits under its account's row.
        let health_row = overview.nodes.first().expect("one node");
        assert_eq!(health_row.kind, RowKind::Account);
        let node = child(health_row, "wellness");
        // 40 (tx tag flows down) + 25 (subtree tag on posting) = 65; 99 excluded.
        assert_eq!(node.actual, Some(aud(dec!(65))));
        assert_eq!(child(health_row, UNBUDGETED).actual, Some(aud(dec!(99))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn filter_amount_is_commodity_exact(pool: sqlx::SqlitePool) {
        // A tracking-only budget (no target) whose account has one USD and one BTC posting.
        // `over:USD50` must count ONLY the USD posting; BTC is never magnitude-compared.
        let accounts = AccountService::new(pool.clone());
        let wallet = accounts
            .create()
            .name("Wallet")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("wallet");
        let source = accounts
            .create()
            .name("Source")
            .account_type(AccountType::Income)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("source");

        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(wallet.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("tracking budget");

        let txns = TransactionService::new(pool.clone());
        for (usd_amt, btc_amt) in [(dec!(100), dec!(60))] {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "negation of a bounded test amount"
            )]
            let neg_usd = -usd_amt;
            txns.create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 6, 5))
                    .description("Mixed")
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(wallet.clone())
                            .amount(Amount::new(usd_amt, CommodityCode::new("USD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(wallet.clone())
                            .amount(Amount::new(btc_amt, CommodityCode::new("BTC")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(source.clone())
                            .amount(Amount::new(neg_usd, CommodityCode::new("USD")))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        }

        let q = crate::search::TransactionQuery {
            amount: Some(crate::search::AmountQuery {
                min: Some(dec!(50)),
                max: None,
                commodity: Some(CommodityCode::new("USD")),
            }),
            ..Default::default()
        };
        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, JUNE, Some(&q), JUNE_CLOSED)
            .await
            .expect("overview");
        let node = overview.nodes.first().expect("one node");
        // Only the USD 100 posting survives; BTC 60 (>= min 50) is filtered out
        // on commodity, not magnitude.
        assert_eq!(
            node.actual,
            Some(Amount::new(dec!(100), CommodityCode::new("USD")))
        );
    }

    /// A Goal week that met its target reads Good; the week not yet started
    /// has no verdict; the open week is paced.
    #[sqlx::test(migrations = "./migrations")]
    async fn sub_rows_take_core_verdicts(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let savings = accounts
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("savings");
        let bank = accounts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(savings.clone())
            .effective_from(Date::constant(2026, 6, 1))
            .target(Amount::new(dec!(70), CommodityCode::new("AUD")))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Goal)
            .call()
            .await
            .expect("create")
            .value;
        TransactionService::new(pool.clone())
            .create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 6, 2))
                    .description("Transfer")
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(savings.clone())
                            .amount(Amount::new(dec!(70), CommodityCode::new("AUD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(bank.clone())
                            .amount(Amount::new(dec!(-70), CommodityCode::new("AUD")))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        let tree = BudgetTreeService::new(pool.clone(), noop_fx());
        let rows = tree
            .native_periods(
                &budget,
                Date::constant(2026, 6, 1),
                Date::constant(2026, 7, 1),
                None,
                Date::constant(2026, 6, 10),
            )
            .await
            .expect("native");
        let first = rows.first().expect("first week");
        assert_eq!(first.verdict, Some(Verdict::Good));
        let open = rows
            .iter()
            .find(|r| r.overlap.overlap_start == Date::constant(2026, 6, 8))
            .expect("open");
        // Day 3 of 7, nothing saved: paced reference 30, ratio 0, Bad for a Goal.
        assert_eq!(open.ratio, Some(dec!(0)));
        assert_eq!(open.verdict, Some(Verdict::Bad));
        let future = rows
            .iter()
            .find(|r| r.overlap.overlap_start == Date::constant(2026, 6, 15))
            .expect("future");
        assert_eq!(future.verdict, None);
    }

    /// Budgets `Food` monthly at 40.00 with `intent`, posts `moved` between
    /// `Food` and `Bank` on 2026-06-02, and returns the June row's
    /// `(verdict, ratio)` beside the sole sub-row's, both judged on 2026-06-10.
    async fn june_row_and_sub_row(
        pool: SqlitePool,
        intent: BudgetIntent,
        moved: Decimal,
    ) -> (
        (Option<Verdict>, Option<Decimal>),
        (Option<Verdict>, Option<Decimal>),
    ) {
        let today = Date::constant(2026, 6, 10);
        let june = Date::constant(2026, 6, 1);
        let accounts = AccountService::new(pool.clone());
        let food = accounts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let bank = accounts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food.clone())
            .effective_from(june)
            .target(Amount::new(dec!(40.00), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(intent)
            .call()
            .await
            .expect("create")
            .value;
        TransactionService::new(pool.clone())
            .create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 6, 2))
                    .description("Shop")
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(food.clone())
                            .amount(Amount::new(moved, CommodityCode::new("AUD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(bank.clone())
                            .amount(Amount::new(
                                Decimal::ZERO.checked_sub(moved).expect("negation"),
                                CommodityCode::new("AUD"),
                            ))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        let tree = BudgetTreeService::new(pool, noop_fx());
        let overview = tree
            .get_overview(&Period::Monthly, june, None, today)
            .await
            .expect("overview");
        let node = find(&overview.nodes, "Food");
        let rows = tree
            .native_periods(&budget, june, Date::constant(2026, 7, 1), None, today)
            .await
            .expect("native");
        let [sub] = rows.as_slice() else {
            panic!("June is exactly one monthly period, got {}", rows.len());
        };
        ((node.verdict, node.ratio), (sub.verdict, sub.ratio))
    }

    /// Asserts that a one-period window judges its sub-row as it judges the row.
    async fn assert_sub_row_matches_row(pool: SqlitePool, intent: BudgetIntent, moved: Decimal) {
        let (row, sub) = june_row_and_sub_row(pool, intent, moved).await;
        assert!(row.0.is_some(), "an open window is judged");
        assert_eq!(sub, row);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn single_period_sub_row_matches_row_limit_over_pace(pool: SqlitePool) {
        assert_sub_row_matches_row(pool, BudgetIntent::Limit, dec!(30.00)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn single_period_sub_row_matches_row_limit_under_pace(pool: SqlitePool) {
        assert_sub_row_matches_row(pool, BudgetIntent::Limit, dec!(5.00)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn single_period_sub_row_matches_row_goal_under_pace(pool: SqlitePool) {
        assert_sub_row_matches_row(pool, BudgetIntent::Goal, dec!(5.00)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn single_period_sub_row_matches_row_goal_met(pool: SqlitePool) {
        assert_sub_row_matches_row(pool, BudgetIntent::Goal, dec!(40.00)).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn native_periods_respect_filter(pool: sqlx::SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let gym = accounts
            .create()
            .name("Gym")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("gym");
        let checking = accounts
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("checking");
        let budgets = BudgetService::new(pool.clone());
        budgets
            .create()
            .account_id(gym.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(dec!(30), CommodityCode::new("AUD")))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("budget");
        let budget = BudgetService::new(pool.clone())
            .list()
            .await
            .expect("list")
            .into_iter()
            .next()
            .expect("one");

        let txns = TransactionService::new(pool.clone());
        for (desc, amt) in [("Membership", dec!(30)), ("Locker fee", dec!(5))] {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "negation of a bounded test amount"
            )]
            let neg_amt = -amt;
            txns.create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 6, 3))
                    .description(desc)
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(gym.clone())
                            .amount(Amount::new(amt, CommodityCode::new("AUD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(checking.clone())
                            .amount(Amount::new(neg_amt, CommodityCode::new("AUD")))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        }

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let q = query_text("membership");
        let rows = svc
            .native_periods(
                &budget,
                Date::constant(2026, 6, 1),
                Date::constant(2026, 7, 1),
                Some(&q),
                Date::constant(2026, 7, 2),
            )
            .await
            .expect("native");
        let total: bc_models::Decimal = rows.iter().map(|r| r.actuals).sum();
        assert_eq!(total, dec!(30)); // "Locker fee" (5) excluded.
    }

    /// Sub-row actuals sum to the row actual, and a tracking-only budget's
    /// sub-rows share the window's dominant commodity.
    #[sqlx::test(migrations = "./migrations")]
    async fn tracking_only_sub_rows_share_the_window_commodity(pool: SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let food = accounts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let bank = accounts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        let txns = TransactionService::new(pool.clone());
        for (day, amt, code) in [(2, dec!(90), "AUD"), (10, dec!(30), "USD")] {
            #[expect(
                clippy::arithmetic_side_effects,
                reason = "negation of a bounded test amount"
            )]
            let neg_amt = -amt;
            txns.create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 6, day))
                    .description("Shop")
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(food.clone())
                            .amount(Amount::new(amt, CommodityCode::new(code)))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(bank.clone())
                            .amount(Amount::new(neg_amt, CommodityCode::new(code)))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");
        }
        let tree = BudgetTreeService::new(pool.clone(), noop_fx());
        let rows = tree
            .native_periods(
                &budget,
                Date::constant(2026, 6, 1),
                Date::constant(2026, 7, 1),
                None,
                Date::constant(2026, 7, 2),
            )
            .await
            .expect("native");
        assert!(
            rows.iter()
                .all(|r| r.commodity == Some(CommodityCode::new("AUD")))
        );
        let total: bc_models::Decimal = rows.iter().map(|r| r.actuals).sum();
        assert_eq!(total, dec!(90));
        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(
                &budget,
                bc_models::BudgetWindow::custom(
                    Date::constant(2026, 6, 1),
                    Date::constant(2026, 7, 1),
                    "june".to_owned(),
                ),
                None,
            )
            .await
            .expect("status");
        assert_eq!(total, status.actuals);
        let usd: bc_models::Decimal = rows.iter().filter_map(|r| r.unvalued.get("USD")).sum();
        assert_eq!(usd, dec!(30));
    }

    /// A tracking-only first period with no postings still takes the commodity
    /// of the first later period that has a target.
    #[sqlx::test(migrations = "./migrations")]
    async fn later_target_commodity_labels_an_empty_tracking_only_start(pool: SqlitePool) {
        let food = AccountService::new(pool.clone())
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(food)
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2026, 6, 15))
                .target(aud(dec!(50)))
                .period(Period::Weekly)
                .rollover(RolloverPolicy::ResetToZero)
                .intent(BudgetIntent::Limit)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");

        let start = Date::constant(2026, 6, 1);
        let end = Date::constant(2026, 7, 1);
        let valuation = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .window_postings(
                &budget,
                &bc_models::BudgetWindow::custom(start, end, "june".to_owned()),
                None,
            )
            .await
            .expect("valuation");
        assert_eq!(valuation.commodity, Some(CommodityCode::new("AUD")));
        let rows = BudgetTreeService::new(pool.clone(), noop_fx())
            .native_periods(&budget, start, end, None, Date::constant(2026, 7, 2))
            .await
            .expect("native");
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|r| r.commodity == Some(CommodityCode::new("AUD")))
        );
    }

    /// A targeted budget with no postings still labels its sub-rows with the
    /// target's commodity.
    #[sqlx::test(migrations = "./migrations")]
    async fn targeted_budget_without_postings_labels_sub_rows(pool: SqlitePool) {
        let food = AccountService::new(pool.clone())
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food)
            .effective_from(Date::constant(2026, 1, 1))
            .target(aud(dec!(50)))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        let rows = BudgetTreeService::new(pool.clone(), noop_fx())
            .native_periods(
                &budget,
                Date::constant(2026, 6, 1),
                Date::constant(2026, 7, 1),
                None,
                Date::constant(2026, 7, 2),
            )
            .await
            .expect("native");
        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .all(|r| r.commodity == Some(CommodityCode::new("AUD")))
        );
    }

    /// A mid-month revision splits March into a stub and a fresh period; the
    /// two shares sum to one month's target.
    #[sqlx::test(migrations = "./migrations")]
    async fn mid_month_revision_targets_one_month(pool: SqlitePool) {
        let food = AccountService::new(pool.clone())
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let svc = BudgetService::new(pool.clone());
        let (budget, _) = svc
            .create()
            .account_id(food)
            .effective_from(Date::constant(2026, 1, 1))
            .target(aud(dec!(200)))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        svc.revise(
            budget.id(),
            bc_models::BudgetRevision::builder()
                .budget_id(budget.id().clone())
                .effective_from(Date::constant(2026, 3, 16))
                .target(aud(dec!(200)))
                .period(Period::Monthly)
                .rollover(RolloverPolicy::ResetToZero)
                .intent(BudgetIntent::Limit)
                .created_at(Timestamp::now())
                .build(),
        )
        .await
        .expect("revise");

        let tree = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = tree
            .get_overview(
                &Period::Monthly,
                Date::constant(2026, 3, 1),
                None,
                Date::constant(2026, 4, 2),
            )
            .await
            .expect("overview");
        let food_row = find(&overview.nodes, "Food");
        // 200 x 15/31 = 96.77 for the stub, 200 x 16/31 = 103.23 for the rest.
        assert_eq!(food_row.target, Some(aud(dec!(200.00))));

        let native = tree
            .native_periods(
                &budget,
                Date::constant(2026, 3, 1),
                Date::constant(2026, 4, 1),
                None,
                Date::constant(2026, 7, 2),
            )
            .await
            .expect("native");
        let targets: Vec<_> = native.iter().map(|n| n.effective_target).collect();
        assert_eq!(targets, vec![Some(dec!(96.77)), Some(dec!(103.23))]);
    }

    /// Spend the carry chain could not value shows on the window's status
    /// alone; no sub-row repeats it.
    #[sqlx::test(migrations = "./migrations")]
    async fn carry_chain_unvalued_stays_off_the_sub_rows(pool: SqlitePool) {
        let accounts = AccountService::new(pool.clone());
        let food = accounts
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("food");
        let bank = accounts
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("bank");
        let (budget, _) = BudgetService::new(pool.clone())
            .create()
            .account_id(food.clone())
            .effective_from(Date::constant(2026, 1, 1))
            .target(aud(dec!(50)))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::CarryForward)
            .intent(BudgetIntent::Limit)
            .call()
            .await
            .expect("create")
            .value;
        // May sits in the carry chain behind June; no FX rate values its USD.
        TransactionService::new(pool.clone())
            .create(
                Transaction::builder()
                    .id(bc_models::TransactionId::new())
                    .date(Date::constant(2026, 5, 20))
                    .description("Shop")
                    .postings(vec![
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(food.clone())
                            .amount(Amount::new(dec!(30), CommodityCode::new("USD")))
                            .build(),
                        Posting::builder()
                            .id(PostingId::new())
                            .account_id(bank.clone())
                            .amount(Amount::new(dec!(-30), CommodityCode::new("USD")))
                            .build(),
                    ])
                    .reconciliation(Reconciliation::Reconciled)
                    .created_at(jiff::Timestamp::now())
                    .build(),
            )
            .await
            .expect("tx");

        let start = Date::constant(2026, 6, 1);
        let end = Date::constant(2026, 7, 1);
        let status = BudgetStatusEngine::new(pool.clone(), noop_fx())
            .status_for_window(
                &budget,
                bc_models::BudgetWindow::custom(start, end, "june".to_owned()),
                None,
            )
            .await
            .expect("status");
        assert_eq!(status.unvalued.get("USD"), Some(dec!(30)));
        let rows = BudgetTreeService::new(pool.clone(), noop_fx())
            .native_periods(&budget, start, end, None, Date::constant(2026, 7, 2))
            .await
            .expect("native");
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.unvalued.is_empty()));
    }
}
