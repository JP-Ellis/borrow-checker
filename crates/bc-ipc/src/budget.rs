//! Budget types shared between Tauri backend and Leptos frontend.

use rust_decimal::Decimal;
use serde::Deserialize;
use serde::Serialize;

use crate::TagInfo;
use crate::Transaction;
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

/// What a budget's target is for, which decides whether overshooting is good.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum BudgetIntent {
    /// Stay within the target.
    Limit,
    /// Reach at least the target.
    Goal,
    /// Land near the target.
    Estimate,
}

impl core::fmt::Display for BudgetIntent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Limit => write!(f, "limit"),
            Self::Goal => write!(f, "goal"),
            Self::Estimate => write!(f, "estimate"),
        }
    }
}

// MARK: models conversions

#[cfg(feature = "models")]
impl From<bc_models::BudgetIntent> for BudgetIntent {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::BudgetIntent and bc_ipc::BudgetIntent are #[non_exhaustive]; \
                  the wildcard fallback to Limit is intentional for future unknown variants"
    )]
    fn from(value: bc_models::BudgetIntent) -> Self {
        match value {
            bc_models::BudgetIntent::Limit => Self::Limit,
            bc_models::BudgetIntent::Goal => Self::Goal,
            bc_models::BudgetIntent::Estimate => Self::Estimate,
            _ => Self::Limit,
        }
    }
}

#[cfg(feature = "models")]
impl From<BudgetIntent> for bc_models::BudgetIntent {
    #[inline]
    fn from(value: BudgetIntent) -> Self {
        match value {
            BudgetIntent::Limit => Self::Limit,
            BudgetIntent::Goal => Self::Goal,
            BudgetIntent::Estimate => Self::Estimate,
        }
    }
}

#[cfg(feature = "models")]
impl From<bc_models::Verdict> for Verdict {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "bc_models::Verdict is #[non_exhaustive]; the wildcard fallback to Bad \
                  surfaces a future unknown variant as the worst case"
    )]
    fn from(value: bc_models::Verdict) -> Self {
        match value {
            bc_models::Verdict::Good => Self::Good,
            bc_models::Verdict::Warn => Self::Warn,
            bc_models::Verdict::Bad => Self::Bad,
            _ => Self::Bad,
        }
    }
}

#[cfg(feature = "models")]
impl From<bc_models::RolloverPolicy> for RolloverPolicy {
    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::RolloverPolicy and bc_ipc::RolloverPolicy are #[non_exhaustive]; \
                  the wildcard fallback to ResetToZero is intentional for future unknown variants"
    )]
    fn from(value: bc_models::RolloverPolicy) -> Self {
        match value {
            bc_models::RolloverPolicy::CarryForward => Self::CarryForward,
            bc_models::RolloverPolicy::ResetToZero => Self::ResetToZero,
            bc_models::RolloverPolicy::CapAtTarget => Self::CapAtTarget,
            _ => Self::ResetToZero,
        }
    }
}

