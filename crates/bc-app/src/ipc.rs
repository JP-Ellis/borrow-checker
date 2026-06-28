//! Type conversions between `bc_models` and `bc_ipc` at the Tauri IPC boundary.
//!
//! Neither [`IntoIpc`] nor [`IntoModel`] can use the standard [`From`] trait
//! because both sides of each conversion are defined in external crates (the
//! Rust orphan rule). The extension-trait pattern is the idiomatic alternative.

// MARK: Traits

/// Converts a `bc_models` type into its `bc_ipc` counterpart.
pub(crate) trait IntoIpc {
    /// The IPC counterpart type.
    type Output;
    /// Convert `self` into its IPC representation.
    fn into_ipc(self) -> Self::Output;
}

/// Converts a `bc_ipc` type back into its `bc_models` counterpart.
pub(crate) trait IntoModel {
    /// The domain model counterpart type.
    type Output;
    /// Convert `self` into its domain model representation.
    fn into_model(self) -> Self::Output;
}

// MARK: Amount

impl IntoIpc for &bc_models::Amount {
    type Output = bc_ipc::Amount;

    /// Converts a [`bc_models::Amount`] to an IPC [`bc_ipc::Amount`].
    ///
    /// Carries the decimal value across the boundary verbatim — no lossy
    /// minor-unit conversion.
    #[inline]
    fn into_ipc(self) -> bc_ipc::Amount {
        bc_ipc::Amount::new(self.value(), self.commodity().as_str())
    }
}

impl IntoModel for &bc_ipc::Amount {
    type Output = bc_models::Amount;

    /// Converts an IPC [`bc_ipc::Amount`] to a [`bc_models::Amount`].
    #[inline]
    fn into_model(self) -> bc_models::Amount {
        bc_models::Amount::new(
            self.value,
            bc_models::CommodityCode::new(self.currency_code.clone()),
        )
    }
}

// MARK: AccountType

impl IntoIpc for bc_models::AccountType {
    type Output = bc_ipc::AccountType;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::AccountType and bc_ipc::AccountType are #[non_exhaustive]; \
                  the wildcard fallback to Asset is intentional for future unknown variants"
    )]
    fn into_ipc(self) -> bc_ipc::AccountType {
        match self {
            bc_models::AccountType::Asset => bc_ipc::AccountType::Asset,
            bc_models::AccountType::Liability => bc_ipc::AccountType::Liability,
            bc_models::AccountType::Equity => bc_ipc::AccountType::Equity,
            bc_models::AccountType::Income => bc_ipc::AccountType::Income,
            bc_models::AccountType::Expense => bc_ipc::AccountType::Expense,
            _ => bc_ipc::AccountType::Asset,
        }
    }
}

// MARK: Reconciliation

impl IntoIpc for bc_models::Reconciliation {
    type Output = bc_ipc::Reconciliation;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "bc_models::Reconciliation is #[non_exhaustive]; the wildcard fallback is intentional"
    )]
    fn into_ipc(self) -> bc_ipc::Reconciliation {
        match self {
            bc_models::Reconciliation::Unreconciled => bc_ipc::Reconciliation::Unreconciled,
            bc_models::Reconciliation::Flagged => bc_ipc::Reconciliation::Flagged,
            bc_models::Reconciliation::Reconciled => bc_ipc::Reconciliation::Reconciled,
            _ => bc_ipc::Reconciliation::Unreconciled,
        }
    }
}

impl IntoModel for bc_ipc::Reconciliation {
    type Output = bc_models::Reconciliation;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "bc_ipc::Reconciliation is #[non_exhaustive]; the wildcard fallback is intentional"
    )]
    fn into_model(self) -> bc_models::Reconciliation {
        match self {
            bc_ipc::Reconciliation::Unreconciled => bc_models::Reconciliation::Unreconciled,
            bc_ipc::Reconciliation::Flagged => bc_models::Reconciliation::Flagged,
            bc_ipc::Reconciliation::Reconciled => bc_models::Reconciliation::Reconciled,
            _ => bc_models::Reconciliation::Unreconciled,
        }
    }
}

