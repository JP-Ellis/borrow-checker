//! Budget domain types.
//!
//! A [`Budget`] attaches an allocation target, period, and optional tag filter
//! to any account. The account tree (specifically `Expense`-type accounts) is
//! the category hierarchy; budgets are linked entities on top of it.
//!
//! [`BudgetAllocation`] records the amount explicitly budgeted for a specific
//! budget line in a given period (zero-based budgeting workflow).

use jiff::Timestamp;
use jiff::civil::Date;

crate::define_id!(BudgetId, "budget");
crate::define_id!(BudgetAllocationId, "budget_alloc");

/// Determines what happens to unspent (or overspent) funds at the end of a period.
///
/// # Example
///
/// ```
/// use bc_models::RolloverPolicy;
///
/// let policy = RolloverPolicy::CarryForward;
/// assert_eq!(policy, RolloverPolicy::CarryForward);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum RolloverPolicy {
    /// Unspent funds roll into the next period's balance.
    CarryForward,
    /// The budget resets to zero at the start of each period.
    ResetToZero,
    /// Unspent funds carry forward but are capped at the allocation target.
    CapAtTarget,
}

/// A budget line anchored to an account.
///
/// A `Budget` attaches allocation targets, period rules, and an optional tag
/// filter to a specific account. The account is the category; the budget adds
/// planning metadata on top of it.
///
/// Budget anchoring is **permanent** — a `Budget` cannot be re-anchored to a
/// different account after creation. To restructure categories, archive the
/// budget and create a replacement on the new account.
///
/// When [`Budget::tag_filter`] is `None`, all postings to [`Budget::account_id`]
/// count against this budget. When `Some(tag)`, only postings carrying that tag
/// or a descendant tag count (descendant-or-equal semantics).
///
/// When [`Budget::target`] is `None` the budget operates in *tracking-only* mode:
/// transactions are categorised but no allocation target is enforced.
///
/// # Example
///
/// ```
/// use bc_models::{Budget, AccountId, RolloverPolicy, Period};
/// use jiff::Timestamp;
///
/// let budget = Budget::builder()
///     .account_id(AccountId::new())
///     .period(Period::Monthly)
///     .rollover(RolloverPolicy::ResetToZero)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert!(budget.target().is_none());
/// assert!(budget.tag_filter().is_none());
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Budget {
    /// Stable, opaque identifier for this budget (a prefixed `UUIDv7`).
    ///
    /// Auto-generated when not supplied; only set this when re-hydrating a
    /// record from storage.
    #[builder(default)]
    id: BudgetId,

    /// The account this budget is anchored to.
    ///
    /// Permanent after creation — archive and recreate to change the account.
    account_id: crate::AccountId,

    /// Optional tag filter.
    ///
    /// `None` — all postings to `account_id` count.
    /// `Some(tag)` — only postings carrying `tag` or a descendant of `tag`
    /// count against this budget.
    tag_filter: Option<crate::TagId>,

    /// Display name for this budget line.
    ///
    /// When `None`, the account name is used as the display label.
    #[builder(into)]
    name: Option<String>,

    /// Optional allocation target per period.
    ///
    /// `None` places the budget in tracking-only mode.
    target: Option<crate::money::Amount>,

    /// The recurring period over which the allocation target is measured.
    period: crate::period::Period,

    /// What happens to unspent funds at the end of each period.
    rollover: RolloverPolicy,

    /// Timestamp recorded when this budget was first persisted.
    created_at: Timestamp,

    /// Timestamp at which this budget was archived, or `None` if still active.
    archived_at: Option<Timestamp>,
}

impl Budget {
    /// Returns the budget ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &BudgetId {
        &self.id
    }

    /// Returns the account ID this budget is anchored to.
    #[inline]
    #[must_use]
    pub fn account_id(&self) -> &crate::AccountId {
        &self.account_id
    }

    /// Returns the tag filter, if any.
    #[inline]
    #[must_use]
    pub fn tag_filter(&self) -> Option<&crate::TagId> {
        self.tag_filter.as_ref()
    }

    /// Returns the display name, if set.
    #[inline]
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    /// Returns the allocation target, if set.
    #[inline]
    #[must_use]
    pub fn target(&self) -> Option<&crate::money::Amount> {
        self.target.as_ref()
    }

    /// Returns `true` when no allocation target is set (tracking-only mode).
    #[inline]
    #[must_use]
    pub fn is_tracking_only(&self) -> bool {
        self.target.is_none()
    }

    /// Returns the budget period.
    #[inline]
    #[must_use]
    pub fn period(&self) -> &crate::period::Period {
        &self.period
    }

    /// Returns the rollover policy.
    #[inline]
    #[must_use]
    pub fn rollover(&self) -> RolloverPolicy {
        self.rollover
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns `true` if this budget has been archived.
    #[inline]
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }

    /// Returns the archive timestamp, if archived.
    #[inline]
    #[must_use]
    pub fn archived_at(&self) -> Option<&Timestamp> {
        self.archived_at.as_ref()
    }
}

