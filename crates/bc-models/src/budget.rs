//! Budget domain types.
//!
//! A [`Budget`] is a permanent anchor linking a budget to an account.
//! Configuration (target, period, rollover, tag filter, name) lives in
//! time-ordered [`BudgetRevision`]s. This design makes history immutable
//! and auditable: revising a budget creates a new revision rather than
//! mutating existing data.
//!
//! [`RolloverPolicy`] controls what happens to unspent funds at period end.

use jiff::Timestamp;
use jiff::civil::Date;

crate::define_id!(BudgetId, "budget");
crate::define_id!(BudgetRevisionId, "budget_rev");

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

/// A budget anchor: the permanent identity of a budget.
///
/// The anchor carries only identity and lifecycle. All configuration
/// (target, period, rollover, tag filter, name) lives in time-ordered
/// [`BudgetRevision`]s. `account_id` is immutable — re-anchoring to a different
/// account is a different budget; archive and recreate instead.
#[derive(bon::Builder, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Budget {
    /// Stable, opaque identifier (a prefixed `UUIDv7`).
    #[builder(default)]
    id: BudgetId,
    /// The account this budget is permanently anchored to.
    account_id: crate::AccountId,
    /// Timestamp recorded when this budget was first created.
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

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns the archive timestamp, if archived.
    #[inline]
    #[must_use]
    pub fn archived_at(&self) -> Option<&Timestamp> {
        self.archived_at.as_ref()
    }

    /// Returns `true` if this budget has been archived.
    #[inline]
    #[must_use]
    pub fn is_archived(&self) -> bool {
        self.archived_at.is_some()
    }
}

/// A single effective-dated configuration of a budget.
///
/// The revision governing a date `d` is the one with the greatest
/// `effective_from <= d`. Each revision tiles its own period grid starting at
/// `effective_from` (see [`crate::budget_timeline`]).
#[derive(bon::Builder, Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct BudgetRevision {
    /// Stable, opaque identifier (a prefixed `UUIDv7`).
    #[builder(default)]
    id: BudgetRevisionId,
    /// The anchor this revision belongs to.
    budget_id: BudgetId,
    /// Exact date this configuration begins (inclusive).
    effective_from: Date,
    /// Display label; falls back to the account name when `None`.
    #[builder(into)]
    name: Option<String>,
    /// Allocation target per period; `None` = tracking-only.
    target: Option<crate::money::Amount>,
    /// Recurring period over which the target is measured.
    period: crate::period::Period,
    /// What happens to unspent funds at period end.
    rollover: RolloverPolicy,
    /// Optional tag filter (descendant-or-equal semantics); `None` = all postings.
    tag_filter: Option<crate::TagId>,
    /// Timestamp recorded when this revision was persisted.
    created_at: Timestamp,
}

impl BudgetRevision {
    /// Returns the revision ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &BudgetRevisionId {
        &self.id
    }

    /// Returns the anchor ID this revision belongs to.
    #[inline]
    #[must_use]
    pub fn budget_id(&self) -> &BudgetId {
        &self.budget_id
    }

    /// Returns the effective-from date.
    #[inline]
    #[must_use]
    pub fn effective_from(&self) -> Date {
        self.effective_from
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

    /// Returns the tag filter, if any.
    #[inline]
    #[must_use]
    pub fn tag_filter(&self) -> Option<&crate::TagId> {
        self.tag_filter.as_ref()
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns `true` when no target is set (tracking-only mode).
    #[inline]
    #[must_use]
    pub fn is_tracking_only(&self) -> bool {
        self.target.is_none()
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
    fn rollover_policy_serialises_as_snake_case() {
        let carry = serde_json::to_string(&RolloverPolicy::CarryForward).expect("ser");
        let reset = serde_json::to_string(&RolloverPolicy::ResetToZero).expect("ser");
        let cap = serde_json::to_string(&RolloverPolicy::CapAtTarget).expect("ser");
        assert_eq!(carry, r#""carry_forward""#);
        assert_eq!(reset, r#""reset_to_zero""#);
        assert_eq!(cap, r#""cap_at_target""#);
    }

    #[test]
    fn budget_with_archived_at_is_archived() {
        let now = jiff::Timestamp::now();
        let budget = Budget::builder()
            .account_id(crate::AccountId::new())
            .created_at(now)
            .archived_at(now)
            .build();
        assert!(budget.is_archived());
        assert!(budget.archived_at().is_some());
    }

    #[test]
    fn budget_partial_eq() {
        let ts = jiff::Timestamp::now();
        let a = Budget::builder()
            .account_id(crate::AccountId::new())
            .created_at(ts)
            .build();
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn budget_revision_id_has_prefix() {
        let id = BudgetRevisionId::new();
        assert!(id.to_string().starts_with("budget_rev_"), "got {id}");
    }

    #[test]
    fn budget_anchor_holds_identity_only() {
        let now = jiff::Timestamp::now();
        let b = Budget::builder()
            .account_id(crate::AccountId::new())
            .created_at(now)
            .build();
        assert!(b.id().to_string().starts_with("budget_"));
        assert!(!b.is_archived());
    }

    #[test]
    fn budget_revision_tracking_only_when_no_target() {
        let rev = BudgetRevision::builder()
            .budget_id(BudgetId::new())
            .effective_from(Date::constant(2026, 1, 1))
            .period(Period::Monthly)
            .rollover(RolloverPolicy::ResetToZero)
            .created_at(jiff::Timestamp::now())
            .build();
        assert!(rev.is_tracking_only());
        assert!(rev.tag_filter().is_none());
        assert_eq!(rev.effective_from(), Date::constant(2026, 1, 1));
    }

    #[test]
    fn budget_revision_with_target_is_not_tracking_only() {
        let rev = BudgetRevision::builder()
            .budget_id(BudgetId::new())
            .effective_from(Date::constant(2026, 1, 1))
            .target(Amount::new(
                Decimal::from(250_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Weekly)
            .rollover(RolloverPolicy::CarryForward)
            .created_at(jiff::Timestamp::now())
            .build();
        assert!(!rev.is_tracking_only());
        assert_eq!(rev.period(), &Period::Weekly);
    }
}
