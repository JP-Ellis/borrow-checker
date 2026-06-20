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

impl core::fmt::Display for RolloverPolicy {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::CarryForward => write!(f, "Carry forward"),
            Self::ResetToZero => write!(f, "Reset to zero"),
            Self::CapAtTarget => write!(f, "Cap at target"),
        }
    }
}

/// The intersection of a revision's reign with a display window.
///
/// `start`/`end` are the inclusive/exclusive bounds of the slice of the window
/// this revision governs. `covers_full_window` is `true` only when that slice is
/// the entire window (the revision alone governs it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WindowOverlap {
    /// First day this revision is active within the window (inclusive).
    pub start: jiff::civil::Date,
    /// Exclusive end of the active range within the window.
    pub end: jiff::civil::Date,
    /// `true` when the active range spans the entire display window.
    pub covers_full_window: bool,
}

impl WindowOverlap {
    /// Creates a new [`WindowOverlap`].
    #[must_use]
    #[inline]
    pub fn new(start: jiff::civil::Date, end: jiff::civil::Date, covers_full_window: bool) -> Self {
        Self {
            start,
            end,
            covers_full_window,
        }
    }
}

/// One revision in a budget's timeline, as seen against a display window.
///
/// A revision governs `[effective_from, reign_end)` (open-ended for the latest
/// revision). `window_overlap` is `Some` when that reign intersects the display
/// window — `covers_full_window` distinguishes a revision governing the whole
/// window from one governing only a sub-range; `None` means it is inactive in
/// the current window.
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[builder(on(String, into))]
#[non_exhaustive]
pub struct BudgetRevisionView {
    /// Revision identifier (`BudgetRevisionId`).
    pub id: String,
    /// Exact stored date this revision takes effect.
    pub effective_from: jiff::civil::Date,
    /// The next revision's `effective_from`, or `None` for the latest revision.
    pub reign_end: Option<jiff::civil::Date>,
    /// Display label; `None` falls back to the account name.
    pub name: Option<String>,
    /// Per-period target, or `None` for tracking-only.
    pub target: Option<Amount>,
    /// Recurrence period.
    pub period: crate::Period,
    /// Compact period label, e.g. `"weekly"`.
    pub period_label: String,
    /// Rollover policy.
    pub rollover: RolloverPolicy,
    /// Tag filter id string, or `None` if unfiltered.
    pub tag_filter: Option<String>,
    /// Overlap of this revision's reign with the display window.
    pub window_overlap: Option<WindowOverlap>,
}

/// One node in the budget tree returned by `get_budget_overview`.
///
/// Leaf nodes represent individual budgets; parent nodes aggregate their
/// children.
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[builder(on(String, into))]
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
    /// Native period label, e.g. `"monthly"`.
    pub native_period_label: String,
    /// `true` when the budget's native period differs from the display window.
    pub has_mixed_period: bool,
    /// Rollover policy. `None` for aggregate parent nodes.
    pub rollover: Option<RolloverPolicy>,
    /// Optional tag filter path string (e.g. `"person:me"`). `None` if unfiltered.
    pub tag_filter: Option<String>,
    /// `true` when this budget has no allocation target (tracking-only mode).
    pub is_tracking_only: bool,
    /// Child nodes (empty for leaf rows).
    #[builder(default)]
    pub children: Vec<BudgetTreeNode>,
}