/// A record of the amount explicitly budgeted for a budget line in a specific period.
///
/// Used in zero-based budgeting: before the period starts, the user allocates
/// funds to each budget line. Actuals accumulate as transactions are recorded.
///
/// # Example
///
/// ```
/// use bc_models::{BudgetAllocation, BudgetId, Amount, CommodityCode, Decimal};
/// use jiff::{Timestamp, civil::Date};
///
/// let alloc = BudgetAllocation::builder()
///     .budget_id(BudgetId::new())
///     .period_start(Date::constant(2026, 1, 1))
///     .amount(Amount::new(Decimal::from(500), CommodityCode::new("AUD")))
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(alloc.period_start(), Date::constant(2026, 1, 1));
/// ```
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct BudgetAllocation {
    /// Stable, opaque identifier (a prefixed `UUIDv7`).
    #[builder(default)]
    id: BudgetAllocationId,

    /// The budget this allocation applies to.
    budget_id: BudgetId,

    /// The calendar date on which this budget period begins.
    period_start: Date,

    /// The amount budgeted for this period.
    amount: crate::money::Amount,

    /// Timestamp recorded when this allocation was first persisted.
    created_at: Timestamp,
}

impl BudgetAllocation {
    /// Returns the allocation ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &BudgetAllocationId {
        &self.id
    }

    /// Returns the budget ID this allocation belongs to.
    #[inline]
    #[must_use]
    pub fn budget_id(&self) -> &BudgetId {
        &self.budget_id
    }

    /// Returns the start date of the budget period this allocation covers.
    #[inline]
    #[must_use]
    pub fn period_start(&self) -> Date {
        self.period_start
    }

    /// Returns the budgeted amount.
    #[inline]
    #[must_use]
    pub fn amount(&self) -> &crate::money::Amount {
        &self.amount
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

#[cfg(test)]
mod tests {
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::Amount;
    use crate::CommodityCode;
    use crate::Decimal;
    use crate::Period;

    #[test]
    fn budget_id_round_trips_display_from_str() {
        let id = BudgetId::new();
        let s = id.to_string();
        let parsed: BudgetId = s.parse().expect("should parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn budget_id_has_budget_prefix() {
        let id = BudgetId::new();
        assert!(
            id.to_string().starts_with("budget_"),
            "expected budget_ prefix, got {id}"
        );
    }

    #[test]
    fn budget_allocation_id_round_trips_display_from_str() {
        let id = BudgetAllocationId::new();
        let s = id.to_string();
        let parsed: BudgetAllocationId = s.parse().expect("should parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn budget_allocation_id_has_budget_alloc_prefix() {
        let id = BudgetAllocationId::new();
        assert!(
            id.to_string().starts_with("budget_alloc_"),
            "expected budget_alloc_ prefix, got {id}"
        );
    }

    #[test]
    fn rollover_policy_serialises_as_snake_case() {
        let carry = serde_json::to_string(&RolloverPolicy::CarryForward).expect("ser");
        let reset = serde_json::to_string(&RolloverPolicy::ResetToZero).expect("ser");
        let cap = serde_json::to_string(&RolloverPolicy::CapAtTarget).expect("ser");
        assert_eq!(carry, r#""carry_forward""#);
        assert_eq!(reset, r#""reset_to_zero""#);
        assert_eq!(cap, r#""cap_at_target""#);
    }

    #[test]
    fn budget_without_target_is_tracking_only() {
        let budget = Budget::builder()
            .account_id(crate::AccountId::new())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build();
        assert!(budget.is_tracking_only());
        assert!(budget.target().is_none());
        assert!(budget.tag_filter().is_none());
    }

    #[test]
    fn budget_with_target_is_not_tracking_only() {
        let budget = Budget::builder()
            .account_id(crate::AccountId::new())
            .target(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build();
        assert!(!budget.is_tracking_only());
    }

    #[test]
    fn budget_with_tag_filter_stores_tag_id() {
        let tag_id = crate::TagId::new();
        let budget = Budget::builder()
            .account_id(crate::AccountId::new())
            .tag_filter(tag_id.clone())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build();
        assert_eq!(budget.tag_filter(), Some(&tag_id));
    }

    #[test]
    fn budget_allocation_stores_fields() {
        let budget_id = BudgetId::new();
        let alloc = BudgetAllocation::builder()
            .budget_id(budget_id.clone())
            .period_start(Date::constant(2026, 1, 1))
            .amount(Amount::new(
                Decimal::from(600_i32),
                CommodityCode::new("AUD"),
            ))
            .created_at(jiff::Timestamp::now())
            .build();
        assert_eq!(alloc.budget_id(), &budget_id);
        assert_eq!(alloc.period_start(), Date::constant(2026, 1, 1));
        assert!(alloc.id().to_string().starts_with("budget_alloc_"));
    }

    #[test]
    fn budget_with_archived_at_is_archived() {
        let now = jiff::Timestamp::now();
        let budget = Budget::builder()
            .account_id(crate::AccountId::new())
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(now)
            .archived_at(now)
            .build();
        assert!(budget.is_archived());
        assert!(budget.archived_at().is_some());
    }
}