#[cfg(feature = "models")]
impl From<RolloverPolicy> for bc_models::RolloverPolicy {
    #[inline]
    fn from(value: RolloverPolicy) -> Self {
        match value {
            RolloverPolicy::CarryForward => Self::CarryForward,
            RolloverPolicy::ResetToZero => Self::ResetToZero,
            RolloverPolicy::CapAtTarget => Self::CapAtTarget,
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
    /// Source expression the target was evaluated from, or `None` for a
    /// literal target.
    pub target_expr: Option<String>,
    /// What the target is for.
    pub intent: BudgetIntent,
    /// `true` when the target's sign differs from an adjacent revision's.
    #[builder(default)]
    pub sign_flip: bool,
    /// Recurrence period.
    pub period: crate::Period,
    /// Compact period label, e.g. `"weekly"`.
    pub period_label: String,
    /// Rollover policy.
    pub rollover: RolloverPolicy,
    /// Tag filter, or `None` if unfiltered.
    pub tag_filter: Option<TagInfo>,
    /// Overlap of this revision's reign with the display window.
    pub window_overlap: Option<WindowOverlap>,
}

/// What a budget tree row stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// A row's traffic light. Ordered so that `max` is the worst.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Verdict {
    /// On track.
    Good,
    /// Close to the edge of the band.
    Warn,
    /// Outside the band.
    Bad,
}

/// One row in the budget tree returned by `get_budget_overview`.
///
/// Rows nest by account, then by budget; a row's children sum to its
/// `actual` unless a posting beneath counts in two of them
/// (`double_counted`).
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[builder(on(String, into))]
#[non_exhaustive]
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent row flags, each shown as its own pill"
)]
pub struct BudgetTreeNode {
    /// Budget id, or a synthetic id for account and leftover rows
    /// (`acct:{account_id}`, `unalloc:{budget_id}`, `unbud:{account_id}`).
    pub id: String,
    /// What the row stands for.
    pub kind: RowKind,
    /// Anchoring account.
    pub account_id: String,
    /// Budget name, account leaf name, tag path, or leftover label.
    pub label: String,
    /// Tag of a filtered budget.
    pub tag_filter: Option<TagInfo>,
    /// Row total; children sum to it. `None` when the row is `mixed` or no
    /// commodity is known.
    pub actual: Option<Amount>,
    /// Window-effective target. `None` for unbudgeted and tracking-only rows,
    /// and for account rows whose budgets disagree.
    pub target: Option<Amount>,
    /// The governing revision's target expression, if any.
    pub target_expr: Option<String>,
    /// The budget's intent. `None` for account and unbudgeted rows.
    pub intent: Option<BudgetIntent>,
    /// The intent a new revision on this row's account starts with.
    pub default_intent: BudgetIntent,
    /// Bar segment: spend claimed by sub-budgets, or by the budget itself.
    #[builder(default)]
    pub claimed: Decimal,
    /// Bar segment: envelope-owned spend under this row.
    #[builder(default)]
    pub unallocated: Decimal,
    /// Bar segment: unbudgeted spend under this row.
    #[builder(default)]
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
    #[builder(default)]
    pub mixed: bool,
    /// A posting beneath counts in two of this row's children.
    #[builder(default)]
    pub double_counted: bool,
    /// The envelope's sub-budget targets exceed its own.
    #[builder(default)]
    pub over_allocated: bool,
    /// A revision of this budget flips sign against a neighbour.
    #[builder(default)]
    pub sign_flip: bool,
    /// Native period label, e.g. `"monthly"`.
    pub native_period_label: String,
    /// `true` when the budget's native period differs from the display window.
    #[builder(default)]
    pub has_mixed_period: bool,
    /// Rollover policy. `None` for rows without a governing revision.
    pub rollover: Option<RolloverPolicy>,
    /// Rows nested under this one.
    #[builder(default)]
    pub children: Vec<BudgetTreeNode>,
    /// Spend that could not be valued in the row's commodity, per
    /// commodity, across the display window and the carry chain behind its
    /// rollover. Empty when every posting counted.
    #[builder(default)]
    pub unvalued: Vec<Amount>,
}

/// Header figures for the budget page.
#[derive(bon::Builder, Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetSummary {
    /// Budget and unallocated rows with a red verdict.
    #[builder(default)]
    pub red: u32,
    /// Budget and unallocated rows with a warn verdict.
    #[builder(default)]
    pub warn: u32,
    /// Budget and unallocated rows with a green verdict.
    #[builder(default)]
    pub green: u32,
    /// Unbudgeted total per type root with a non-zero total, as the root's
    /// name and the total.
    #[builder(default)]
    pub unbudgeted: Vec<(String, Amount)>,
    /// `true` when any row has unvalued spend. A flag rather than a sum:
    /// rows can target different commodities, so the amounts live on each
    /// row's `unvalued`.
    #[builder(default)]
    pub has_unvalued: bool,
}

/// The complete budget page data for one display window.
#[derive(bon::Builder, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetOverview {
    /// Header figures.
    pub summary: BudgetSummary,
    /// Type root rows.
    #[builder(default)]
    pub nodes: Vec<BudgetTreeNode>,
    /// Fraction of the window elapsed by the end of today; `None` for a
    /// window that has not started.
    pub elapsed_fraction: Option<Decimal>,
}

/// One transaction behind a budget tree row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BudgetRowTransaction {
    /// The transaction.
    pub transaction: Transaction,
    /// Label of the budget its posting landed in beneath the row, or `None`
    /// when the row owns the posting itself.
    pub bucket: Option<String>,
    /// `true` when a posting of this transaction counts in two budgets
    /// neither of whose rows nests the other.
    pub double_counted: bool,
}

