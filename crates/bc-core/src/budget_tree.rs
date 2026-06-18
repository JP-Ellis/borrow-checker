//! Budget tree assembly: builds per-display-window status trees from active budgets.

use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;

use bc_models::Amount;
use bc_models::Decimal;
use bc_models::Period;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::budget::BudgetService;
use crate::budget::BudgetStatusEngine;
use crate::period_overlap::overlapping_periods;

// MARK: BudgetTreeItem

/// Computed status for one budget in one display window.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BudgetTreeItem {
    /// The budget this item represents.
    pub budget: bc_models::Budget,
    /// The account this budget is anchored to.
    pub account: bc_models::Account,
    /// Tree depth (0 = root expense group, 1 = category, 2+ = sub-category).
    pub depth: u32,
    /// Effective target for the display window (pro-rated across native periods).
    /// `None` for tracking-only budgets.
    pub effective_target: Option<Decimal>,
    /// Target commodity (from `Budget::target`).
    pub commodity: Option<bc_models::CommodityCode>,
    /// Actual spend within the display window, grouped by commodity.
    /// Empty when no transactions have been posted against the account.
    pub actuals: Vec<Amount>,
    /// `true` when the budget's native period differs from the display period.
    pub has_mixed_period: bool,
    /// Child budget items (nested under this account in the hierarchy).
    pub children: Vec<BudgetTreeItem>,
}

// MARK: BudgetTreeSummary

/// Aggregate KPI values for the budget overview header.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BudgetTreeSummary {
    /// Sum of effective targets across all leaf budgets (same commodity).
    pub total_effective_target: Decimal,
    /// Sum of actuals across all leaf budgets, grouped by commodity.
    /// Multiple entries indicate a multi-currency overview.
    pub total_actuals: Vec<Amount>,
    /// Dominant commodity across leaf budgets.
    pub commodity: Option<bc_models::CommodityCode>,
    /// Count of leaf budgets where `actuals > effective_target`.
    pub overspent_count: u32,
}

// MARK: BudgetOverview