/// KPI summary for the budget page header.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetSummary {
    /// Total effective budget target across all active budgets in the display window.
    /// `None` when no commodity can be determined (e.g. all budgets are tracking-only).
    pub total_budgeted: Option<Amount>,
    /// Total actual spend across all active budgets in the display window.
    /// `None` when no commodity can be determined.
    pub total_spent: Option<Amount>,
    /// `total_budgeted - total_spent` (may be negative when overspent).
    /// `None` when no commodity can be determined.
    pub total_remaining: Option<Amount>,
    /// `true` when budgets across the display window use more than one commodity,
    /// making a single-currency total meaningless.
    pub has_mixed_commodities: bool,
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
        total_budgeted: Option<Amount>,
        total_spent: Option<Amount>,
        total_remaining: Option<Amount>,
        has_mixed_commodities: bool,
        overspent_count: u32,
    ) -> Self {
        Self {
            total_budgeted,
            total_spent,
            total_remaining,
            has_mixed_commodities,
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
    /// Start of this native period (inclusive).
    pub period_start: jiff::civil::Date,
    /// End of this native period (exclusive).
    pub period_end: jiff::civil::Date,
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
        period_start: jiff::civil::Date,
        period_end: jiff::civil::Date,
        effective_target: Option<Amount>,
        spent: Amount,
    ) -> Self {
        Self {
            label: label.into(),
            period_start,
            period_end,
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
    use crate::Period;

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
        let child = BudgetTreeNode::builder()
            .id("child-1")
            .account_id("acct-2")
            .account_name("Savings")
            .depth(1)
            .name("Groceries")
            .effective_target(Amount::from_minor(50_000, "AUD", 2))
            .spent(Amount::from_minor(12_300, "AUD", 2))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .rollover(RolloverPolicy::CarryForward)
            .is_tracking_only(false)
            .build();
        let node = BudgetTreeNode::builder()
            .id("parent-1")
            .account_id("acct-1")
            .account_name("Everyday")
            .depth(0)
            .spent(Amount::from_minor(0, "AUD", 2))
            .native_period_label("monthly")
            .has_mixed_period(false)
            .is_tracking_only(false)
            .children(vec![child])
            .build();
        let json = serde_json::to_string(&node).expect("ser");
        let back: BudgetTreeNode = serde_json::from_str(&json).expect("de");
        assert_eq!(node, back);
    }

    #[test]
    fn native_period_row_serde_roundtrip() {
        let row = NativePeriodRow::new(
            "w24 · 9–15 Jun",
            jiff::civil::Date::constant(2026, 6, 9),
            jiff::civil::Date::constant(2026, 6, 16),
            Some(Amount::from_minor(15_000, "AUD", 2)),
            Amount::from_minor(8_200, "AUD", 2),
        );
        let json = serde_json::to_string(&row).expect("ser");
        let back: NativePeriodRow = serde_json::from_str(&json).expect("de");
        assert_eq!(row, back);
    }

    #[test]
    fn budget_summary_serde_roundtrip() {
        let zero = Amount::from_minor(0, "AUD", 2);
        let summary = BudgetSummary::new(
            Some(zero.clone()),
            Some(zero.clone()),
            Some(zero.clone()),
            false,
            0,
        );
        let json = serde_json::to_string(&summary).expect("ser");
        let back: BudgetSummary = serde_json::from_str(&json).expect("de");
        assert_eq!(summary, back);
    }

    #[test]
    fn budget_summary_none_totals_roundtrip() {
        let summary = BudgetSummary::new(None, None, None, true, 0);
        let json = serde_json::to_string(&summary).expect("ser");
        let back: BudgetSummary = serde_json::from_str(&json).expect("de");
        assert_eq!(summary, back);
    }

    #[test]
    fn window_overlap_serde_roundtrip() {
        let o = WindowOverlap::new(
            jiff::civil::Date::constant(2026, 1, 1),
            jiff::civil::Date::constant(2026, 4, 1),
            false,
        );
        let json = serde_json::to_string(&o).expect("ser");
        let back: WindowOverlap = serde_json::from_str(&json).expect("de");
        assert_eq!(o, back);
    }

    #[test]
    fn budget_revision_view_serde_roundtrip() {
        let view = BudgetRevisionView::builder()
            .id("budget_rev_1")
            .effective_from(jiff::civil::Date::constant(2027, 1, 1))
            .reign_end(jiff::civil::Date::constant(2027, 9, 1))
            .name("Groceries")
            .target(Amount::from_minor(25_000, "AUD", 2))
            .period(Period::Weekly)
            .period_label("weekly")
            .rollover(RolloverPolicy::CarryForward)
            .tag_filter("tag_abc")
            .window_overlap(WindowOverlap::new(
                jiff::civil::Date::constant(2027, 1, 1),
                jiff::civil::Date::constant(2027, 9, 1),
                true,
            ))
            .build();
        let json = serde_json::to_string(&view).expect("ser");
        let back: BudgetRevisionView = serde_json::from_str(&json).expect("de");
        assert_eq!(view, back);
    }

    #[test]
    fn budget_revision_view_optional_fields_roundtrip() {
        // tracking-only, no name, no reign_end, not active in window.
        let view = BudgetRevisionView::builder()
            .id("budget_rev_2")
            .effective_from(jiff::civil::Date::constant(2026, 1, 1))
            .period(Period::Monthly)
            .period_label("monthly")
            .rollover(RolloverPolicy::ResetToZero)
            .build();
        assert!(view.target.is_none());
        assert!(view.name.is_none());
        assert!(view.reign_end.is_none());
        assert!(view.window_overlap.is_none());
        let json = serde_json::to_string(&view).expect("ser");
        let back: BudgetRevisionView = serde_json::from_str(&json).expect("de");
        assert_eq!(view, back);
    }
}
