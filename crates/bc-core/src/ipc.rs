//! Conversions from bc-core types into bc-ipc DTOs.
//!
//! This module is gated behind the `ipc` feature and hosts the
//! `impl From<bc_core::…> for bc_ipc::…` conversions plus named constructors
//! that translate core domain and projection types into their serialisable IPC
//! counterparts. Because the source types are local to this crate, these impls
//! are permitted by the orphan rule even though the destination DTOs live in
//! `bc-ipc`.
//!
//! Single-argument, infallible conversions use plain `impl From<&Source> for
//! bc_ipc::Dto` blocks, which the orphan rule permits because the source type
//! is local to this crate. Conversions that genuinely need more than one
//! argument (`bc_ipc::AuditEntry::from_event`,
//! `bc_ipc::NativePeriodRow::from_native`) cannot be expressed as `From`, so
//! they are exposed as extension traits instead; callers bring the trait into
//! scope to use the named constructor.

use rust_decimal::Decimal;

use crate::BudgetTreeItem;
use crate::Event;
use crate::NativePeriodStatus;
use crate::budget_tree::BudgetTreeSummary;

// MARK: Error mapping

/// Maps a [`crate::BcError`] to its IPC [`bc_ipc::BcError`] counterpart.
///
/// User-facing validation failures (`InvalidInput`, `BadData`, the account/tag
/// rule violations, marker conflicts, and commodity-in-use errors) surface as
/// [`bc_ipc::BcError::Validation`] so the UI can render a friendly message;
/// `NotFound` maps to [`bc_ipc::BcError::NotFound`]; everything genuinely
/// internal (database, IO, serialisation) becomes [`bc_ipc::BcError::Internal`].
impl From<crate::BcError> for bc_ipc::BcError {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "crate::BcError is #[non_exhaustive]; catch-all required for future variants"
    )]
    fn from(e: crate::BcError) -> Self {
        use crate::BcError as Core;

        match &e {
            Core::NotFound(_) => bc_ipc::BcError::NotFound(e.to_string()),
            Core::InvalidInput(_)
            | Core::BadData(_)
            | Core::AlreadyArchived(_)
            | Core::InvalidAccountKind { .. }
            | Core::TagInUse(_)
            | Core::MarkerConflict { .. }
            | Core::CommodityInUse(_) => bc_ipc::BcError::Validation(e.to_string()),
            _ => bc_ipc::BcError::Internal(e.to_string()),
        }
    }
}

// MARK: Audit entries

/// Extension trait building a [`bc_ipc::AuditEntry`] from a core [`Event`].
pub trait AuditEntryExt {
    /// Maps a core [`Event`] recorded at `ts` to a UI audit entry.
    ///
    /// # Arguments
    ///
    /// * `ts` - When the event was recorded.
    /// * `event` - The core event to describe.
    ///
    /// # Returns
    ///
    /// A [`bc_ipc::AuditEntry`] with a short kind tag and a human-readable
    /// message.
    fn from_event(ts: jiff::Timestamp, event: &Event) -> Self;
}

impl AuditEntryExt for bc_ipc::AuditEntry {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is #[non_exhaustive]; catch-all arm required for exhaustiveness against future variants"
    )]
    fn from_event(ts: jiff::Timestamp, event: &Event) -> Self {
        let (kind, message): (&str, String) = match event {
            Event::TransactionCreated { .. } => ("create", "transaction created".to_owned()),
            Event::TransactionAmended { .. } => ("amend", "transaction amended".to_owned()),
            Event::TransactionVoided { .. } => ("void", "transaction voided".to_owned()),
            Event::TransactionReversed { .. } => ("reverse", "transaction reversed".to_owned()),
            Event::TransactionPayeeChanged { to, .. } => (
                "payee",
                format!("payee → {}", to.as_deref().unwrap_or("(none)")),
            ),
            Event::TransactionDateChanged { to, .. } => ("date", format!("date → {to}")),
            Event::TransactionExtraDatesChanged { .. } => {
                ("dates", "extra dates changed".to_owned())
            }
            Event::TransactionDescriptionChanged { .. } => {
                ("desc", "description changed".to_owned())
            }
            Event::TransactionNoteChanged { to, .. } => (
                "note",
                match to {
                    Some(_) => "note changed".to_owned(),
                    None => "note removed".to_owned(),
                },
            ),
            Event::TransactionTagsChanged { added, removed, .. } => {
                ("tags", format!("tags +{} -{}", added.len(), removed.len()))
            }
            Event::TransactionReconciled { from, to, .. } => {
                ("reconcile", format!("reconciliation {from:?} → {to:?}"))
            }
            Event::PostingRecategorised { to_account, .. } => {
                ("recat", format!("recategorised → {to_account}"))
            }
            Event::PostingAmountChanged { .. } => ("amount", "amount changed".to_owned()),
            Event::PostingNoteChanged { .. } => ("note", "posting note changed".to_owned()),
            Event::PostingSpreadChanged { to, .. } => (
                "spread",
                match to {
                    Some((from, until)) => format!("spread {from}..{until}"),
                    None => "spread cleared".to_owned(),
                },
            ),
            Event::PostingAdded { account, .. } => ("split", format!("+leg {account}")),
            Event::PostingRemoved { .. } => ("split", "removed leg".to_owned()),
            other => {
                let k = other.kind();
                (k, k.to_owned())
            }
        };
        bc_ipc::AuditEntry::new(ts, kind.to_owned(), message)
    }
}

