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
//! `bc_ipc::NativePeriodRow::from_native`, `bc_ipc::AccountNode::from_model`,
//! `bc_ipc::Transaction::from_model_with_accounts`) cannot be expressed as
//! `From`, so they are exposed as extension traits instead; callers bring the
//! trait into scope to use the named constructor.
//!
//! Domain-walking presentation helpers (account-path building, tag resolution)
//! live here rather than in `bc-ipc`, so that crate stays a thin serde contract
//! carrying only basic scalar/enum/`Commodity` conversions behind its `models`
//! feature.

use rust_decimal::Decimal;

use crate::BudgetTreeItem;
use crate::Event;
use crate::NativePeriodStatus;
use crate::budget_tree::BudgetTreeSummary;
use crate::search::AmountQuery;
use crate::search::TransactionQuery;

// MARK: Error mapping

/// Maps a [`crate::BcError`] to its IPC [`bc_ipc::BcError`] counterpart.
///
/// User-facing validation failures (`InvalidInput`, `BadData`, the account/tag
/// rule violations, marker conflicts, commodity-in-use errors, and merge
/// precondition failures) surface as
/// [`bc_ipc::BcError::Validation`] so the UI can render a friendly message;
/// `NotFound` maps to [`bc_ipc::BcError::NotFound`]; everything genuinely
/// internal (database, IO, serialisation) becomes [`bc_ipc::BcError::Internal`].
///
/// `NotFound` carries only its inner payload — not the full `Display` string —
/// because [`bc_ipc::BcError::NotFound`] already prepends its own `"not found:"`
/// prefix; passing `e.to_string()` would duplicate it.
impl From<crate::BcError> for bc_ipc::BcError {
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "crate::BcError is #[non_exhaustive]; catch-all required for future variants"
    )]
    fn from(e: crate::BcError) -> Self {
        use crate::BcError as Core;

        match &e {
            Core::NotFound(id) => bc_ipc::BcError::NotFound(id.clone()),
            Core::InvalidInput(_)
            | Core::BadData(_)
            | Core::AlreadyArchived(_)
            | Core::InvalidAccountKind { .. }
            | Core::TagInUse(_)
            | Core::MarkerConflict { .. }
            | Core::CommodityInUse(_)
            | Core::NotMergeable { .. }
            | Core::NotMerged(_) => bc_ipc::BcError::Validation(e.to_string()),
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
    /// * `account_names` - Resolved account names keyed by account ID, used to
    ///   render human-readable account references (e.g. for
    ///   [`Event::TransactionSourceAttached`]) instead of raw internal IDs.
    ///   Account IDs absent from this map render as `"unknown account"`.
    ///
    /// # Returns
    ///
    /// A [`bc_ipc::AuditEntry`] with a short kind tag and a human-readable
    /// message.
    #[must_use]
    fn from_event(
        ts: jiff::Timestamp,
        event: &Event,
        account_names: &std::collections::HashMap<bc_models::AccountId, String>,
    ) -> Self;
}

impl AuditEntryExt for bc_ipc::AuditEntry {
    #[inline]
    #[expect(
        clippy::wildcard_enum_match_arm,
        reason = "Event is #[non_exhaustive]; catch-all arm required for exhaustiveness against future variants"
    )]
    fn from_event(
        ts: jiff::Timestamp,
        event: &Event,
        account_names: &std::collections::HashMap<bc_models::AccountId, String>,
    ) -> Self {
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
            Event::TransactionSourceAttached {
                account_id,
                narration,
                ..
            } => {
                let name = account_names
                    .get(account_id)
                    .map_or("unknown account", String::as_str);
                ("import", format!("imported from {name}: {narration}"))
            }
            Event::TransactionSourceDetached { .. } => ("import", "source removed".to_owned()),
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
    #[must_use]
    fn from_native(status: &NativePeriodStatus, label: impl Into<String>, commodity: &str) -> Self;
}

