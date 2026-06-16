//! Budget types shared between Tauri backend and Leptos frontend.

use serde::Deserialize;
use serde::Serialize;

use crate::money::Amount;

/// Rollover policy — what happens to unspent funds at period end.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum RolloverPolicy {
    /// Unspent funds roll into the next period's balance.
    CarryForward,
    /// Budget resets to zero at the start of each period.
    ResetToZero,
    /// Unspent funds carry forward but are capped at the allocation target.
    CapAtTarget,
}

/// One node in the budget tree returned by `get_budget_overview`.
///
/// Leaf nodes represent individual budgets; parent nodes aggregate their
/// children.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetTreeNode {
    /// Stable budget identifier. Use [`str::is_empty`] to detect aggregate-only parent nodes.
    pub id: String,
    /// Stable account identifier.
    pub account_id: String,
    /// Account display name (used as fallback when `name` is `None`).
    pub account_name: String,
    /// How many levels deep this node is (0 = root).
    pub depth: u32,
    /// Budget display name override; `None` means use `account_name`.
    pub name: Option<String>,
    /// Effective budget target for the display window, or `None` for
    /// tracking-only budgets.
    pub effective_target: Option<Amount>,
    /// Actual spend within the display window.
    pub spent: Amount,
    /// Native period label, e.g. `"monthly"`. `None` for aggregate parents.
    pub native_period_label: Option<String>,
    /// `true` when the budget's native period differs from the display window.
    pub has_mixed_period: bool,
    /// Rollover policy. `None` for aggregate parent nodes.
    pub rollover: Option<RolloverPolicy>,
    /// Optional tag filter path string (e.g. `"person:me"`). `None` if unfiltered.
    pub tag_filter: Option<String>,
    /// `true` when this budget has no allocation target (tracking-only mode).
    pub is_tracking_only: bool,
    /// Child nodes (empty for leaf rows).
    pub children: Vec<BudgetTreeNode>,
}

impl BudgetTreeNode {
    /// Creates a new [`BudgetTreeNode`].
    #[must_use]
    #[inline]
    #[expect(
        clippy::too_many_arguments,
        reason = "budget tree node has many fields"
    )]
    pub fn new(
        id: impl Into<String>,
        account_id: impl Into<String>,
        account_name: impl Into<String>,
        depth: u32,
        name: Option<impl Into<String>>,
        effective_target: Option<Amount>,
        spent: Amount,
        native_period_label: Option<impl Into<String>>,
        has_mixed_period: bool,
        rollover: Option<RolloverPolicy>,
        tag_filter: Option<impl Into<String>>,
        is_tracking_only: bool,
        children: Vec<BudgetTreeNode>,
    ) -> Self {
        Self {
            id: id.into(),
            account_id: account_id.into(),
            account_name: account_name.into(),
            depth,
            name: name.map(Into::into),
            effective_target,
            spent,
            native_period_label: native_period_label.map(Into::into),
            has_mixed_period,
            rollover,
            tag_filter: tag_filter.map(Into::into),
            is_tracking_only,
            children,
        }
    }
}

/// KPI summary for the budget page header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetSummary {
    /// Total effective budget target across all active budgets in the display window.
    pub total_budgeted: Amount,
    /// Total actual spend across all active budgets in the display window.
    pub total_spent: Amount,
    /// `total_budgeted - total_spent` (may be negative when overspent).
    pub total_remaining: Amount,
    /// Number of leaf budget lines where `spent > effective_target`.
    pub overspent_count: u32,
}

impl BudgetSummary {
    /// Creates a new [`BudgetSummary`].
    ///
    /// `total_remaining` is not validated against `total_budgeted - total_spent`;
    /// the caller is responsible for consistency.
    #[must_use]
    #[inline]
    pub fn new(
        total_budgeted: Amount,
        total_spent: Amount,
        total_remaining: Amount,
        overspent_count: u32,
    ) -> Self {
        Self {
            total_budgeted,
            total_spent,
            total_remaining,
            overspent_count,
        }
    }
}