// MARK: Budget tree

/// Converts a [`BudgetTreeItem`] (and its children, recursively) into a
/// [`bc_ipc::BudgetTreeNode`].
impl From<&BudgetTreeItem> for bc_ipc::BudgetTreeNode {
    fn from(item: &BudgetTreeItem) -> Self {
        budget_tree_node_recursive(item)
    }
}

/// Recursive implementation of the `From<&BudgetTreeItem>` conversion above.
fn budget_tree_node_recursive(item: &BudgetTreeItem) -> bc_ipc::BudgetTreeNode {
    let spent = item.actuals.first().map_or_else(
        || {
            let c = item
                .commodity
                .as_ref()
                .map_or("", bc_models::CommodityCode::as_str);
            bc_ipc::Amount::new(Decimal::ZERO, c)
        },
        bc_ipc::Amount::from,
    );
    let effective_target = match (item.effective_target, &item.commodity) {
        (Some(t), Some(c)) => Some(bc_ipc::Amount::new(t, c.as_str())),
        _ => None,
    };

    let native_period_label = item
        .governing
        .as_ref()
        .map_or_else(|| "period".to_owned(), |r| period_label(r.period()));

    let children: Vec<_> = item
        .children
        .iter()
        .map(budget_tree_node_recursive)
        .collect();

    let gov = item.governing.as_ref();
    bc_ipc::BudgetTreeNode::builder()
        .id(item.budget.id().to_string())
        .account_id(item.account.id().to_string())
        .account_name(item.account.name().to_owned())
        .depth(item.depth)
        .maybe_name(gov.and_then(|r| r.name()).map(ToOwned::to_owned))
        .maybe_effective_target(effective_target)
        .spent(spent)
        .native_period_label(native_period_label)
        .has_mixed_period(item.has_mixed_period)
        .rollover(
            gov.map_or(
                bc_models::RolloverPolicy::ResetToZero,
                bc_models::BudgetRevision::rollover,
            )
            .into(),
        )
        .maybe_tag_filter(gov.and_then(|r| r.tag_filter()).map(ToString::to_string))
        .is_tracking_only(gov.is_none_or(bc_models::BudgetRevision::is_tracking_only))
        .children(children)
        .build()
}

/// Returns a short lowercase label for a [`bc_models::Period`] variant.
fn period_label(period: &bc_models::Period) -> String {
    match period {
        bc_models::Period::Weekly => "weekly".to_owned(),
        bc_models::Period::Fortnightly { .. } => "fortnightly".to_owned(),
        bc_models::Period::Monthly => "monthly".to_owned(),
        bc_models::Period::Quarterly => "quarterly".to_owned(),
        bc_models::Period::CalendarYear => "calendar year".to_owned(),
        bc_models::Period::FinancialYear { .. } => "financial year".to_owned(),
        bc_models::Period::FinancialQuarter { .. } => "financial quarter".to_owned(),
        bc_models::Period::Custom { .. } => "custom".to_owned(),
        p => {
            tracing::warn!(period = ?p, "unrecognised period type in period_label; falling back to \"period\"");
            "period".to_owned()
        }
    }
}

// MARK: Budget summary