impl NativePeriodRowExt for bc_ipc::NativePeriodRow {
    #[inline]
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

// MARK: Account path helpers

/// Builds a display path for an account by walking up the parent chain.
///
/// Returns a `" :: "`-separated path from the root ancestor down to the account
/// (e.g. `"Assets :: Smart Access"`). Falls back to `account_id` if the account
/// is not present in the map.
///
/// # Arguments
///
/// * `account_id` - ID string of the account to resolve.
/// * `account_map` - Map from ID string to account reference.
fn build_account_path(
    account_id: &str,
    account_map: &std::collections::HashMap<String, &bc_models::Account>,
) -> String {
    let mut parts = Vec::new();
    let mut current = account_id.to_owned();
    let mut visited = std::collections::HashSet::new();

    loop {
        if !visited.insert(current.clone()) {
            break;
        }
        let Some(account) = account_map.get(&current) else {
            break;
        };
        parts.push(account.name().to_owned());
        match account.parent_id() {
            Some(parent) => current = parent.to_string(),
            None => break,
        }
    }

    parts.reverse();
    if parts.is_empty() {
        account_id.to_owned()
    } else {
        parts.join(" :: ")
    }
}

/// Resolves a slice of tag IDs to colon-joined path strings, dropping any ID that
/// is absent from `forest`. Order is preserved; duplicates by path are removed.
///
/// # Arguments
///
/// * `forest` - The loaded tag hierarchy.
/// * `ids` - The tag IDs to resolve.
fn resolve_tag_paths(forest: &bc_models::TagForest, ids: &[bc_models::TagId]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .filter_map(|id| forest.path_of(id).map(|p| p.to_string()))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

// MARK: Account nodes

/// Extension trait building a [`bc_ipc::AccountNode`] from a domain account.
pub trait AccountNodeExt {
    /// Builds an [`bc_ipc::AccountNode`] from a domain account with a
    /// pre-computed balance, resolving tag IDs to display paths via `forest`.
    ///
    /// The balance is supplied by the caller (typically fetched in a separate
    /// batch query) rather than computed here.
    ///
    /// # Arguments
    ///
    /// * `account` - The account to convert.
    /// * `forest` - The loaded tag hierarchy used to resolve account tag IDs to paths.
    /// * `balance` - The pre-computed balance for this account.
    ///
    /// # Returns
    ///
    /// The equivalent IPC account node.
    #[must_use]
    fn from_model(
        account: &bc_models::Account,
        forest: &bc_models::TagForest,
        balance: Option<bc_ipc::Amount>,
    ) -> Self;
}

impl AccountNodeExt for bc_ipc::AccountNode {
    #[inline]
    fn from_model(
        account: &bc_models::Account,
        forest: &bc_models::TagForest,
        balance: Option<bc_ipc::Amount>,
    ) -> Self {
        Self::new(
            account.id().to_string(),
            account.name(),
            None::<&str>,
            balance,
            account.parent_id().map(ToString::to_string),
            account.account_type().into(),
            resolve_tag_paths(forest, account.tag_ids()),
        )
    }
}

// MARK: Transactions

/// Extension trait building a [`bc_ipc::Transaction`] from a domain transaction.
pub trait TransactionExt {
    /// Builds a [`bc_ipc::Transaction`] from a domain transaction, resolving
    /// posting account names from `account_map` and tag IDs to paths via
    /// `forest`.
    ///
    /// The effective tags for each posting are the union of the transaction's
    /// own tags and the posting's own tags, deduplicated by resolved path.
    ///
    /// # Arguments
    ///
    /// * `tx` - The transaction to convert.
    /// * `account_map` - Map from account ID string to account reference.
    /// * `forest` - The loaded tag hierarchy used to resolve tag IDs to paths.
    ///
    /// # Returns
    ///
    /// The equivalent IPC transaction.
    #[must_use]
    fn from_model_with_accounts(
        tx: &bc_models::Transaction,
        account_map: &std::collections::HashMap<String, &bc_models::Account>,
        forest: &bc_models::TagForest,
    ) -> Self;
}

impl TransactionExt for bc_ipc::Transaction {
    #[inline]
    fn from_model_with_accounts(
        tx: &bc_models::Transaction,
        account_map: &std::collections::HashMap<String, &bc_models::Account>,
        forest: &bc_models::TagForest,
    ) -> Self {
        let tx_tag_ids = tx.tag_ids();
        let postings = tx
            .postings()
            .iter()
            .map(|p| {
                let account_id = p.account_id().to_string();
                let account_name = build_account_path(&account_id, account_map);
                let amount = p.amount().map(bc_ipc::Amount::from);
                bc_ipc::Posting::new(
                    p.id().to_string(),
                    bc_ipc::AccountRef::new(account_id, account_name),
                    amount,
                    p.note(),
                    resolve_tag_paths(forest, &tx.effective_tag_ids(p)),
                    p.spread_from(),
                    p.spread_until(),
                )
            })
            .collect();

        let extra_dates = tx
            .extra_dates()
            .iter()
            .map(|(label, date)| (label.clone(), *date))
            .collect();

        Self::new(
            tx.id().to_string(),
            tx.date(),
            tx.payee().unwrap_or_default(),
            tx.description(),
            tx.note(),
            extra_dates,
            tx.reconciliation().into(),
            resolve_tag_paths(forest, tx_tag_ids),
            postings,
            vec![],
        )
    }
}

// MARK: Transfer suggestions

impl From<&crate::TransferSuggestion> for bc_ipc::TransferSuggestion {
    /// Converts a core transfer suggestion into its IPC DTO.
    #[inline]
    fn from(s: &crate::TransferSuggestion) -> Self {
        Self::new(
            s.debit().to_string(),
            s.credit().to_string(),
            bc_ipc::Amount::from(&s.amount),
            s.date_debit.to_string(),
            s.date_credit.to_string(),
            s.debit_account.clone(),
            s.credit_account.clone(),
            s.debit_narration.clone(),
            s.credit_narration.clone(),
        )
    }
}

// MARK: Transaction query

/// Parses an IPC [`bc_ipc::Filter`] into a domain-typed [`TransactionQuery`].
///
/// Account and tag id strings are parsed into their typed ids; a malformed id
/// fails the whole conversion with [`crate::BcError::BadData`].
impl TryFrom<bc_ipc::Filter> for TransactionQuery {
    type Error = crate::BcError;

    fn try_from(f: bc_ipc::Filter) -> Result<Self, Self::Error> {
        let accounts = f
            .accounts
            .iter()
            .map(|s| {
                s.parse::<bc_models::AccountId>()
                    .map_err(|e| crate::BcError::BadData(format!("invalid account id '{s}': {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let tags = f
            .tags
            .iter()
            .map(|s| {
                s.parse::<bc_models::TagId>()
                    .map_err(|e| crate::BcError::BadData(format!("invalid tag id '{s}': {e}")))
            })
            .collect::<Result<Vec<_>, _>>()?;

        let amount = f.amount.map(|a| AmountQuery {
            min: a.min,
            max: a.max,
            commodity: a.commodity.map(bc_models::CommodityCode::new),
        });

        Ok(TransactionQuery {
            date_from: f.date_from,
            date_until: f.date_until,
            accounts,
            tags,
            text: f.text,
            amount,
            reconciliation: f.reconciliation.map(Into::into),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use jiff::Timestamp;
    use pretty_assertions::assert_eq;

    use crate::budget_tree::BudgetTreeSummary;
    use crate::ipc::AuditEntryExt as _;

    #[test]
    fn transfer_suggestion_converts_to_ipc_dto() {
        let debit = bc_models::TransactionId::new();
        let credit = bc_models::TransactionId::new();
        let suggestion = crate::TransferSuggestion {
            debit: debit.clone(),
            credit: credit.clone(),
            amount: bc_models::Amount::new(rust_decimal::Decimal::new(10000, 2), "AUD"),
            date_debit: jiff::civil::date(2025, 6, 26),
            date_credit: jiff::civil::date(2025, 6, 27),
            debit_account: "Savings".to_owned(),
            credit_account: "Mortgage".to_owned(),
            debit_narration: "TFR OUT".to_owned(),
            credit_narration: "TFR IN".to_owned(),
        };
        let dto = bc_ipc::TransferSuggestion::from(&suggestion);
        assert_eq!(dto.debit, debit.to_string());
        assert_eq!(dto.credit, credit.to_string());
        assert_eq!(dto.amount.value(), rust_decimal::Decimal::new(10000, 2));
        assert_eq!(dto.amount.currency_code, "AUD");
        assert_eq!(dto.date_debit, "2025-06-26");
        assert_eq!(dto.date_credit, "2025-06-27");
        assert_eq!(dto.debit_account, "Savings");
        assert_eq!(dto.credit_account, "Mortgage");
        assert_eq!(dto.debit_narration, "TFR OUT");
        assert_eq!(dto.credit_narration, "TFR IN");
    }

    #[test]
    fn audit_entry_from_recategorise_uses_recat_kind() {
        let event = crate::Event::PostingRecategorised {
            id: bc_models::TransactionId::new(),
            posting_id: bc_models::PostingId::new(),
            from_account: bc_models::AccountId::new(),
            to_account: bc_models::AccountId::new(),
        };
        let entry = bc_ipc::AuditEntry::from_event(jiff::Timestamp::now(), &event, &HashMap::new());
        assert_eq!(entry.kind, "recat");
        assert!(!entry.message.is_empty());
    }

    #[test]
    fn source_attached_renders_import_audit_entry() {
        let account = bc_models::AccountId::new();
        let event = crate::Event::TransactionSourceAttached {
            id: bc_models::SourceRefId::new(),
            transaction_id: bc_models::TransactionId::new(),
            account_id: account.clone(),
            date: jiff::civil::date(2025, 6, 27),
            narration: "ACME".to_owned(),
            amount: bc_models::Amount::new(rust_decimal::Decimal::from(100_i32), "AUD"),
            reference: None,
            occurrence: 0,
        };
        let mut account_names = HashMap::new();
        account_names.insert(account.clone(), "Everyday Transaction".to_owned());
        let entry = bc_ipc::AuditEntry::from_event(jiff::Timestamp::now(), &event, &account_names);
        assert_eq!(entry.kind, "import");
        assert!(
            entry.message.contains("Everyday Transaction"),
            "message names the account, got: {}",
            entry.message
        );
        assert!(
            entry.message.contains("ACME"),
            "message names the narration"
        );
        assert!(
            !entry.message.contains(&account.to_string()),
            "message must not leak the raw account id"
        );
    }

    #[test]
    fn budget_summary_from_mixed_currency_tree_has_no_total_spent() {
        let summary = BudgetTreeSummary {
            total_effective_target: rust_decimal::Decimal::ZERO,
            total_actuals: vec![
                bc_models::Amount::new(rust_decimal::Decimal::from(10_i32), "USD"),
                bc_models::Amount::new(rust_decimal::Decimal::from(20_i32), "EUR"),
            ],
            commodity: None,
            overspent_count: 0,
        };

        let ipc_summary = bc_ipc::BudgetSummary::from(&summary);

        assert_eq!(ipc_summary.total_spent, None);
        assert!(ipc_summary.has_mixed_commodities);
    }

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
    fn core_merge_errors_map_to_validation() {
        let not_mergeable = crate::BcError::NotMergeable {
            reason: "commodities differ".to_owned(),
        };
        let not_merged = crate::BcError::NotMerged(bc_models::TransactionId::new());

        assert!(
            matches!(
                bc_ipc::BcError::from(not_mergeable),
                bc_ipc::BcError::Validation(_)
            ),
            "NotMergeable must surface as Validation"
        );
        assert!(
            matches!(
                bc_ipc::BcError::from(not_merged),
                bc_ipc::BcError::Validation(_)
            ),
            "NotMerged must surface as Validation"
        );
    }

    #[test]
    fn core_not_found_carries_bare_id_without_double_prefix() {
        let err = crate::BcError::NotFound("txn-001".to_owned());
        let mapped = bc_ipc::BcError::from(err);
        // The payload is the bare id — the ipc `Display` adds the sole
        // `"not found:"` prefix, so the rendered string must not repeat it.
        assert!(matches!(&mapped, bc_ipc::BcError::NotFound(id) if id == "txn-001"));
        assert_eq!(mapped.to_string(), "not found: txn-001");
    }

    #[test]
    fn marker_conflict_maps_to_validation_with_message() {
        let err = crate::BcError::MarkerConflict {
            marker: "$".to_owned(),
            existing: "USD".to_owned(),
        };
        let mapped = bc_ipc::BcError::from(err);
        assert!(
            matches!(&mapped, bc_ipc::BcError::Validation(msg)
                if msg == "marker conflict: '$' already maps to USD"),
            "MarkerConflict must surface as Validation with variant wording, got {mapped:?}"
        );
    }

    #[test]
    fn commodity_in_use_maps_to_validation_with_message() {
        let err = crate::BcError::CommodityInUse("used by 3 transactions".to_owned());
        let mapped = bc_ipc::BcError::from(err);
        assert!(
            matches!(&mapped, bc_ipc::BcError::Validation(msg)
                if msg == "commodity in use: used by 3 transactions"),
            "CommodityInUse must surface as Validation with variant wording, got {mapped:?}"
        );
    }

    #[test]
    fn resolve_tag_paths_renders_hierarchy_and_dedupes() {
        let person = bc_models::TagId::new();
        let josh = bc_models::TagId::new();
        let forest = bc_models::TagForest::new(vec![
            bc_models::Tag::builder()
                .id(person.clone())
                .name("person")
                .created_at(Timestamp::now())
                .build(),
            bc_models::Tag::builder()
                .id(josh.clone())
                .name("josh")
                .parent_id(person.clone())
                .created_at(Timestamp::now())
                .build(),
        ]);
        let paths =
            super::resolve_tag_paths(&forest, &[josh.clone(), josh.clone(), person.clone()]);
        assert_eq!(paths, vec!["person:josh".to_owned(), "person".to_owned()]);
    }

    #[test]
    fn build_account_path_returns_name_for_root_account() {
        let account = bc_models::Account::builder()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .build();

        let account_id = account.id().to_string();
        let map = HashMap::from([(account_id.clone(), &account)]);

        assert_eq!(super::build_account_path(&account_id, &map), "Checking");
    }

    #[test]
    fn build_account_path_returns_hierarchical_path() {
        let parent = bc_models::Account::builder()
            .name("Assets")
            .account_type(bc_models::AccountType::Asset)
            .build();

        let child = bc_models::Account::builder()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .parent_id(parent.id().clone())
            .build();

        let map = HashMap::from([
            (parent.id().to_string(), &parent),
            (child.id().to_string(), &child),
        ]);

        assert_eq!(
            super::build_account_path(&child.id().to_string(), &map),
            "Assets :: Checking"
        );
    }

    #[test]
    fn build_account_path_falls_back_to_id_when_not_found() {
        let map: HashMap<String, &bc_models::Account> = HashMap::new();
        let fake_id = "account_00000000000000000000000000";
        assert_eq!(super::build_account_path(fake_id, &map), fake_id);
    }
}