/// One native sub-period row shown when a mixed-period badge is expanded.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct NativePeriodRow {
    /// Human-readable label, e.g. `"w24 · 9–15 Jun"` or `"Oct 2026 (31 of 365 days)"`.
    pub label: String,
    /// ISO-8601 start of this native period (inclusive), e.g. `"2026-06-09"`.
    pub period_start: String,
    /// ISO-8601 end of this native period (exclusive), e.g. `"2026-06-16"`.
    pub period_end: String,
    /// Effective target for the overlap of this native period with the display window.
    pub effective_target: Option<Amount>,
    /// Actual spend within this native period.
    pub spent: Amount,
}

impl NativePeriodRow {
    /// Creates a new [`NativePeriodRow`].
    #[must_use]
    #[inline]
    pub fn new(
        label: impl Into<String>,
        period_start: impl Into<String>,
        period_end: impl Into<String>,
        effective_target: Option<Amount>,
        spent: Amount,
    ) -> Self {
        Self {
            label: label.into(),
            period_start: period_start.into(),
            period_end: period_end.into(),
            effective_target,
            spent,
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::Amount;

    #[test]
    fn rollover_policy_serialises_as_snake_case() {
        let carry = serde_json::to_string(&RolloverPolicy::CarryForward).expect("ser");
        assert_eq!(carry, r#""carry_forward""#);
        let reset = serde_json::to_string(&RolloverPolicy::ResetToZero).expect("ser");
        assert_eq!(reset, r#""reset_to_zero""#);
        let cap = serde_json::to_string(&RolloverPolicy::CapAtTarget).expect("ser");
        assert_eq!(cap, r#""cap_at_target""#);
    }

    #[test]
    fn rollover_policy_roundtrips() {
        for variant in [
            RolloverPolicy::CarryForward,
            RolloverPolicy::ResetToZero,
            RolloverPolicy::CapAtTarget,
        ] {
            let json = serde_json::to_string(&variant).expect("ser");
            let back: RolloverPolicy = serde_json::from_str(&json).expect("de");
            assert_eq!(variant, back);
        }
    }

    #[test]
    fn budget_tree_node_serde_roundtrip() {
        let child = BudgetTreeNode::new(
            "child-1",
            "acct-2",
            "Savings",
            1,
            Some("Groceries"),
            Some(Amount::new(50_000, "AUD", 2)),
            Amount::new(12_300, "AUD", 2),
            Some("monthly"),
            false,
            Some(RolloverPolicy::CarryForward),
            None::<String>,
            false,
            vec![],
        );
        let node = BudgetTreeNode::new(
            "parent-1",
            "acct-1",
            "Everyday",
            0,
            None::<String>,
            None,
            Amount::new(0, "AUD", 2),
            None::<String>,
            false,
            None,
            None::<String>,
            false,
            vec![child],
        );
        let json = serde_json::to_string(&node).expect("ser");
        let back: BudgetTreeNode = serde_json::from_str(&json).expect("de");
        assert_eq!(node, back);
    }

    #[test]
    fn native_period_row_serde_roundtrip() {
        let row = NativePeriodRow::new(
            "w24 · 9–15 Jun",
            "2026-06-09",
            "2026-06-16",
            Some(Amount::new(15_000, "AUD", 2)),
            Amount::new(8_200, "AUD", 2),
        );
        let json = serde_json::to_string(&row).expect("ser");
        let back: NativePeriodRow = serde_json::from_str(&json).expect("de");
        assert_eq!(row, back);
    }

    #[test]
    fn budget_summary_serde_roundtrip() {
        let zero = Amount::new(0, "AUD", 2);
        let summary = BudgetSummary::new(zero.clone(), zero.clone(), zero.clone(), 0);
        let json = serde_json::to_string(&summary).expect("ser");
        let back: BudgetSummary = serde_json::from_str(&json).expect("de");
        assert_eq!(summary, back);
    }
}