/// Builds the IPC budget summary header from a core tree summary.
///
/// When a dominant target commodity is present, totals are expressed in that
/// commodity; otherwise the spent total is only reported for a single-currency
/// overview and left absent for mixed-currency overviews.
impl From<&BudgetTreeSummary> for bc_ipc::BudgetSummary {
    fn from(summary: &BudgetTreeSummary) -> Self {
        let (total_budgeted, total_spent, total_remaining) =
            if let Some(tc) = summary.commodity.as_ref() {
                let budgeted = bc_ipc::Amount::new(summary.total_effective_target, tc.as_str());
                let actuals_in_target = summary
                    .total_actuals
                    .iter()
                    .find(|a| a.commodity() == tc)
                    .map_or(Decimal::ZERO, bc_models::Amount::value);
                let spent = bc_ipc::Amount::new(actuals_in_target, tc.as_str());
                let remaining_val = summary
                    .total_effective_target
                    .checked_sub(actuals_in_target)
                    .unwrap_or(Decimal::ZERO);
                let remaining = bc_ipc::Amount::new(remaining_val, tc.as_str());
                (Some(budgeted), Some(spent), Some(remaining))
            } else {
                let spent = match summary.total_actuals.as_slice() {
                    [single] => Some(bc_ipc::Amount::from(single)),
                    _ => None,
                };
                (None, spent, None)
            };

        let has_mixed = summary.total_actuals.len() > 1;
        bc_ipc::BudgetSummary::new(
            total_budgeted,
            total_spent,
            total_remaining,
            has_mixed,
            summary.overspent_count,
        )
    }
}

// MARK: Native periods

/// Extension trait building a [`bc_ipc::NativePeriodRow`] from a core
/// [`NativePeriodStatus`].
pub trait NativePeriodRowExt {
    /// Builds a native period sub-row from a core status, an already-resolved
    /// display `label`, and the target `commodity` used to express amounts.
    ///
    /// # Arguments
    ///
    /// * `status` - The core native period overlap status.
    /// * `label` - The human-readable label for the row.
    /// * `commodity` - The commodity code used for amount conversion.
    ///
    /// # Returns
    ///
    /// The equivalent IPC native period row.
    fn from_native(status: &NativePeriodStatus, label: impl Into<String>, commodity: &str) -> Self;
}

impl NativePeriodRowExt for bc_ipc::NativePeriodRow {
    fn from_native(status: &NativePeriodStatus, label: impl Into<String>, commodity: &str) -> Self {
        let effective_target = status
            .effective_target
            .map(|t| bc_ipc::Amount::new(t, commodity));
        let spent = bc_ipc::Amount::new(status.actuals, commodity);
        bc_ipc::NativePeriodRow::new(
            label,
            status.overlap.native_start,
            status.overlap.native_end,
            effective_target,
            spent,
        )
    }
}

#[cfg(test)]
#[cfg(feature = "ipc")]
mod tests {
    #[test]
    fn core_bad_data_maps_to_validation() {
        let err = crate::BcError::BadData("cannot reconcile an unbalanced transaction".to_owned());
        let mapped = bc_ipc::BcError::from(err);
        assert!(
            matches!(mapped, bc_ipc::BcError::Validation(_)),
            "BadData must surface as Validation, got {mapped:?}"
        );
    }

    #[test]
    fn core_invalid_input_maps_to_validation() {
        let err = crate::BcError::InvalidInput("two or more elided postings".to_owned());
        assert!(matches!(
            bc_ipc::BcError::from(err),
            bc_ipc::BcError::Validation(_)
        ));
    }

    #[test]
    fn core_not_found_maps_to_not_found() {
        let err = crate::BcError::NotFound("txn-001".to_owned());
        assert!(matches!(
            bc_ipc::BcError::from(err),
            bc_ipc::BcError::NotFound(_)
        ));
    }

    #[test]
    fn marker_conflict_maps_to_validation() {
        let err = crate::BcError::MarkerConflict {
            marker: "$".to_owned(),
            existing: "USD".to_owned(),
        };
        let mapped = bc_ipc::BcError::from(err);
        assert!(
            matches!(mapped, bc_ipc::BcError::Validation(_)),
            "MarkerConflict must surface as Validation, got {mapped:?}"
        );
    }

    #[test]
    fn commodity_in_use_maps_to_validation() {
        let err = crate::BcError::CommodityInUse("used by 3 transactions".to_owned());
        let mapped = bc_ipc::BcError::from(err);
        assert!(
            matches!(mapped, bc_ipc::BcError::Validation(_)),
            "CommodityInUse must surface as Validation, got {mapped:?}"
        );
    }
}