/// The complete budget page data for one display window.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct BudgetOverview {
    /// Aggregate KPI values.
    pub summary: BudgetTreeSummary,
    /// Root-level budget tree nodes.
    pub nodes: Vec<BudgetTreeItem>,
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
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or data parse failure.
    #[inline]
    pub async fn get_overview(
        &self,
        display_period: &Period,
        display_start: Date,
    ) -> crate::BcResult<BudgetOverview> {
        let (window_start, window_end) = display_period.range_containing(display_start);

        let budget_svc = BudgetService::new(self.pool.clone());
        let status_engine = BudgetStatusEngine::new(self.pool.clone(), Arc::clone(&self.fx));

        let budgets = budget_svc.list().await?;
        let accounts = crate::account::Service::new(self.pool.clone())
            .list_active()
            .await?;
        let account_map: HashMap<bc_models::AccountId, bc_models::Account> =
            accounts.into_iter().map(|a| (a.id().clone(), a)).collect();

        let mut items: Vec<BudgetTreeItem> = Vec::with_capacity(budgets.len());

        for budget in &budgets {
            let effective_target = self
                .compute_effective_target(
                    &budget_svc,
                    budget,
                    display_period,
                    window_start,
                    window_end,
                )
                .await?;

            let window =
                bc_models::BudgetWindow::custom(window_start, window_end, "display".to_owned());
            let status = status_engine.status_for_window(budget, window).await?;

            let account = account_map
                .get(budget.account_id())
                .ok_or_else(|| crate::BcError::NotFound(budget.account_id().to_string()))?
                .clone();

            let has_mixed_period = !periods_equivalent(display_period, budget.period());
            let commodity = budget.target().map(|t| t.commodity().clone());
            let actuals = match status.commodity {
                Some(c) => vec![Amount::new(status.actuals, c)],
                None => vec![],
            };

            items.push(BudgetTreeItem {
                budget: budget.clone(),
                account,
                depth: 0,
                effective_target,
                commodity,
                actuals,
                has_mixed_period,
                children: vec![],
            });
        }

        // Assign depths from the account hierarchy.
        assign_depths(&mut items, &account_map);

        // Sort into tree order (parent before children, siblings by account name).
        let nodes = build_tree(items);

        let summary = compute_summary(&nodes);

        Ok(BudgetOverview { summary, nodes })
    }

    /// Returns native period breakdown for one budget within `[display_start, display_end)`.
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
    ) -> crate::BcResult<Vec<NativePeriodStatus>> {
        let budget_svc = BudgetService::new(self.pool.clone());
        let status_engine = BudgetStatusEngine::new(self.pool.clone(), Arc::clone(&self.fx));

        let overlaps = overlapping_periods(budget.period(), display_start, display_end)
            .map_err(crate::BcError::InvalidInput)?;

        let mut result = Vec::with_capacity(overlaps.len());
        for overlap in overlaps {
            let alloc = budget_svc
                .get_allocation(budget.id(), overlap.native_start)
                .await?;
            let full_target = alloc
                .as_ref()
                .map(|a| a.amount().value())
                .or_else(|| budget.target().map(bc_models::Amount::value))
                .unwrap_or(Decimal::ZERO);

            let effective_target = budget.target().is_some().then(|| {
                let native_days = overlap.native_days();
                let overlap_days = overlap.overlap_days();
                if native_days == 0_i32 {
                    Decimal::ZERO
                } else {
                    #[expect(
                        clippy::arithmetic_side_effects,
                        reason = "Decimal div/mul for pro-rata; guarded by native_days != 0"
                    )]
                    {
                        (full_target * Decimal::from(overlap_days) / Decimal::from(native_days))
                            .round_dp(2)
                    }
                }
            });

            let window = bc_models::BudgetWindow::custom(
                overlap.overlap_start,
                overlap.overlap_end,
                overlap.native_start.to_string(),
            );
            let status = status_engine.status_for_window(budget, window).await?;

            result.push(NativePeriodStatus {
                overlap,
                effective_target,
                actuals: status.actuals,
                commodity: status.commodity,
            });
        }

        Ok(result)
    }

    /// Computes the effective target for `budget` across the display window.
    async fn compute_effective_target(
        &self,
        budget_svc: &BudgetService,
        budget: &bc_models::Budget,
        _display_period: &Period,
        display_start: Date,
        display_end: Date,
    ) -> crate::BcResult<Option<Decimal>> {
        if budget.target().is_none() {
            return Ok(None);
        }

        let overlaps = overlapping_periods(budget.period(), display_start, display_end)
            .map_err(crate::BcError::InvalidInput)?;

        let mut total = Decimal::ZERO;
        for overlap in overlaps {
            let alloc = budget_svc
                .get_allocation(budget.id(), overlap.native_start)
                .await?;
            let full = alloc
                .as_ref()
                .map(|a| a.amount().value())
                .or_else(|| budget.target().map(bc_models::Amount::value))
                .unwrap_or(Decimal::ZERO);

            let native_days = overlap.native_days();
            let overlap_days = overlap.overlap_days();
            let contribution = if native_days == 0_i32 {
                Decimal::ZERO
            } else {
                #[expect(
                    clippy::arithmetic_side_effects,
                    reason = "Decimal div/mul for pro-rata; guarded by native_days != 0"
                )]
                {
                    (full * Decimal::from(overlap_days) / Decimal::from(native_days)).round_dp(2)
                }
            };
            total = total
                .checked_add(contribution)
                .ok_or_else(|| crate::BcError::BadData("effective target overflow".into()))?;
        }

        Ok(Some(total))
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
}

// MARK: Helpers

