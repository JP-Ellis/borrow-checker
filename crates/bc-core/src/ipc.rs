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

use bc_query::Catalog as _;
use bc_query::catalog::MetaType as QueryType;
use bc_query::catalog::PathEntry;

use crate::BudgetTreeItem;
use crate::Event;
use crate::NativePeriodStatus;
use crate::budget_tree::BudgetTreeSummary;
use crate::metadata::registry::entry_noun;
use crate::search::DbCatalog;
use crate::search::TransactionQuery;

// MARK: Error mapping

/// Maps a [`crate::BcError`] to its IPC [`bc_ipc::BcError`] counterpart.
///
/// User-facing validation failures (`InvalidInput`, `BadData`, the account/tag
/// rule violations, marker conflicts, commodity-in-use errors, and merge
/// precondition failures) surface as
/// [`bc_ipc::BcError::Validation`] so the UI can render a friendly message;
/// `NotFound` maps to [`bc_ipc::BcError::NotFound`]; `Query` maps to
/// [`bc_ipc::BcError::Query`] with each error diagnostic's span; everything genuinely
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
            | Core::AlreadyClosed(_)
            | Core::NotClosed(_)
            | Core::InvalidAccountKind { .. }
            | Core::TagInUse(_)
            | Core::MarkerConflict { .. }
            | Core::CommodityInUse(_)
            | Core::NotMergeable { .. }
            | Core::NotMerged(_)
            | Core::NotUnmergeable { .. } => bc_ipc::BcError::Validation(e.to_string()),
            Core::Query(diagnostics) => bc_ipc::BcError::Query(
                diagnostics
                    .iter()
                    .map(|d| bc_ipc::QueryProblem::new(d.message.clone(), d.span.start, d.span.end))
                    .collect(),
            ),
            Core::Conflict(_) => bc_ipc::BcError::Conflict(e.to_string()),
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
    #[expect(
        clippy::too_many_lines,
        reason = "one match arm per event variant; splitting it would scatter the audit vocabulary"
    )]
    fn from_event(
        ts: jiff::Timestamp,
        event: &Event,
        account_names: &std::collections::HashMap<bc_models::AccountId, String>,
    ) -> Self {
        let (kind, message): (&str, String) = match event {
            Event::TransactionCreated { .. } => ("create", "transaction created".to_owned()),
            Event::TransactionDeleted {
                kept_provenance, ..
            } => (
                "delete",
                if *kept_provenance {
                    "transaction deleted; a re-import skips its statement rows".to_owned()
                } else {
                    "transaction deleted".to_owned()
                },
            ),
            Event::TransactionReversed { reversal_id, .. } => {
                ("reverse", format!("reversed by {reversal_id}"))
            }
            Event::TransactionDateChanged { to, .. } => ("date", format!("date → {to}")),
            Event::TransactionDescriptionChanged { .. } => {
                ("desc", "description changed".to_owned())
            }
            Event::TransactionTagsChanged { added, removed, .. } => {
                ("tags", format!("tags +{} -{}", added.len(), removed.len()))
            }
            Event::TransactionMetadataChanged { before, after, .. } => (
                "meta",
                format!("metadata {} → {} entries", before.len(), after.len()),
            ),
            Event::TransactionReconciled { from, to, .. } => {
                ("reconcile", format!("reconciliation {from:?} → {to:?}"))
            }
            Event::PostingRecategorised { to_account, .. } => {
                ("recat", format!("recategorised → {to_account}"))
            }
            Event::PostingAmountChanged { .. } => ("amount", "amount changed".to_owned()),
            Event::PostingMetadataChanged { before, after, .. } => (
                "meta",
                format!(
                    "posting metadata {} → {} entries",
                    before.len(),
                    after.len()
                ),
            ),
            Event::PostingTagsChanged { added, removed, .. } => (
                "tags",
                format!("posting tags +{} -{}", added.len(), removed.len()),
            ),
            // The four registry events aggregate on the metadata key, and
            // `TransactionService::audit_trail` selects on a transaction id, so
            // no query returns one today. The copy exists so the first reader
            // of a key's history finds a sentence rather than a variant name.
            Event::MetadataKeyRegistered { key, ty } => (
                "key",
                format!(
                    "metadata key '{key}' registered as {}",
                    bc_ipc::MetaTypeDto::from(*ty).label()
                ),
            ),
            Event::MetadataKeyRetyped { key, from, to } => (
                "key",
                format!(
                    "metadata key '{key}' retyped {} → {}",
                    bc_ipc::MetaTypeDto::from(*from).label(),
                    bc_ipc::MetaTypeDto::from(*to).label()
                ),
            ),
            Event::MetadataKeyRenamed { from, to } => {
                ("key", format!("metadata key '{from}' renamed to '{to}'"))
            }
            Event::MetadataKeyDeleted { key, ty, entries } => (
                "key",
                format!(
                    "metadata key '{key}' ({}) deleted with {entries} {}",
                    bc_ipc::MetaTypeDto::from(*ty).label(),
                    entry_noun(*entries)
                ),
            ),
            Event::PostingSpreadChanged { to, .. } => (
                "spread",
                match to {
                    Some((from, until)) => format!("spread {from}..{until}"),
                    None => "spread cleared".to_owned(),
                },
            ),
            Event::PostingAnnotationChanged {
                price_from,
                price_to,
                cost_from,
                cost_to,
                ..
            } => (
                "annotation",
                annotation_message(
                    price_from.as_ref(),
                    price_to.as_ref(),
                    cost_from.as_ref(),
                    cost_to.as_ref(),
                ),
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
            Event::ImportBatchDiscarded {
                removed_postings,
                removed_transactions,
                removed_accounts,
                reverted_fields,
                ..
            } => {
                let mut parts = vec![format!(
                    "{removed_postings} postings, {removed_transactions} transactions removed"
                )];
                if *removed_accounts > 0 {
                    parts.push(format!("{removed_accounts} accounts removed"));
                }
                if *reverted_fields > 0 {
                    parts.push(format!("{reverted_fields} account fields cleared"));
                }
                ("import", format!("import discarded: {}", parts.join("; ")))
            }
            other => {
                let k = other.kind();
                (k, k.to_owned())
            }
        };
        bc_ipc::AuditEntry::new(ts, kind.to_owned(), message)
    }
}