// MARK: Account

impl IntoIpc for &bc_models::Account {
    type Output = bc_ipc::AccountNode;

    #[inline]
    fn into_ipc(self) -> bc_ipc::AccountNode {
        bc_ipc::AccountNode::new(
            self.id().to_string(),
            self.name(),
            None::<&str>,
            None,
            self.parent_id().map(ToString::to_string),
            self.account_type().into_ipc(),
            vec![],
        )
    }
}

/// Converts a [`bc_models::Account`] to [`bc_ipc::AccountNode`] with a pre-computed balance.
///
/// Used by `list_accounts` which fetches balances in a separate batch query.
///
/// # Arguments
///
/// * `account` - The account to convert.
/// * `balance` - The pre-computed balance for this account.
/// * `forest` - The loaded tag hierarchy used to resolve account tag IDs to paths.
#[inline]
pub(crate) fn into_ipc_with_balance(
    account: &bc_models::Account,
    balance: Option<bc_ipc::Amount>,
    forest: &bc_models::TagForest,
) -> bc_ipc::AccountNode {
    bc_ipc::AccountNode::new(
        account.id().to_string(),
        account.name(),
        None::<&str>,
        balance,
        account.parent_id().map(ToString::to_string),
        account.account_type().into_ipc(),
        resolve_tag_paths(forest, account.tag_ids()),
    )
}

// MARK: Budget revision view

/// Computes the overlap of a revision's reign `[reign_start, reign_end)` with the
/// display window `[win_start, win_end)`.
///
/// A `reign_end` of `None` represents the latest revision (open-ended reign).
///
/// # Arguments
///
/// * `reign_start` - Inclusive reign start (`effective_from`).
/// * `reign_end` - Exclusive reign end (next revision's `effective_from`), or `None`.
/// * `win_start` - Inclusive window start.
/// * `win_end` - Exclusive window end.
///
/// # Returns
///
/// `Some(WindowOverlap)` when the reign intersects the window, else `None`.
#[must_use]
pub(crate) fn window_overlap(
    reign_start: jiff::civil::Date,
    reign_end: Option<jiff::civil::Date>,
    win_start: jiff::civil::Date,
    win_end: jiff::civil::Date,
) -> Option<bc_ipc::WindowOverlap> {
    let start = reign_start.max(win_start);
    let end = reign_end.map_or(win_end, |re| re.min(win_end));
    if start >= end {
        return None;
    }
    let covers_full_window = start == win_start && end == win_end;
    Some(bc_ipc::WindowOverlap::new(start, end, covers_full_window))
}

// MARK: Period

impl IntoIpc for &bc_models::Period {
    type Output = bc_ipc::Period;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Period {
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "bc_models::Period is #[non_exhaustive]; unknown future variants fall back to Monthly"
        )]
        match self {
            bc_models::Period::Weekly => bc_ipc::Period::Weekly,
            bc_models::Period::Fortnightly { .. } => bc_ipc::Period::Fortnightly,
            bc_models::Period::Monthly => bc_ipc::Period::Monthly,
            bc_models::Period::Quarterly => bc_ipc::Period::Quarterly,
            bc_models::Period::CalendarYear => bc_ipc::Period::CalendarYear,
            bc_models::Period::FinancialYear {
                start_month,
                start_day,
            } => bc_ipc::Period::FinancialYear {
                start_month: *start_month,
                start_day: *start_day,
            },
            bc_models::Period::FinancialQuarter {
                start_month,
                start_day,
            } => bc_ipc::Period::FinancialQuarter {
                start_month: *start_month,
                start_day: *start_day,
            },
            bc_models::Period::Custom {
                days: Some(1),
                weeks: None,
                months: None,
            } => bc_ipc::Period::Daily,
            other => {
                tracing::warn!(
                    ?other,
                    "Period has no bc_ipc equivalent; defaulting to monthly"
                );
                bc_ipc::Period::Monthly
            }
        }
    }
}