/// Returns `true` when `a` and `b` are the same [`Period`] variant.
fn periods_equivalent(a: &Period, b: &Period) -> bool {
    core::mem::discriminant(a) == core::mem::discriminant(b)
}

/// Walks each item's account parent chain and assigns a tree depth.
fn assign_depths(
    items: &mut [BudgetTreeItem],
    account_map: &HashMap<bc_models::AccountId, bc_models::Account>,
) {
    for item in items.iter_mut() {
        let mut depth = 0_u32;
        let mut current = item.account.parent_id().cloned();
        let mut visited: HashSet<bc_models::AccountId> = HashSet::new();
        while let Some(parent_id) = current {
            if visited.contains(&parent_id) {
                tracing::warn!(
                    account_id = %parent_id,
                    "cycle detected in account parent chain; stopping depth walk"
                );
                break;
            }
            visited.insert(parent_id.clone());
            if account_map.contains_key(&parent_id) {
                depth = depth.saturating_add(1);
            }
            current = account_map
                .get(&parent_id)
                .and_then(|a| a.parent_id())
                .cloned();
        }
        item.depth = depth;
    }
}

/// Assembles items into a nested tree rooted at accounts that have no budgeted parent.
///
/// Items whose account's parent is also in the budget set become children of that
/// parent rather than roots.  Siblings at every level are sorted by account name.
fn build_tree(items: Vec<BudgetTreeItem>) -> Vec<BudgetTreeItem> {
    let budget_account_ids: HashSet<bc_models::AccountId> =
        items.iter().map(|i| i.account.id().clone()).collect();

    let mut by_account: HashMap<bc_models::AccountId, BudgetTreeItem> = items
        .into_iter()
        .map(|i| (i.account.id().clone(), i))
        .collect();

    let root_ids: Vec<bc_models::AccountId> = by_account
        .iter()
        .filter(|(_, item)| {
            item.account
                .parent_id()
                .is_none_or(|pid| !budget_account_ids.contains(pid))
        })
        .map(|(id, _)| id.clone())
        .collect();

    let mut roots: Vec<BudgetTreeItem> = root_ids
        .into_iter()
        .filter_map(|id| {
            let mut item = by_account.remove(&id)?;
            item.children = collect_children(item.account.id(), &mut by_account);
            Some(item)
        })
        .collect();

    roots.sort_by(|a, b| a.account.name().cmp(b.account.name()));
    roots
}

/// Recursively collects and nests children of `parent_id` from `by_account`.
fn collect_children(
    parent_id: &bc_models::AccountId,
    by_account: &mut HashMap<bc_models::AccountId, BudgetTreeItem>,
) -> Vec<BudgetTreeItem> {
    let child_ids: Vec<bc_models::AccountId> = by_account
        .values()
        .filter(|item| item.account.parent_id() == Some(parent_id))
        .map(|item| item.account.id().clone())
        .collect();

    let mut children: Vec<BudgetTreeItem> = child_ids
        .into_iter()
        .filter_map(|id| {
            let mut item = by_account.remove(&id)?;
            item.children = collect_children(item.account.id(), by_account);
            Some(item)
        })
        .collect();

    children.sort_by(|a, b| a.account.name().cmp(b.account.name()));
    children
}

/// Merges `new` into `amounts`, adding to an existing entry with the same commodity
/// or appending a new entry.
fn merge_amount(amounts: &mut Vec<Amount>, new: Amount) {
    match amounts
        .iter_mut()
        .find(|a| a.commodity() == new.commodity())
    {
        Some(existing) => {
            *existing = Amount::new(
                existing.value().saturating_add(new.value()),
                existing.commodity().clone(),
            );
        }
        None => amounts.push(new),
    }
}