/// Renders a [`Event::PostingAnnotationChanged`] for the audit log: each
/// half that changed, in Beancount form (`price @ 1.50 AUD`, `cost {105
/// AUD}`), or `cleared` for a half that was removed.
fn annotation_message(
    price_from: Option<&bc_models::Quote>,
    price_to: Option<&bc_models::Quote>,
    cost_from: Option<&bc_models::Cost>,
    cost_to: Option<&bc_models::Cost>,
) -> String {
    let mut parts = Vec::with_capacity(2);
    if price_from != price_to {
        parts.push(match price_to {
            Some(q) => format!("price {}", render_quote(q, "@", "@@")),
            None => "price cleared".to_owned(),
        });
    }
    if cost_from != cost_to {
        parts.push(match cost_to {
            Some(c) => format!("cost {}", render_cost(c)),
            None => "cost cleared".to_owned(),
        });
    }
    if parts.is_empty() {
        return "price or cost changed".to_owned();
    }
    parts.join(", ")
}

/// Renders a quote as `<marker> <value> <commodity>`, choosing `per_unit` or
/// `total` as the marker by the quote's form.
fn render_quote(quote: &bc_models::Quote, per_unit: &str, total: &str) -> String {
    let marker = if quote.is_total() { total } else { per_unit };
    let amount = quote.amount();
    format!("{marker} {} {}", amount.value(), amount.commodity())
}

/// Renders a cost basis as a Beancount cost block: `{105 AUD, 2024-03-01,
/// "lot-a"}`, doubled braces for a total.
fn render_cost(cost: &bc_models::Cost) -> String {
    let basis = cost.basis();
    let amount = basis.amount();
    let mut parts = vec![format!("{} {}", amount.value(), amount.commodity())];
    if let Some(date) = cost.date() {
        parts.push(date.to_string());
    }
    if let Some(label) = cost.label() {
        parts.push(format!("\"{label}\""));
    }
    let inner = parts.join(", ");
    if basis.is_total() {
        format!("{{{{{inner}}}}}")
    } else {
        format!("{{{inner}}}")
    }
}

// MARK: Budget tree

impl From<crate::RowKind> for bc_ipc::RowKind {
    #[inline]
    fn from(kind: crate::RowKind) -> Self {
        match kind {
            crate::RowKind::Budget => Self::Budget,
            crate::RowKind::Account => Self::Account,
            crate::RowKind::Unallocated => Self::Unallocated,
            crate::RowKind::Unbudgeted => Self::Unbudgeted,
        }
    }
}

/// Converts a [`BudgetTreeItem`] (and its children, recursively) into a
/// [`bc_ipc::BudgetTreeNode`].
impl From<&BudgetTreeItem> for bc_ipc::BudgetTreeNode {
    fn from(item: &BudgetTreeItem) -> Self {
        budget_tree_node_recursive(item)
    }
}

/// Recursive implementation of the `From<&BudgetTreeItem>` conversion above.
fn budget_tree_node_recursive(item: &BudgetTreeItem) -> bc_ipc::BudgetTreeNode {
    let gov = item.governing.as_ref();
    let native_period_label = gov.map_or_else(|| "period".to_owned(), |r| period_label(r.period()));
    let default_intent = bc_models::BudgetIntent::default_for(item.account.account_type());

    bc_ipc::BudgetTreeNode::builder()
        .id(item.id.clone())
        .kind(item.kind.into())
        .account_id(item.account.id().to_string())
        .label(item.label.clone())
        .maybe_tag_filter(
            item.tag_filter
                .as_ref()
                .map(|(id, path)| bc_ipc::TagInfo::new(id.to_string(), path.clone())),
        )
        .maybe_actual(item.actual.as_ref().map(bc_ipc::Amount::from))
        .maybe_target(item.target.as_ref().map(bc_ipc::Amount::from))
        .maybe_target_expr(gov.and_then(|r| r.target_expr()).map(ToOwned::to_owned))
        .maybe_intent(item.intent.map(Into::into))
        .default_intent(default_intent.into())
        .claimed(item.claimed)
        .unallocated(item.unallocated)
        .unbudgeted(item.unbudgeted)
        .maybe_verdict(item.verdict.map(Into::into))
        .maybe_ratio(item.ratio)
        .maybe_worst_descendant(item.worst_descendant.map(Into::into))
        .mixed(item.mixed)
        .double_counted(item.double_counted)
        .over_allocated(item.over_allocated)
        .sign_flip(item.sign_flip)
        .native_period_label(native_period_label)
        .has_mixed_period(item.has_mixed_period)
        .maybe_rollover(gov.map(|r| r.rollover().into()))
        .children(
            item.children
                .iter()
                .map(budget_tree_node_recursive)
                .collect(),
        )
        .unvalued(balances_to_amounts(&item.unvalued))
        .build()
}

/// Renders a [`bc_models::Balances`] as IPC amounts in first-seen order.
fn balances_to_amounts(balances: &bc_models::Balances) -> Vec<bc_ipc::Amount> {
    balances
        .iter()
        .map(|(code, value)| bc_ipc::Amount::new(value, code))
        .collect()
}