impl IntoModel for bc_ipc::Period {
    type Output = bc_models::Period;

    #[inline]
    fn into_model(self) -> bc_models::Period {
        match self {
            bc_ipc::Period::Daily => bc_models::Period::Custom {
                days: Some(1),
                weeks: None,
                months: None,
            },
            bc_ipc::Period::Weekly => bc_models::Period::Weekly,
            bc_ipc::Period::Fortnightly => {
                // TODO: use the globally-configured fortnightly anchor (Milestone 5 config).
                // 2026-01-05 (Monday) is a placeholder; any user whose pay cycle does not
                // align to this anchor will see misaligned fortnightly buckets.
                tracing::warn!(
                    anchor = "2026-01-05",
                    "fortnightly anchor is hardcoded; user pay cycles may not align"
                );
                #[expect(
                    clippy::expect_used,
                    reason = "2026-01-05 is a valid date; this can never panic"
                )]
                let anchor =
                    jiff::civil::Date::new(2026, 1, 5).expect("2026-01-05 is a valid date");
                bc_models::Period::Fortnightly { anchor }
            }
            bc_ipc::Period::Monthly => bc_models::Period::Monthly,
            bc_ipc::Period::Quarterly => bc_models::Period::Quarterly,
            bc_ipc::Period::CalendarYear => bc_models::Period::CalendarYear,
            bc_ipc::Period::FinancialYear {
                start_month,
                start_day,
            } => bc_models::Period::FinancialYear {
                start_month,
                start_day,
            },
            bc_ipc::Period::FinancialQuarter {
                start_month,
                start_day,
            } => bc_models::Period::FinancialQuarter {
                start_month,
                start_day,
            },
            unknown => {
                tracing::warn!(?unknown, "unknown Period; falling back to Monthly");
                bc_models::Period::Monthly
            }
        }
    }
}

// MARK: RolloverPolicy

impl IntoIpc for bc_models::RolloverPolicy {
    type Output = bc_ipc::RolloverPolicy;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_models::RolloverPolicy and bc_ipc::RolloverPolicy are #[non_exhaustive]; \
                  the wildcard fallback to ResetToZero is intentional for future unknown variants"
    )]
    fn into_ipc(self) -> bc_ipc::RolloverPolicy {
        match self {
            bc_models::RolloverPolicy::CarryForward => bc_ipc::RolloverPolicy::CarryForward,
            bc_models::RolloverPolicy::ResetToZero => bc_ipc::RolloverPolicy::ResetToZero,
            bc_models::RolloverPolicy::CapAtTarget => bc_ipc::RolloverPolicy::CapAtTarget,
            _ => bc_ipc::RolloverPolicy::ResetToZero,
        }
    }
}

impl IntoModel for bc_ipc::RolloverPolicy {
    type Output = bc_models::RolloverPolicy;

    #[inline]
    #[expect(
        clippy::match_same_arms,
        reason = "both bc_ipc::RolloverPolicy and bc_models::RolloverPolicy are #[non_exhaustive]; \
                  the wildcard fallback to ResetToZero is intentional for future unknown variants"
    )]
    fn into_model(self) -> bc_models::RolloverPolicy {
        match self {
            bc_ipc::RolloverPolicy::CarryForward => bc_models::RolloverPolicy::CarryForward,
            bc_ipc::RolloverPolicy::ResetToZero => bc_models::RolloverPolicy::ResetToZero,
            bc_ipc::RolloverPolicy::CapAtTarget => bc_models::RolloverPolicy::CapAtTarget,
            _ => bc_models::RolloverPolicy::ResetToZero,
        }
    }
}

// MARK: Posting