/// Computes aggregate KPI values from the full budget tree (all depths).
fn compute_summary(nodes: &[BudgetTreeItem]) -> BudgetTreeSummary {
    let mut total_target = Decimal::ZERO;
    let mut total_actuals: Vec<Amount> = Vec::new();
    let mut overspent = 0_u32;
    let mut commodity = None;

    accumulate_summary(
        nodes,
        &mut total_target,
        &mut total_actuals,
        &mut overspent,
        &mut commodity,
    );

    BudgetTreeSummary {
        total_effective_target: total_target,
        total_actuals,
        commodity,
        overspent_count: overspent,
    }
}

/// Recursively accumulates KPI values, counting only leaf nodes.
fn accumulate_summary(
    nodes: &[BudgetTreeItem],
    total_target: &mut Decimal,
    total_actuals: &mut Vec<Amount>,
    overspent: &mut u32,
    commodity: &mut Option<bc_models::CommodityCode>,
) {
    for node in nodes {
        if node.children.is_empty() {
            // Leaf node: count directly toward totals.
            if let Some(t) = node.effective_target {
                *total_target = total_target.saturating_add(t);
                if commodity.is_none() {
                    commodity.clone_from(&node.commodity);
                }
            }
            for amount in &node.actuals {
                merge_amount(total_actuals, amount.clone());
            }
            let node_total: Decimal = node.actuals.iter().map(Amount::value).sum();
            if node.effective_target.is_some_and(|t| node_total > t) {
                *overspent = overspent.saturating_add(1);
            }
        } else {
            // Parent node: recurse into children only.
            accumulate_summary(
                &node.children,
                total_target,
                total_actuals,
                overspent,
                commodity,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Period;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::RolloverPolicy;
    use bc_models::Transaction;
    use bc_models::TransactionStatus;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::BudgetTreeService;
    use crate::account::Service as AccountService;
    use crate::budget::BudgetService;
    use crate::fx::noop_fx;
    use crate::transaction::Service as TransactionService;

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
            .target(Amount::new(dec!(300), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
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
                .status(TransactionStatus::Cleared)
                .created_at(jiff::Timestamp::now())
                .build(),
        )
        .await
        .expect("create tx");

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, Date::constant(2026, 6, 1))
            .await
            .expect("overview");

        assert_eq!(overview.nodes.len(), 1);
        let node = overview.nodes.first().expect("one node");
        assert_eq!(
            node.actuals,
            vec![Amount::new(dec!(68), CommodityCode::new("AUD"))]
        );
        assert_eq!(node.effective_target, Some(dec!(300)));
        assert_eq!(overview.summary.overspent_count, 0);
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
            .target(Amount::new(dec!(30), CommodityCode::new("AUD")))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("budget");

        drop(checking);

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, Date::constant(2026, 6, 1))
            .await
            .expect("overview");

        assert_eq!(overview.nodes.len(), 1);
        let node = overview.nodes.first().expect("one node");
        // June 2026 has 5 overlapping weeks (4 full + 1 partial of 2 days).
        // Effective target = 4 x $30 + (2/7 x $30) approx $128.57
        let target = node.effective_target.expect("has target");
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
            .target(Amount::new(dec!(500), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("food budget");
        budgets
            .create()
            .account_id(restaurants.clone())
            .target(Amount::new(dec!(200), CommodityCode::new("AUD")))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("restaurant budget");

        let svc = BudgetTreeService::new(pool.clone(), noop_fx());
        let overview = svc
            .get_overview(&Period::Monthly, Date::constant(2026, 6, 1))
            .await
            .expect("overview");

        // One root (Food) with one child (Restaurants).
        assert_eq!(overview.nodes.len(), 1, "expected one root node");
        let root = overview.nodes.first().expect("one node");
        assert_eq!(root.children.len(), 1, "expected one child under Food");

        let child = root.children.first().expect("one child");
        assert_eq!(child.account.name(), "Restaurants");

        // Summary counts only the leaf (Restaurants = $200); Food is a parent.
        assert_eq!(overview.summary.total_effective_target, dec!(200));
        assert_eq!(overview.summary.overspent_count, 0);

        drop(restaurants);
    }
}