impl BudgetRowTransaction {
    /// Creates a new [`BudgetRowTransaction`].
    #[must_use]
    #[inline]
    pub fn new(transaction: Transaction, bucket: Option<String>, double_counted: bool) -> Self {
        Self {
            transaction,
            bucket,
            double_counted,
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
    /// Spend that could not be valued in the budget commodity, per
    /// commodity, within this native period.
    pub unvalued: Vec<Amount>,
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
        unvalued: Vec<Amount>,
    ) -> Self {
        Self {
            label: label.into(),
            period_start,
            period_end,
            effective_target,
            spent,
            unvalued,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

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
    fn budget_intent_serde_roundtrip() {
        let json = serde_json::to_string(&BudgetIntent::Estimate).expect("ser");
        assert_eq!(json, r#""estimate""#);
        let back: BudgetIntent = serde_json::from_str(&json).expect("de");
        assert_eq!(back, BudgetIntent::Estimate);
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

    #[rstest]
    #[case(RowKind::Budget, r#""budget""#)]
    #[case(RowKind::Account, r#""account""#)]
    #[case(RowKind::Unallocated, r#""unallocated""#)]
    #[case(RowKind::Unbudgeted, r#""unbudgeted""#)]
    fn row_kind_serde_roundtrip(#[case] kind: RowKind, #[case] expected: &str) {
        let json = serde_json::to_string(&kind).expect("ser");
        assert_eq!(json, expected);
        let back: RowKind = serde_json::from_str(&json).expect("de");
        assert_eq!(back, kind);
    }

    #[rstest]
    #[case(Verdict::Good, r#""good""#)]
    #[case(Verdict::Warn, r#""warn""#)]
    #[case(Verdict::Bad, r#""bad""#)]
    fn verdict_serde_roundtrip(#[case] verdict: Verdict, #[case] expected: &str) {
        let json = serde_json::to_string(&verdict).expect("ser");
        assert_eq!(json, expected);
        let back: Verdict = serde_json::from_str(&json).expect("de");
        assert_eq!(back, verdict);
    }

    #[test]
    fn verdict_orders_worst_last() {
        assert!(Verdict::Good < Verdict::Warn);
        assert!(Verdict::Warn < Verdict::Bad);
    }

    /// An `Account` row with a `Budget` child and an `Unbudgeted` child.
    fn two_level_node() -> BudgetTreeNode {
        let budget = BudgetTreeNode::builder()
            .id("budget_1")
            .kind(RowKind::Budget)
            .account_id("acct-2")
            .label("Widgets")
            .actual(Amount::new(dec!(75.00), "AUD"))
            .target(Amount::new(dec!(100.00), "AUD"))
            .target_expr("(400.00 / 4)")
            .intent(BudgetIntent::Limit)
            .default_intent(BudgetIntent::Limit)
            .claimed(dec!(75.00))
            .verdict(Verdict::Good)
            .ratio(dec!(0.75))
            .native_period_label("monthly")
            .rollover(RolloverPolicy::CarryForward)
            .build();
        let unbudgeted = BudgetTreeNode::builder()
            .id("unbud:acct-1")
            .kind(RowKind::Unbudgeted)
            .account_id("acct-1")
            .label("↳ unbudgeted")
            .actual(Amount::new(dec!(20.00), "AUD"))
            .default_intent(BudgetIntent::Limit)
            .unbudgeted(dec!(20.00))
            .native_period_label("period")
            .build();
        BudgetTreeNode::builder()
            .id("acct:acct-1")
            .kind(RowKind::Account)
            .account_id("acct-1")
            .label("Expenses")
            .actual(Amount::new(dec!(95.00), "AUD"))
            .default_intent(BudgetIntent::Limit)
            .claimed(dec!(75.00))
            .unbudgeted(dec!(20.00))
            .worst_descendant(Verdict::Good)
            .native_period_label("period")
            .children(vec![budget, unbudgeted])
            .build()
    }

    #[test]
    fn budget_tree_node_serde_roundtrip() {
        let node = two_level_node();
        let json = serde_json::to_string(&node).expect("ser");
        let back: BudgetTreeNode = serde_json::from_str(&json).expect("de");
        assert_eq!(back, node);
        assert_eq!(
            back.children.first().and_then(|c| c.ratio),
            Some(dec!(0.75))
        );
        assert_eq!(
            back.children.get(1).map(|c| c.kind),
            Some(RowKind::Unbudgeted)
        );
    }

    #[test]
    fn budget_overview_serde_roundtrip() {
        let summary = BudgetSummary::builder()
            .red(1)
            .warn(2)
            .green(3)
            .unbudgeted(vec![(
                "Expenses".to_owned(),
                Amount::new(dec!(20.00), "AUD"),
            )])
            .has_unvalued(true)
            .build();
        let overview = BudgetOverview::builder()
            .summary(summary)
            .nodes(vec![two_level_node()])
            .elapsed_fraction(dec!(0.5))
            .build();
        let json = serde_json::to_string(&overview).expect("ser");
        let back: BudgetOverview = serde_json::from_str(&json).expect("de");
        assert_eq!(back, overview);
        assert_eq!(back.elapsed_fraction, Some(dec!(0.5)));
    }

    #[test]
    fn native_period_row_serde_roundtrip() {
        let row = NativePeriodRow::new(
            "w24 · 9–15 Jun",
            jiff::civil::Date::constant(2026, 6, 9),
            jiff::civil::Date::constant(2026, 6, 16),
            Some(Amount::new(Decimal::new(15_000, 2), "AUD")),
            Amount::new(Decimal::new(8_200, 2), "AUD"),
            vec![],
        );
        let json = serde_json::to_string(&row).expect("ser");
        let back: NativePeriodRow = serde_json::from_str(&json).expect("de");
        assert_eq!(row, back);
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
            .target(Amount::new(Decimal::new(25_000, 2), "AUD"))
            .period(Period::Weekly)
            .period_label("weekly")
            .rollover(RolloverPolicy::CarryForward)
            .intent(BudgetIntent::Goal)
            .target_expr("(30.00 / 4)")
            .sign_flip(true)
            .tag_filter(TagInfo::new("tag_abc", "household"))
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
            .intent(BudgetIntent::Limit)
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