/// Returns a short lowercase label for a [`bc_models::Period`] variant.
fn period_label(period: &bc_models::Period) -> String {
    match period {
        bc_models::Period::Daily => "daily".to_owned(),
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

/// Builds the IPC budget summary header from a core tree summary, naming each
/// unbudgeted type root by its account name.
impl From<&BudgetTreeSummary> for bc_ipc::BudgetSummary {
    fn from(summary: &BudgetTreeSummary) -> Self {
        bc_ipc::BudgetSummary::builder()
            .red(summary.red)
            .warn(summary.warn)
            .green(summary.green)
            .unbudgeted(
                summary
                    .unbudgeted
                    .iter()
                    .map(|(_, name, total)| (name.clone(), bc_ipc::Amount::from(total)))
                    .collect(),
            )
            .has_unvalued(summary.has_unvalued)
            .build()
    }
}

/// Builds the IPC budget overview from a core overview.
impl From<&crate::BudgetOverview> for bc_ipc::BudgetOverview {
    fn from(overview: &crate::BudgetOverview) -> Self {
        bc_ipc::BudgetOverview::builder()
            .summary(bc_ipc::BudgetSummary::from(&overview.summary))
            .nodes(
                overview
                    .nodes
                    .iter()
                    .map(bc_ipc::BudgetTreeNode::from)
                    .collect(),
            )
            .maybe_elapsed_fraction(overview.elapsed_fraction)
            .build()
    }
}

// MARK: Native periods

/// Extension trait building a [`bc_ipc::NativePeriodRow`] from a core
/// [`NativePeriodStatus`].
pub trait NativePeriodRowExt {
    /// Builds a native period sub-row from a core status and an
    /// already-resolved display `label`.
    ///
    /// # Arguments
    ///
    /// * `status` - The core native period overlap status.
    /// * `label` - The human-readable label for the row.
    ///
    /// # Returns
    ///
    /// The equivalent IPC native period row, its amounts in the status's
    /// commodity. Without a commodity the row has no target and no spend.
    #[must_use]
    fn from_native(status: &NativePeriodStatus, label: impl Into<String>) -> Self;
}

impl NativePeriodRowExt for bc_ipc::NativePeriodRow {
    #[inline]
    fn from_native(status: &NativePeriodStatus, label: impl Into<String>) -> Self {
        let in_commodity = |value| {
            status
                .commodity
                .as_ref()
                .map(|c| bc_ipc::Amount::new(value, c.as_str()))
        };
        bc_ipc::NativePeriodRow::new(
            label,
            status.overlap.native_start,
            status.overlap.native_end,
            status.effective_target.and_then(in_commodity),
            in_commodity(status.actuals),
            balances_to_amounts(&status.unvalued),
            status.verdict.map(bc_ipc::Verdict::from),
            status.ratio,
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

/// Resolves tag IDs to [`bc_ipc::TagInfo`]s, dropping any ID absent from
/// `forest`. Order is preserved; duplicate IDs are removed.
///
/// # Arguments
///
/// * `forest` - The loaded tag hierarchy.
/// * `ids` - The tag IDs to resolve.
fn resolve_tags(forest: &bc_models::TagForest, ids: &[bc_models::TagId]) -> Vec<bc_ipc::TagInfo> {
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .filter(|id| seen.insert(*id))
        .filter_map(|id| {
            forest
                .path_of(id)
                .map(|p| bc_ipc::TagInfo::new(id.to_string(), p.to_string()))
        })
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
            resolve_tags(forest, account.tag_ids()),
            account.opened_on(),
            account.closed_on(),
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
        let residual = crate::residual::residual_of_postings(tx.postings());
        let postings = tx
            .postings()
            .iter()
            .map(|p| {
                let account_id = p.account_id().to_string();
                let account_name = build_account_path(&account_id, account_map);
                let amount = match p.amount() {
                    Some(a) => bc_ipc::PostingAmount::Stored(bc_ipc::Amount::from(a)),
                    None => match residual {
                        Ok(crate::residual::Residual::Attributable(ref balances)) => {
                            bc_ipc::PostingAmount::Derived(
                                balances
                                    .iter()
                                    .map(|(code, value)| bc_ipc::Amount::new(value, code))
                                    .collect(),
                            )
                        }
                        // Two or more elided legs, or a residual that overflowed
                        // Decimal — neither is attributable to this leg.
                        Ok(_) | Err(_) => bc_ipc::PostingAmount::Ambiguous,
                    },
                };
                bc_ipc::Posting::new(
                    p.id().to_string(),
                    bc_ipc::AccountRef::new(account_id, account_name),
                    amount,
                    p.metadata()
                        .iter()
                        .map(bc_ipc::MetaEntryDto::from)
                        .collect(),
                    resolve_tags(forest, p.tag_ids()),
                    p.spread_from(),
                    p.spread_until(),
                )
                .with_price(p.price().map(bc_ipc::Quote::from))
                .with_cost(p.cost().map(bc_ipc::Cost::from))
            })
            .collect();

        Self::new(
            tx.id().to_string(),
            tx.date(),
            tx.description(),
            tx.metadata()
                .iter()
                .map(bc_ipc::MetaEntryDto::from)
                .collect(),
            tx.reconciliation().into(),
            resolve_tags(forest, tx_tag_ids),
            postings,
            vec![],
            tx.balanced(),
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

// MARK: Query catalog

impl From<&DbCatalog> for bc_ipc::QueryCatalog {
    /// Converts the database catalog into the palette's copy.
    #[inline]
    fn from(catalog: &DbCatalog) -> Self {
        let path =
            |entry: &PathEntry| bc_ipc::CatalogPath::new(entry.id.clone(), entry.path.clone());
        Self::new(
            catalog.accounts().iter().map(path).collect(),
            catalog.tags().iter().map(path).collect(),
            catalog
                .commodities()
                .iter()
                .map(|c| {
                    bc_ipc::CatalogCommodity::new(
                        c.code.clone(),
                        c.symbol.clone(),
                        c.aliases.clone(),
                    )
                })
                .collect(),
            catalog
                .meta_keys()
                .iter()
                .map(|k| {
                    bc_ipc::CatalogKey::new(
                        k.key.clone(),
                        meta_type_dto(k.ty),
                        u64::try_from(k.mismatched).unwrap_or(u64::MAX),
                    )
                })
                .collect(),
            catalog.archived().to_vec(),
        )
    }
}

/// The IPC form of a query catalog key's type.
const fn meta_type_dto(ty: QueryType) -> bc_ipc::MetaTypeDto {
    match ty {
        QueryType::Text => bc_ipc::MetaTypeDto::Text,
        QueryType::Number => bc_ipc::MetaTypeDto::Number,
        QueryType::Boolean => bc_ipc::MetaTypeDto::Boolean,
        QueryType::Date => bc_ipc::MetaTypeDto::Date,
        QueryType::Timestamp => bc_ipc::MetaTypeDto::Timestamp,
        QueryType::Amount => bc_ipc::MetaTypeDto::Amount,
        QueryType::Account => bc_ipc::MetaTypeDto::Account,
    }
}

// MARK: Transaction query

impl crate::transaction::Service {
    /// Parses and resolves an IPC filter's query against the current database,
    /// keeping its date window.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::Query`] for text that does not parse or
    /// resolve, and [`crate::BcError`] if the catalog cannot be loaded.
    pub async fn query_from_filter(
        &self,
        filter: bc_ipc::Filter,
    ) -> crate::BcResult<TransactionQuery> {
        self.parse_query(&filter.query, filter.date_from, filter.date_until)
            .await
    }

    /// Loads every fact query text resolves against, for the palette.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] if the catalog cannot be loaded.
    pub async fn query_catalog(&self) -> crate::BcResult<bc_ipc::QueryCatalog> {
        let catalog = DbCatalog::load(self.pool()).await?;
        Ok(bc_ipc::QueryCatalog::from(&catalog))
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;

    use bc_models::Amount;
    use bc_models::Balances;
    use bc_query::catalog::Catalog as _;
    use bc_query::catalog::MetaKey;
    use bc_query::catalog::MetaType as QueryType;
    use bc_query::catalog::PathEntry;
    use bc_query::catalog::Snapshot;
    use bc_query::currency::Commodity;
    use jiff::Timestamp;
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use serde_json::json;

    use crate::NativePeriodStatus;
    use crate::budget_tree::BudgetTreeItem;
    use crate::budget_tree::BudgetTreeSummary;
    use crate::ipc::AuditEntryExt as _;
    use crate::ipc::NativePeriodRowExt as _;
    use crate::ipc::TransactionExt;
    use crate::period_overlap::PeriodOverlap;

    #[sqlx::test(migrations = "./migrations")]
    async fn a_query_error_crosses_ipc_with_its_span(pool: sqlx::SqlitePool) {
        let svc = crate::transaction::Service::new(pool);
        let ok = svc
            .query_from_filter(bc_ipc::Filter::new(
                "",
                Some(jiff::civil::date(2026, 1, 1)),
                None,
            ))
            .await
            .expect("blank query");
        assert!(ok.expr.is_none());
        assert_eq!(ok.date_from, Some(jiff::civil::date(2026, 1, 1)));

        let err = svc
            .query_from_filter(bc_ipc::Filter::new("@pyee:x", None, None))
            .await
            .expect_err("unknown key");
        let bc_ipc::BcError::Query(problems) = bc_ipc::BcError::from(err) else {
            panic!("a query error maps to BcError::Query");
        };
        assert_eq!(problems.first().map(|p| (p.start, p.end)), Some((0, 5)));
    }

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
        assert_ne!(entry.message, "");
    }

    #[test]
    fn audit_entry_from_posting_tags_changed_counts_each_side() {
        let event = crate::Event::PostingTagsChanged {
            id: bc_models::TransactionId::new(),
            posting_id: bc_models::PostingId::new(),
            added: vec![bc_models::TagId::new(), bc_models::TagId::new()],
            removed: vec![bc_models::TagId::new()],
        };
        let entry = bc_ipc::AuditEntry::from_event(Timestamp::now(), &event, &HashMap::new());
        assert_eq!(entry.kind, "tags");
        assert_eq!(entry.message, "posting tags +2 -1");
    }

    #[test]
    fn annotation_audit_entry_renders_each_changed_half() {
        let cost = bc_models::Cost::builder()
            .basis(bc_models::Quote::Total(Amount::new(dec!(210), "AUD")))
            .date(jiff::civil::date(2024, 3, 1))
            .label("lot-a")
            .build();
        let event = crate::Event::PostingAnnotationChanged {
            id: bc_models::TransactionId::new(),
            posting_id: bc_models::PostingId::new(),
            price_from: Some(bc_models::Quote::PerUnit(Amount::new(dec!(1.50), "AUD"))),
            price_to: None,
            cost_from: None,
            cost_to: Some(cost),
        };
        let entry = bc_ipc::AuditEntry::from_event(Timestamp::now(), &event, &HashMap::new());
        assert_eq!(entry.kind, "annotation");
        assert_eq!(
            entry.message,
            "price cleared, cost {{210 AUD, 2024-03-01, \"lot-a\"}}"
        );
    }

    #[test]
    fn annotation_audit_entry_skips_the_unchanged_half() {
        let price = Some(bc_models::Quote::Total(Amount::new(dec!(6.37), "AUD")));
        let event = crate::Event::PostingAnnotationChanged {
            id: bc_models::TransactionId::new(),
            posting_id: bc_models::PostingId::new(),
            price_from: price.clone(),
            price_to: price,
            cost_from: None,
            cost_to: Some(
                bc_models::Cost::builder()
                    .basis(bc_models::Quote::PerUnit(Amount::new(dec!(105), "AUD")))
                    .build(),
            ),
        };
        let entry = bc_ipc::AuditEntry::from_event(Timestamp::now(), &event, &HashMap::new());
        assert_eq!(entry.message, "cost {105 AUD}");
    }

    #[test]
    fn source_attached_renders_import_audit_entry() {
        let account = bc_models::AccountId::new();
        let event = crate::Event::TransactionSourceAttached {
            id: bc_models::SourceRefId::new(),
            transaction_id: bc_models::TransactionId::new(),
            posting_id: bc_models::PostingId::new(),
            account_id: account.clone(),
            date: jiff::civil::date(2025, 6, 27),
            narration: "ACME".to_owned(),
            amount: Some(bc_models::Amount::new(
                rust_decimal::Decimal::from(100_i32),
                "AUD",
            )),
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
    fn registry_events_render_readable_audit_entries() {
        let key = bc_models::MetaKey::new("invoice").expect("valid key");
        let cases = [
            (
                crate::Event::MetadataKeyRegistered {
                    key: key.clone(),
                    ty: bc_models::MetaType::Number,
                },
                "metadata key 'invoice' registered as number",
            ),
            (
                crate::Event::MetadataKeyRetyped {
                    key: key.clone(),
                    from: bc_models::MetaType::Number,
                    to: bc_models::MetaType::Text,
                },
                "metadata key 'invoice' retyped number → text",
            ),
            (
                crate::Event::MetadataKeyRenamed {
                    from: key.clone(),
                    to: bc_models::MetaKey::new("reference").expect("valid key"),
                },
                "metadata key 'invoice' renamed to 'reference'",
            ),
            (
                crate::Event::MetadataKeyDeleted {
                    key: key.clone(),
                    ty: bc_models::MetaType::Number,
                    entries: 1,
                },
                "metadata key 'invoice' (number) deleted with 1 entry",
            ),
            (
                crate::Event::MetadataKeyDeleted {
                    key: key.clone(),
                    ty: bc_models::MetaType::Text,
                    entries: 0,
                },
                "metadata key 'invoice' (text) deleted with 0 entries",
            ),
        ];
        for (event, expected) in cases {
            let entry =
                bc_ipc::AuditEntry::from_event(jiff::Timestamp::now(), &event, &HashMap::new());
            assert_eq!(entry.kind, "key");
            assert_eq!(entry.message, expected);
        }
    }

    #[rstest]
    #[case::postings_only(0, 0, "import discarded: 6 postings, 4 transactions removed")]
    #[case::accounts(
        2,
        4,
        "import discarded: 6 postings, 4 transactions removed; 2 accounts removed; \
         4 account fields cleared"
    )]
    fn import_batch_discarded_renders_readable_audit_entry(
        #[case] removed_accounts: u64,
        #[case] reverted_fields: u64,
        #[case] expected: &str,
    ) {
        let event = crate::Event::ImportBatchDiscarded {
            batch_id: bc_models::ImportBatchId::new(),
            removed_postings: 6,
            removed_transactions: 4,
            detached_adopted: 3,
            freed_tombstones: 5,
            other_batch_references_removed: 7,
            other_batch_references_tombstoned: 8,
            edited_postings: 1,
            reconciled_postings: 2,
            flagged_postings: 9,
            removed_tags: 3,
            kept_tags: 1,
            removed_accounts,
            kept_accounts: 1,
            reverted_fields,
        };
        let entry = bc_ipc::AuditEntry::from_event(jiff::Timestamp::now(), &event, &HashMap::new());
        assert_eq!(entry.kind, "import");
        assert_eq!(entry.message, expected);
    }

    #[test]
    fn budget_summary_names_unbudgeted_roots() {
        let root = bc_models::AccountId::new();
        let summary = BudgetTreeSummary {
            red: 1,
            warn: 2,
            green: 3,
            unbudgeted: vec![(root, "Expenses".to_owned(), Amount::new(dec!(20), "AUD"))],
            has_unvalued: true,
        };

        let ipc_summary = bc_ipc::BudgetSummary::from(&summary);

        assert_eq!(
            ipc_summary,
            bc_ipc::BudgetSummary::builder()
                .red(1)
                .warn(2)
                .green(3)
                .unbudgeted(vec![(
                    "Expenses".to_owned(),
                    bc_ipc::Amount::new(dec!(20), "AUD")
                )])
                .has_unvalued(true)
                .build()
        );
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
        assert!(
            matches!(
                bc_ipc::BcError::from(crate::BcError::NotUnmergeable {
                    reason: "absorbed leg was removed".to_owned(),
                }),
                bc_ipc::BcError::Validation(_)
            ),
            "NotUnmergeable must surface as Validation"
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

    /// A `person` root with a `person:josh` child, and both IDs.
    fn person_forest() -> (bc_models::TagForest, bc_models::TagId, bc_models::TagId) {
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
        (forest, person, josh)
    }

    #[test]
    fn resolve_tags_pairs_ids_with_paths_and_dedupes_by_id() {
        let (forest, person, josh) = person_forest();
        let out = super::resolve_tags(&forest, &[josh.clone(), josh.clone(), person.clone()]);
        assert_eq!(
            out,
            vec![
                bc_ipc::TagInfo::new(josh.to_string(), "person:josh"),
                bc_ipc::TagInfo::new(person.to_string(), "person"),
            ]
        );
    }

    #[test]
    fn resolve_tags_drops_an_id_missing_from_the_forest() {
        let (forest, person, _) = person_forest();
        let out = super::resolve_tags(&forest, &[bc_models::TagId::new(), person.clone()]);
        assert_eq!(
            out,
            vec![bc_ipc::TagInfo::new(person.to_string(), "person")]
        );
    }

    #[test]
    fn transaction_tags_snapshot() {
        let (forest, person, josh) = person_forest();
        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(jiff::civil::date(2026, 1, 3))
            .description("Supermarket")
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::UNIX_EPOCH)
            .tag_ids(vec![person.clone()])
            .postings(vec![
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .tag_ids(vec![josh.clone()])
                    .build(),
            ])
            .build();

        let dto = <bc_ipc::Transaction as TransactionExt>::from_model_with_accounts(
            &tx,
            &HashMap::new(),
            &forest,
        );

        insta::assert_json_snapshot!(
            json!({
                "tags": dto.tags,
                "posting_tags": dto.postings.first().map(|p| &p.tags),
            }),
            { ".**.id" => "[id]" }
        );
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

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn elided_leg_converts_to_a_derived_amount() {
        let bank = bc_models::AccountId::new();
        let food = bc_models::AccountId::new();
        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(jiff::civil::date(2026, 1, 1))
            .description("Groceries")
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .postings(vec![
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(food)
                    .amount(Amount::new(dec!(50), "AUD"))
                    .metadata(bc_models::Metadata::new(vec![bc_models::MetaEntry::new(
                        bc_models::MetaKey::new("note").expect("valid key"),
                        bc_models::MetaValue::Text("doctor's appointment".to_owned()),
                    )]))
                    .build(),
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(bank)
                    .maybe_amount(None)
                    .build(),
            ])
            .build();

        let accounts = HashMap::new();
        let forest = bc_models::TagForest::default();
        let dto = <bc_ipc::Transaction as TransactionExt>::from_model_with_accounts(
            &tx, &accounts, &forest,
        );

        let bc_ipc::PostingAmount::Derived(residual) = &dto.postings[1].amount else {
            panic!("the elided leg must carry a derived amount");
        };
        assert_eq!(residual.len(), 1);
        assert_eq!(residual[0].value, dec!(-50));
        assert_eq!(residual[0].currency_code, "AUD");

        assert!(matches!(
            dto.postings[0].amount,
            bc_ipc::PostingAmount::Stored(_)
        ));
        assert_eq!(
            dto.postings[0].metadata,
            vec![bc_ipc::MetaEntryDto::new(
                "note",
                bc_ipc::MetaValueDto::Text("doctor's appointment".to_owned())
            )],
            "a posting's metadata crosses the boundary in display order"
        );
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn two_elided_legs_convert_to_ambiguous() {
        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(jiff::civil::date(2026, 1, 1))
            .description("Ambiguous")
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .postings(vec![
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .amount(Amount::new(dec!(50), "AUD"))
                    .build(),
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .maybe_amount(None)
                    .build(),
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(bc_models::AccountId::new())
                    .maybe_amount(None)
                    .build(),
            ])
            .build();

        let accounts = HashMap::new();
        let forest = bc_models::TagForest::default();
        let dto = <bc_ipc::Transaction as TransactionExt>::from_model_with_accounts(
            &tx, &accounts, &forest,
        );

        assert!(matches!(
            dto.postings[1].amount,
            bc_ipc::PostingAmount::Ambiguous
        ));
        assert!(matches!(
            dto.postings[2].amount,
            bc_ipc::PostingAmount::Ambiguous
        ));
    }

    #[test]
    #[expect(clippy::indexing_slicing, reason = "test with known length")]
    fn transaction_ext_carries_price_and_cost() {
        let acct = bc_models::AccountId::new();
        let account = bc_models::Account::builder()
            .id(acct.clone())
            .name("Brokerage")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .build();
        let map = HashMap::from([(acct.to_string(), &account)]);
        let forest = bc_models::TagForest::default();
        let cost = bc_models::Cost::builder()
            .basis(bc_models::Quote::PerUnit(bc_models::Amount::new(
                dec!(105),
                "AUD",
            )))
            .label("lot-a")
            .build();
        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(jiff::civil::Date::constant(2024, 3, 1))
            .description("buy")
            .postings(vec![
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(acct.clone())
                    .amount(Amount::new(dec!(2), "AAPL"))
                    .cost(cost)
                    .price(bc_models::Quote::PerUnit(bc_models::Amount::new(
                        dec!(150),
                        "AUD",
                    )))
                    .build(),
                bc_models::Posting::builder()
                    .id(bc_models::PostingId::new())
                    .account_id(acct.clone())
                    .amount(Amount::new(dec!(-210), "AUD"))
                    .build(),
            ])
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let ipc =
            <bc_ipc::Transaction as TransactionExt>::from_model_with_accounts(&tx, &map, &forest);
        let leg = &ipc.postings[0];
        assert_eq!(
            leg.price,
            Some(bc_ipc::Quote::PerUnit(bc_ipc::Amount::new(
                dec!(150),
                "AUD"
            )))
        );
        assert_eq!(
            leg.cost,
            Some(bc_ipc::Cost::new(
                bc_ipc::Quote::PerUnit(bc_ipc::Amount::new(dec!(105), "AUD")),
                None,
                Some("lot-a".to_owned()),
            ))
        );
        assert_eq!(leg.weight(), Some(bc_ipc::Amount::new(dec!(210), "AUD")));
        assert_eq!(ipc.postings[1].price, None);
    }

    #[test]
    fn period_label_names_daily() {
        assert_eq!(super::period_label(&bc_models::Period::Daily), "daily");
    }

    /// A tree item on an expense account, for node-conversion tests.
    fn item(
        id: &str,
        kind: crate::RowKind,
        unvalued: Balances,
        children: Vec<BudgetTreeItem>,
    ) -> BudgetTreeItem {
        let account = bc_models::Account::builder()
            .name("Widgets")
            .account_type(bc_models::AccountType::Expense)
            .build();
        BudgetTreeItem {
            id: id.to_owned(),
            kind,
            account,
            budget: None,
            governing: None,
            label: "Widgets".to_owned(),
            tag_filter: None,
            actual: Some(Amount::new(dec!(75), "AUD")),
            target: None,
            intent: None,
            claimed: dec!(75),
            unallocated: dec!(0),
            unbudgeted: dec!(0),
            verdict: None,
            ratio: None,
            worst_descendant: None,
            mixed: false,
            double_counted: false,
            over_allocated: false,
            sign_flip: false,
            has_mixed_period: false,
            unvalued,
            postings: Vec::new(),
            children,
        }
    }

    #[test]
    fn budget_tree_node_maps_every_field() {
        let mut budget_row = item("budget_1", crate::RowKind::Budget, Balances::new(), vec![]);
        let alice = bc_models::TagId::new();
        budget_row.tag_filter = Some((alice.clone(), "person:alice".to_owned()));
        budget_row.target = Some(Amount::new(dec!(100), "AUD"));
        budget_row.intent = Some(bc_models::BudgetIntent::Estimate);
        budget_row.verdict = Some(bc_models::Verdict::Warn);
        budget_row.ratio = Some(dec!(0.75));
        budget_row.double_counted = true;
        budget_row.over_allocated = true;
        budget_row.sign_flip = true;
        budget_row.has_mixed_period = true;
        let mut root = item(
            "acct:root",
            crate::RowKind::Account,
            Balances::new(),
            vec![budget_row],
        );
        root.worst_descendant = Some(bc_models::Verdict::Warn);
        root.unallocated = dec!(5);
        root.unbudgeted = dec!(20);
        root.mixed = true;
        root.actual = None;

        let node = bc_ipc::BudgetTreeNode::from(&root);

        assert_eq!(node.id, "acct:root");
        assert_eq!(node.kind, bc_ipc::RowKind::Account);
        assert_eq!(node.actual, None);
        assert_eq!(node.intent, None);
        assert_eq!(node.default_intent, bc_ipc::BudgetIntent::Limit);
        assert_eq!(node.worst_descendant, Some(bc_ipc::Verdict::Warn));
        assert_eq!(node.claimed, dec!(75));
        assert_eq!(node.unallocated, dec!(5));
        assert_eq!(node.unbudgeted, dec!(20));
        assert!(node.mixed);
        assert_eq!(node.native_period_label, "period");
        assert_eq!(node.rollover, None);

        let child = node.children.first().expect("budget child");
        assert_eq!(child.id, "budget_1");
        assert_eq!(child.kind, bc_ipc::RowKind::Budget);
        assert_eq!(child.label, "Widgets");
        assert_eq!(
            child.tag_filter,
            Some(bc_ipc::TagInfo::new(alice.to_string(), "person:alice"))
        );
        assert_eq!(child.actual, Some(bc_ipc::Amount::new(dec!(75), "AUD")));
        assert_eq!(child.target, Some(bc_ipc::Amount::new(dec!(100), "AUD")));
        assert_eq!(child.intent, Some(bc_ipc::BudgetIntent::Estimate));
        assert_eq!(child.verdict, Some(bc_ipc::Verdict::Warn));
        assert_eq!(child.ratio, Some(dec!(0.75)));
        assert!(child.double_counted);
        assert!(child.over_allocated);
        assert!(child.sign_flip);
        assert!(child.has_mixed_period);
    }

    #[test]
    fn budget_tree_node_takes_governing_revision_details() {
        let mut row = item("budget_1", crate::RowKind::Budget, Balances::new(), vec![]);
        row.governing = Some(
            bc_models::BudgetRevision::builder()
                .budget_id(bc_models::BudgetId::new())
                .effective_from(jiff::civil::date(2026, 1, 1))
                .period(bc_models::Period::Weekly)
                .rollover(bc_models::RolloverPolicy::CarryForward)
                .intent(bc_models::BudgetIntent::Limit)
                .target(Amount::new(dec!(100), "AUD"))
                .target_expr("(400.00 / 4)")
                .created_at(Timestamp::now())
                .build(),
        );

        let node = bc_ipc::BudgetTreeNode::from(&row);

        assert_eq!(node.native_period_label, "weekly");
        assert_eq!(node.rollover, Some(bc_ipc::RolloverPolicy::CarryForward));
        assert_eq!(node.target_expr.as_deref(), Some("(400.00 / 4)"));
    }

    #[rstest::rstest]
    #[case(bc_models::AccountType::Expense, bc_ipc::BudgetIntent::Limit)]
    #[case(bc_models::AccountType::Income, bc_ipc::BudgetIntent::Goal)]
    #[case(bc_models::AccountType::Asset, bc_ipc::BudgetIntent::Goal)]
    fn budget_tree_node_default_intent_follows_account_type(
        #[case] account_type: bc_models::AccountType,
        #[case] expected: bc_ipc::BudgetIntent,
    ) {
        let mut row = item("acct:x", crate::RowKind::Account, Balances::new(), vec![]);
        row.account = bc_models::Account::builder()
            .name("Widgets")
            .account_type(account_type)
            .build();

        assert_eq!(bc_ipc::BudgetTreeNode::from(&row).default_intent, expected);
    }

    #[test]
    fn budget_tree_node_carries_unvalued_amounts() {
        let mut unvalued = Balances::new();
        unvalued += &Amount::new(dec!(5), "USD");
        let node =
            bc_ipc::BudgetTreeNode::from(&item("b", crate::RowKind::Budget, unvalued, vec![]));

        assert_eq!(node.unvalued, vec![bc_ipc::Amount::new(dec!(5), "USD")]);
    }

    #[rstest::rstest]
    #[case(crate::RowKind::Budget, bc_ipc::RowKind::Budget)]
    #[case(crate::RowKind::Account, bc_ipc::RowKind::Account)]
    #[case(crate::RowKind::Unallocated, bc_ipc::RowKind::Unallocated)]
    #[case(crate::RowKind::Unbudgeted, bc_ipc::RowKind::Unbudgeted)]
    fn row_kind_converts(#[case] core: crate::RowKind, #[case] expected: bc_ipc::RowKind) {
        assert_eq!(bc_ipc::RowKind::from(core), expected);
    }

    #[rstest]
    #[case(vec![Some(dec!(50)), Some(dec!(-50))], true)]
    #[case(vec![Some(dec!(50))], false)]
    #[case(vec![Some(dec!(50)), None], true)]
    fn transaction_ext_carries_the_models_balance_verdict(
        #[case] amounts: Vec<Option<rust_decimal::Decimal>>,
        #[case] want: bool,
    ) {
        let leg = |value: Option<rust_decimal::Decimal>| {
            bc_models::Posting::builder()
                .id(bc_models::PostingId::new())
                .account_id(bc_models::AccountId::new())
                .maybe_amount(value.map(|v| Amount::new(v, "AUD")))
                .build()
        };
        let postings = amounts.into_iter().map(leg).collect();
        let tx = bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(jiff::civil::date(2026, 1, 1))
            .description("Test")
            .reconciliation(bc_models::Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .postings(postings)
            .build();
        let dto = <bc_ipc::Transaction as TransactionExt>::from_model_with_accounts(
            &tx,
            &HashMap::new(),
            &bc_models::TagForest::default(),
        );
        assert_eq!(dto.balanced, want);
        assert_eq!(dto.balanced, tx.balanced());
    }

    /// A native period status with `actuals` of 12.50, in `commodity`.
    fn native_status(
        commodity: Option<&str>,
        effective_target: Option<rust_decimal::Decimal>,
    ) -> NativePeriodStatus {
        let start = jiff::civil::date(2026, 3, 2);
        let end = jiff::civil::date(2026, 3, 9);
        NativePeriodStatus {
            overlap: PeriodOverlap {
                native_start: start,
                native_end: end,
                overlap_start: start,
                overlap_end: end,
            },
            effective_target,
            actuals: dec!(12.50),
            commodity: commodity.map(bc_models::CommodityCode::new),
            unvalued: Balances::new(),
            verdict: None,
            ratio: None,
        }
    }

    #[test]
    fn native_row_states_amounts_in_the_status_commodity() {
        let row = bc_ipc::NativePeriodRow::from_native(
            &native_status(Some("AUD"), Some(dec!(46.67))),
            "w10",
        );
        assert_eq!(
            row.effective_target,
            Some(bc_ipc::Amount::new(dec!(46.67), "AUD"))
        );
        assert_eq!(row.spent, Some(bc_ipc::Amount::new(dec!(12.50), "AUD")));
    }

    #[test]
    fn native_row_without_a_commodity_has_no_spend() {
        let row = bc_ipc::NativePeriodRow::from_native(&native_status(None, None), "w10");
        assert_eq!(row.effective_target, None);
        assert_eq!(row.spent, None);
    }

    #[test]
    fn a_db_catalog_converts_with_its_mismatch_counts() {
        let catalog = crate::search::DbCatalog::from_parts(
            vec![PathEntry::new("a1", ["Assets", "Bank"])],
            vec![PathEntry::new("t1", ["trip", "flights"])],
            vec![Commodity::new("AUD", Some("A$"), &["AU$"])],
            vec![MetaKey::new("km", QueryType::Number, 2)],
        )
        .with_archived(vec!["a1".to_owned()]);
        assert_eq!(
            bc_ipc::QueryCatalog::from(&catalog),
            bc_ipc::QueryCatalog::new(
                vec![bc_ipc::CatalogPath::new(
                    "a1",
                    vec!["Assets".to_owned(), "Bank".to_owned()]
                )],
                vec![bc_ipc::CatalogPath::new(
                    "t1",
                    vec!["trip".to_owned(), "flights".to_owned()]
                )],
                vec![bc_ipc::CatalogCommodity::new(
                    "AUD",
                    Some("A$".to_owned()),
                    vec!["AU$".to_owned()]
                )],
                vec![bc_ipc::CatalogKey::new(
                    "km",
                    bc_ipc::MetaTypeDto::Number,
                    2
                )],
                vec!["a1".to_owned()],
            )
        );
    }

    #[rstest]
    #[case(QueryType::Text)]
    #[case(QueryType::Number)]
    #[case(QueryType::Boolean)]
    #[case(QueryType::Date)]
    #[case(QueryType::Timestamp)]
    #[case(QueryType::Amount)]
    #[case(QueryType::Account)]
    fn a_key_type_survives_the_trip_to_the_palette(#[case] ty: QueryType) {
        let catalog = crate::search::DbCatalog::from_parts(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![MetaKey::new("k", ty, 0)],
        );
        let snapshot = Snapshot::from(bc_ipc::QueryCatalog::from(&catalog));
        assert_eq!(
            snapshot
                .meta_keys()
                .iter()
                .map(|k| k.ty)
                .collect::<Vec<_>>(),
            vec![ty]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_query_catalog_keeps_archived_accounts(pool: sqlx::SqlitePool) {
        let accounts = crate::account::Service::new(pool.clone());
        let old = accounts
            .create()
            .name("Old Savings")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        let live = accounts
            .create()
            .name("Everyday")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("account");
        accounts
            .archive(&old, crate::Cascade::Reject)
            .await
            .expect("archive");

        let catalog = crate::transaction::Service::new(pool)
            .query_catalog()
            .await
            .expect("catalog");

        let mut paths: Vec<(String, Vec<String>)> = catalog
            .accounts
            .into_iter()
            .map(|a| (a.id, a.path))
            .collect();
        paths.sort();
        let mut expected = vec![
            (old.to_string(), vec!["Old Savings".to_owned()]),
            (live.to_string(), vec!["Everyday".to_owned()]),
        ];
        expected.sort();
        assert_eq!(paths, expected);
        assert_eq!(catalog.archived, vec![old.to_string()]);
    }
}