impl IntoIpc for &bc_models::Posting {
    type Output = bc_ipc::Posting;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Posting {
        let account_id = self.account_id().to_string();
        let amount = self.amount().map(IntoIpc::into_ipc);
        bc_ipc::Posting::new(
            self.id().to_string(),
            bc_ipc::AccountRef::new(account_id.clone(), account_id),
            amount,
            self.note(),
            vec![],
            self.spread_from(),
            self.spread_until(),
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
pub(crate) fn build_account_path(
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
pub(crate) fn resolve_tag_paths(
    forest: &bc_models::TagForest,
    ids: &[bc_models::TagId],
) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    ids.iter()
        .filter_map(|id| forest.path_of(id).map(|p| p.to_string()))
        .filter(|path| seen.insert(path.clone()))
        .collect()
}

/// Converts a [`bc_models::Transaction`] to [`bc_ipc::Transaction`], resolving
/// posting account names from `account_map` and tag IDs to paths via `forest`.
///
/// The effective tags for each posting are the union of the transaction's own
/// tags and the posting's own tags, deduplicated by resolved path.
///
/// # Arguments
///
/// * `tx` - The transaction to convert.
/// * `account_map` - Map from account ID string to account reference.
/// * `forest` - The loaded tag hierarchy used to resolve tag IDs to paths.
pub(crate) fn transaction_into_ipc_with_accounts(
    tx: &bc_models::Transaction,
    account_map: &std::collections::HashMap<String, &bc_models::Account>,
    forest: &bc_models::TagForest,
) -> bc_ipc::Transaction {
    let tx_tag_ids = tx.tag_ids();
    let postings = tx
        .postings()
        .iter()
        .map(|p| {
            let account_id = p.account_id().to_string();
            let account_name = build_account_path(&account_id, account_map);
            let amount = p.amount().map(IntoIpc::into_ipc);
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

    bc_ipc::Transaction::new(
        tx.id().to_string(),
        tx.date(),
        tx.payee().unwrap_or_default(),
        tx.description(),
        tx.note(),
        extra_dates,
        tx.reconciliation().into_ipc(),
        resolve_tag_paths(forest, tx_tag_ids),
        postings,
        vec![],
    )
}

// MARK: Budget tree

/// Converts a [`bc_core::BudgetTreeItem`] into a [`bc_ipc::BudgetTreeNode`].
pub(crate) fn budget_tree_item_into_ipc(item: &bc_core::BudgetTreeItem) -> bc_ipc::BudgetTreeNode {
    budget_tree_node_recursive(item)
}

/// Recursive implementation of [`budget_tree_item_into_ipc`].
fn budget_tree_node_recursive(item: &bc_core::BudgetTreeItem) -> bc_ipc::BudgetTreeNode {
    let spent = item.actuals.first().map_or_else(
        || {
            let c = item
                .commodity
                .as_ref()
                .map_or("", bc_models::CommodityCode::as_str);
            bc_ipc::Amount::new(rust_decimal::Decimal::ZERO, c)
        },
        IntoIpc::into_ipc,
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
            .into_ipc(),
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

// MARK: Transaction

impl IntoIpc for &bc_models::Transaction {
    type Output = bc_ipc::Transaction;

    #[inline]
    fn into_ipc(self) -> bc_ipc::Transaction {
        let postings: Vec<bc_ipc::Posting> =
            self.postings().iter().map(IntoIpc::into_ipc).collect();
        let extra_dates = self
            .extra_dates()
            .iter()
            .map(|(label, date)| (label.clone(), *date))
            .collect();
        bc_ipc::Transaction::new(
            self.id().to_string(),
            self.date(),
            self.payee().unwrap_or_default(),
            self.description(),
            self.note(),
            extra_dates,
            self.reconciliation().into_ipc(),
            vec![], // tag path resolution deferred: posting tag_ids carry raw ids (option a)
            postings,
            vec![],
        )
    }
}

// MARK: Error mapping

/// Maps a [`bc_core::BcError`] to its IPC [`bc_ipc::BcError`] counterpart.
///
/// User-facing validation failures (`InvalidInput`, `BadData`, and the
/// account/tag rule violations) surface as [`bc_ipc::BcError::Validation`] so the
/// UI can render a friendly message; `NotFound` maps to
/// [`bc_ipc::BcError::NotFound`]; everything genuinely internal (database, IO,
/// serialisation) becomes [`bc_ipc::BcError::Internal`].
///
/// A free function rather than a [`From`] impl because both error types are
/// defined in external crates (the orphan rule forbids the impl here).
///
/// # Arguments
///
/// * `err` - The core error to translate.
///
/// # Returns
///
/// The corresponding serialisable IPC error.
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "bc_core::BcError is #[non_exhaustive]; catch-all required for future variants"
)]
pub(crate) fn core_error_to_ipc(err: &bc_core::BcError) -> bc_ipc::BcError {
    use bc_core::BcError as Core;

    match err {
        Core::NotFound(_) => bc_ipc::BcError::NotFound(err.to_string()),
        Core::InvalidInput(_)
        | Core::BadData(_)
        | Core::AlreadyArchived(_)
        | Core::InvalidAccountKind { .. }
        | Core::TagInUse(_) => bc_ipc::BcError::Validation(err.to_string()),
        _ => bc_ipc::BcError::Internal(err.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::IntoIpc as _;
    use super::IntoModel as _;
    use super::build_account_path;
    use super::core_error_to_ipc;
    use super::resolve_tag_paths;
    use super::window_overlap;

    #[test]
    fn core_bad_data_maps_to_validation() {
        let err =
            bc_core::BcError::BadData("cannot reconcile an unbalanced transaction".to_owned());
        let mapped = core_error_to_ipc(&err);
        assert!(
            matches!(mapped, bc_ipc::BcError::Validation(_)),
            "BadData must surface as Validation, got {mapped:?}"
        );
    }

    #[test]
    fn core_invalid_input_maps_to_validation() {
        let err = bc_core::BcError::InvalidInput("two or more elided postings".to_owned());
        assert!(matches!(
            core_error_to_ipc(&err),
            bc_ipc::BcError::Validation(_)
        ));
    }

    #[test]
    fn core_not_found_maps_to_not_found() {
        let err = bc_core::BcError::NotFound("txn-001".to_owned());
        assert!(matches!(
            core_error_to_ipc(&err),
            bc_ipc::BcError::NotFound(_)
        ));
    }

    #[test]
    fn resolve_tag_paths_renders_hierarchy_and_dedupes() {
        use jiff::Timestamp;
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
        let paths = resolve_tag_paths(&forest, &[josh.clone(), josh.clone(), person.clone()]);
        assert_eq!(paths, vec!["person:josh".to_owned(), "person".to_owned()]);
    }

    #[test]
    fn amount_into_ipc_aud() {
        let model = bc_models::Amount::new(
            rust_decimal::Decimal::new(1050, 2), // 10.50
            bc_models::CommodityCode::new("AUD"),
        );
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.value, rust_decimal::Decimal::new(1050, 2));
        assert_eq!(ipc.currency_code, "AUD");
    }

    #[test]
    fn amount_into_model_aud() {
        let ipc = bc_ipc::Amount::new(Decimal::new(1050, 2), "AUD");
        let model = (&ipc).into_model();
        assert_eq!(model.value(), rust_decimal::Decimal::new(1050, 2));
        assert_eq!(model.commodity().as_str(), "AUD");
    }

    #[test]
    fn amount_round_trip_jpy() {
        let model = bc_models::Amount::new(
            rust_decimal::Decimal::new(1234, 0),
            bc_models::CommodityCode::new("JPY"),
        );
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.value, rust_decimal::Decimal::new(1234, 0));
        let back = (&ipc).into_model();
        assert_eq!(back, model);
    }

    #[test]
    fn amount_round_trip_btc() {
        let model = bc_models::Amount::new(
            rust_decimal::Decimal::new(12345, 8), // 0.00012345 BTC
            bc_models::CommodityCode::new("BTC"),
        );
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.value, rust_decimal::Decimal::new(12345, 8));
        let back = (&ipc).into_model();
        assert_eq!(back, model);
    }

    #[test]
    fn amount_round_trip_large_btc_no_overflow() {
        let big = rust_decimal::Decimal::from_i128_with_scale(100_000_000_000_000_000_000_i128, 8);
        let model = bc_models::Amount::new(big, bc_models::CommodityCode::new("BTC"));
        let ipc = (&model).into_ipc();
        assert_eq!(ipc.value, big);
        let back = (&ipc).into_model();
        assert_eq!(back, model);
    }

    #[test]
    fn build_account_path_returns_name_for_root_account() {
        let account = bc_models::Account::builder()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .build();

        let account_id = account.id().to_string();
        let map = HashMap::from([(account_id.clone(), &account)]);

        assert_eq!(build_account_path(&account_id, &map), "Checking");
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
            build_account_path(&child.id().to_string(), &map),
            "Assets :: Checking"
        );
    }

    #[test]
    fn build_account_path_falls_back_to_id_when_not_found() {
        let map: HashMap<String, &bc_models::Account> = HashMap::new();
        let fake_id = "account_00000000000000000000000000";
        assert_eq!(build_account_path(fake_id, &map), fake_id);
    }

    #[test]
    fn window_overlap_full_cover_when_reign_spans_window() {
        use jiff::civil::date;
        let o = window_overlap(
            date(2025, 1, 1),
            Some(date(2028, 1, 1)),
            date(2026, 7, 1),
            date(2026, 7, 8),
        );
        assert_eq!(
            o,
            Some(bc_ipc::WindowOverlap::new(
                date(2026, 7, 1),
                date(2026, 7, 8),
                true
            ))
        );
    }

    #[test]
    fn window_overlap_partial_from_left() {
        use jiff::civil::date;
        // reign ends inside the window -> partial, range [win_start, reign_end).
        let o = window_overlap(
            date(2025, 1, 1),
            Some(date(2026, 9, 1)),
            date(2026, 7, 1),
            date(2027, 7, 1),
        );
        assert_eq!(
            o,
            Some(bc_ipc::WindowOverlap::new(
                date(2026, 7, 1),
                date(2026, 9, 1),
                false
            ))
        );
    }

    #[test]
    fn window_overlap_open_reign_extends_to_window_end() {
        use jiff::civil::date;
        // reign_end None (latest revision) starting inside the window -> partial to win_end.
        let o = window_overlap(date(2026, 10, 1), None, date(2026, 7, 1), date(2027, 7, 1));
        assert_eq!(
            o,
            Some(bc_ipc::WindowOverlap::new(
                date(2026, 10, 1),
                date(2027, 7, 1),
                false
            ))
        );
    }

    #[test]
    fn window_overlap_none_when_disjoint() {
        use jiff::civil::date;
        let o = window_overlap(date(2030, 1, 1), None, date(2026, 7, 1), date(2027, 7, 1));
        assert_eq!(o, None);
    }

    #[test]
    fn model_period_into_ipc_maps_known_variants() {
        use crate::ipc::IntoIpc as _;
        assert_eq!(
            (&bc_models::Period::Weekly).into_ipc(),
            bc_ipc::Period::Weekly
        );
        assert_eq!(
            (&bc_models::Period::Monthly).into_ipc(),
            bc_ipc::Period::Monthly
        );
        assert_eq!(
            (&bc_models::Period::Custom {
                days: Some(1),
                weeks: None,
                months: None
            })
                .into_ipc(),
            bc_ipc::Period::Daily
        );
    }
}
