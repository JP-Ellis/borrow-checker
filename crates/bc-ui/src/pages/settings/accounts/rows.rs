// Pure row logic for the Settings accounts table: ordering, depth, display
// text, and the edit draft with its patch.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim, which is why this header uses `//` rather than `//!`.

use std::collections::HashMap;
use std::collections::HashSet;

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AccountPatchDto;
use bc_ipc::AcquisitionDto;
use bc_ipc::TagInfo;
use jiff::civil::Date;

use crate::pages::accounts::tree::type_rank;

/// Orders accounts as the sidebar does, paired with their depth.
///
/// Roots come by type (asset, liability, equity, income, expense) then name;
/// each is followed depth-first by its children by name. An account whose
/// parent is absent from `details` is treated as a root. Each account appears
/// at most once, so a corrupt parent cycle cannot loop forever; an account
/// reachable from no root is left out.
///
/// # Arguments
///
/// * `details` - The flat account list, in any order.
///
/// # Returns
///
/// `(depth, account)` pairs, roots at depth `0`.
#[must_use]
pub(crate) fn tree_order(details: &[AccountDetail]) -> Vec<(usize, &AccountDetail)> {
    let known: HashSet<&str> = details.iter().map(|d| d.id.as_str()).collect();
    let mut children: HashMap<&str, Vec<&AccountDetail>> = HashMap::new();
    let mut roots: Vec<&AccountDetail> = Vec::new();
    // Visiting by name makes every child list name-ordered as it is built.
    let mut by_name: Vec<&AccountDetail> = details.iter().collect();
    by_name.sort_by(|a, b| a.name.cmp(&b.name));
    for d in by_name {
        match d.parent_id.as_deref() {
            Some(p) if known.contains(p) => children.entry(p).or_default().push(d),
            _ => roots.push(d),
        }
    }
    roots.sort_by(|a, b| {
        type_rank(a.account_type)
            .cmp(&type_rank(b.account_type))
            .then_with(|| a.name.cmp(&b.name))
    });

    let mut out = Vec::with_capacity(details.len());
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<(usize, &AccountDetail)> = roots.into_iter().rev().map(|d| (0, d)).collect();
    while let Some((depth, d)) = stack.pop() {
        if !seen.insert(d.id.as_str()) {
            continue;
        }
        if let Some(kids) = children.get(d.id.as_str()) {
            stack.extend(kids.iter().rev().map(|k| (depth.saturating_add(1), *k)));
        }
        out.push((depth, d));
    }
    out
}

/// The short human label for an account kind.
#[must_use]
pub(crate) fn kind_label(kind: AccountKindDto) -> &'static str {
    match kind {
        AccountKindDto::DepositAccount => "deposit",
        AccountKindDto::ManualAsset => "manual asset",
        AccountKindDto::Receivable => "receivable",
        AccountKindDto::VirtualAllocation => "virtual",
        AccountKindDto::Group => "group",
        // Covers only future #[non_exhaustive] variants.
        _ => "other",
    }
}

/// A date as ISO `YYYY-MM-DD`, or empty when absent.
#[must_use]
pub(crate) fn date_text(date: Option<Date>) -> String {
    date.map_or_default(|d| d.to_string())
}

/// The lifecycle state shown for an account: `archived` wins over `closed`.
#[must_use]
pub(crate) fn state_label(details: &AccountDetail) -> &'static str {
    if details.archived {
        "archived"
    } else if details.closed_on.is_some() {
        "closed"
    } else {
        "active"
    }
}

// MARK: Editing

/// The editable fields of one account row, as the user has left them.
///
/// Built from the row's base with [`From`]; [`diff`] turns it back into the
/// patch that a save sends.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RowDraft {
    /// Display name.
    pub name: String,
    /// Parent account ID, or `None` for top level.
    pub parent_id: Option<String>,
    /// Description text; empty means none.
    pub description: String,
    /// Attached tags, in any order.
    pub tags: Vec<TagInfo>,
    /// Allowed commodity codes, in order; the first is the display default.
    pub commodity_codes: Vec<String>,
    /// Opening date.
    pub opened_on: Option<Date>,
    /// Closing date; edited only on a closed account.
    pub closed_on: Option<Date>,
    /// The whole acquisition record of a manual asset.
    pub acquisition: Option<AcquisitionDto>,
}

impl From<&AccountDetail> for RowDraft {
    #[inline]
    fn from(details: &AccountDetail) -> Self {
        Self {
            name: details.name.clone(),
            parent_id: details.parent_id.clone(),
            description: details.description.clone().unwrap_or_default(),
            tags: details.tags.clone(),
            commodity_codes: details.commodity_codes.clone(),
            opened_on: details.opened_on,
            closed_on: details.closed_on,
            acquisition: details.acquisition.clone(),
        }
    }
}

/// The patch that turns `base` into `edited`, carrying only changed fields.
///
/// Tags compare as a set and commodities as an ordered list. A changed
/// acquisition carries the whole record, since the backend replaces it. A
/// cleared closing date is no change: reopening is its own command.
///
/// # Arguments
///
/// * `base` - The account as last loaded or saved.
/// * `edited` - The row's draft.
///
/// # Returns
///
/// `None` when nothing changed.
#[must_use]
pub(crate) fn diff(base: &AccountDetail, edited: &RowDraft) -> Option<AccountPatchDto> {
    let mut patch = AccountPatchDto::default();
    if edited.name != base.name {
        patch.name = Some(edited.name.clone());
    }
    if edited.parent_id != base.parent_id {
        patch.parent_id = Some(edited.parent_id.clone());
    }
    if edited.description != base.description.as_deref().unwrap_or_default() {
        patch.description =
            Some((!edited.description.is_empty()).then(|| edited.description.clone()));
    }
    let tag_ids = |tags: &[TagInfo]| -> Vec<String> {
        let mut ids: Vec<String> = tags.iter().map(|t| t.id.clone()).collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let edited_tags = tag_ids(&edited.tags);
    if edited_tags != tag_ids(&base.tags) {
        patch.tag_ids = Some(edited_tags);
    }
    if edited.commodity_codes != base.commodity_codes {
        patch.commodity_codes = Some(edited.commodity_codes.clone());
    }
    if edited.opened_on != base.opened_on {
        patch.opened_on = Some(edited.opened_on);
    }
    if edited.closed_on != base.closed_on {
        patch.closed_on = edited.closed_on;
    }
    // An emptied record counts as no record, so typing into a blank
    // acquisition and clearing it again leaves the row clean.
    let recorded = |record: &Option<AcquisitionDto>| {
        record.clone().filter(|a| *a != AcquisitionDto::default())
    };
    let edited_acquisition = recorded(&edited.acquisition);
    if edited_acquisition != recorded(&base.acquisition) {
        patch.acquisition = Some(edited_acquisition.unwrap_or_default());
    }
    (patch != AccountPatchDto::default()).then_some(patch)
}

/// The accounts `id` may move under, in tree order.
///
/// A parent has the same type, is neither `id` nor one of its descendants,
/// and is not closed when the account is open, nor archived when it is
/// active. Top level is always valid and is not listed.
///
/// # Arguments
///
/// * `details` - Every saved account.
/// * `id` - The account being moved.
/// * `open` - Whether the account is open.
/// * `active` - Whether the account is not archived.
///
/// # Returns
///
/// The candidate parents; empty when `id` is not in `details`.
#[must_use]
pub(crate) fn valid_parents<'a>(
    details: &'a [AccountDetail],
    id: &str,
    open: bool,
    active: bool,
) -> Vec<&'a AccountDetail> {
    let Some(account) = details.iter().find(|d| d.id == id) else {
        return Vec::new();
    };
    let mut subtree: HashSet<&str> = HashSet::from([id]);
    let mut frontier = vec![id];
    while let Some(parent) = frontier.pop() {
        for child in details
            .iter()
            .filter(|d| d.parent_id.as_deref() == Some(parent))
        {
            if subtree.insert(child.id.as_str()) {
                frontier.push(child.id.as_str());
            }
        }
    }
    tree_order(details)
        .into_iter()
        .map(|(_, d)| d)
        .filter(|d| d.account_type == account.account_type && !subtree.contains(d.id.as_str()))
        .filter(|d| !open || d.closed_on.is_none())
        .filter(|d| !active || !d.archived)
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountType;
    use bc_ipc::DepreciationPolicyDto;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    fn account(
        id: &str,
        name: &str,
        parent: Option<&str>,
        account_type: AccountType,
    ) -> AccountDetail {
        AccountDetail::builder()
            .id(id)
            .name(name)
            .path(name)
            .maybe_parent_id(parent.map(str::to_owned))
            .account_type(account_type)
            .kind(AccountKindDto::Group)
            .commodity_codes(vec![])
            .tags(vec![])
            .event_count(0)
            .build()
    }

    fn fixture() -> Vec<AccountDetail> {
        vec![
            account("food", "Food", Some("expenses"), AccountType::Expense),
            account("savings", "Savings", Some("bank"), AccountType::Asset),
            account("expenses", "Expenses", None, AccountType::Expense),
            account("cards", "Cards", None, AccountType::Liability),
            account("bank", "Bank", Some("assets"), AccountType::Asset),
            account("assets", "Assets", None, AccountType::Asset),
            account("cheque", "Cheque", Some("bank"), AccountType::Asset),
        ]
    }

    fn ids_with_depth(details: &[AccountDetail]) -> Vec<(usize, &str)> {
        tree_order(details)
            .into_iter()
            .map(|(depth, d)| (depth, d.id.as_str()))
            .collect()
    }

    #[test]
    fn tree_order_is_depth_first_by_type_then_name() {
        assert_eq!(
            ids_with_depth(&fixture()),
            vec![
                (0, "assets"),
                (1, "bank"),
                (2, "cheque"),
                (2, "savings"),
                (0, "cards"),
                (0, "expenses"),
                (1, "food"),
            ]
        );
    }

    #[test]
    fn archived_rows_keep_their_tree_position() {
        let mut details = fixture();
        for d in &mut details {
            if d.id == "bank" {
                d.archived = true;
            }
        }
        let order = ids_with_depth(&details);
        assert_eq!(order, ids_with_depth(&fixture()));
        assert_eq!(order.get(1), Some(&(1, "bank")));
    }

    #[test]
    fn orphan_is_a_root_and_cycle_members_are_dropped() {
        let details = vec![
            account("lost", "Lost", Some("missing"), AccountType::Asset),
            account("a", "A", Some("b"), AccountType::Asset),
            account("b", "B", Some("a"), AccountType::Asset),
        ];
        assert_eq!(ids_with_depth(&details), vec![(0, "lost")]);
    }

    #[test]
    fn tree_order_of_nothing_is_empty() {
        assert_eq!(tree_order(&[]), vec![]);
    }

    #[rstest]
    #[case(AccountKindDto::DepositAccount, "deposit")]
    #[case(AccountKindDto::ManualAsset, "manual asset")]
    #[case(AccountKindDto::Receivable, "receivable")]
    #[case(AccountKindDto::VirtualAllocation, "virtual")]
    #[case(AccountKindDto::Group, "group")]
    fn kind_labels(#[case] kind: AccountKindDto, #[case] expected: &str) {
        assert_eq!(kind_label(kind), expected);
    }

    #[test]
    fn dates_are_iso_or_empty() {
        assert_eq!(date_text(Some(date(2025, 6, 3))), "2025-06-03");
        assert_eq!(date_text(None), "");
    }

    #[rstest]
    #[case(false, None, "active")]
    #[case(false, Some(date(2025, 6, 30)), "closed")]
    #[case(true, None, "archived")]
    #[case(true, Some(date(2025, 6, 30)), "archived")]
    fn state_labels(
        #[case] archived: bool,
        #[case] closed_on: Option<Date>,
        #[case] expected: &str,
    ) {
        let mut d = account("a", "A", None, AccountType::Asset);
        d.archived = archived;
        d.closed_on = closed_on;
        assert_eq!(state_label(&d), expected);
    }

    // MARK: Editing

    fn manual_asset() -> AccountDetail {
        let mut d = account("car", "Car", Some("assets"), AccountType::Asset);
        d.kind = AccountKindDto::ManualAsset;
        d.acquisition = Some(
            AcquisitionDto::builder()
                .date(date(2024, 3, 1))
                .cost(dec!(12000))
                .depreciation_policy(DepreciationPolicyDto::StraightLine {
                    annual_rate: dec!(0.20),
                })
                .build(),
        );
        d
    }

    #[test]
    fn diff_of_untouched_row_is_none() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.description = Some("Everyday banking".to_owned());
        d.commodity_codes = vec!["AUD".to_owned(), "USD".to_owned()];
        d.tags = vec![TagInfo::new("t1", "joint")];
        d.opened_on = Some(date(2024, 1, 1));
        assert_eq!(diff(&d, &RowDraft::from(&d)), None);
        let car = manual_asset();
        assert_eq!(diff(&car, &RowDraft::from(&car)), None);
    }

    #[test]
    fn renaming_sends_only_the_name() {
        let d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut draft = RowDraft::from(&d);
        draft.name = "Banking".to_owned();
        assert_eq!(
            diff(&d, &draft),
            Some(AccountPatchDto::builder().name("Banking").build())
        );
    }

    #[test]
    fn reordering_commodities_is_a_change() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.commodity_codes = vec!["AUD".to_owned(), "USD".to_owned()];
        let mut draft = RowDraft::from(&d);
        draft.commodity_codes.reverse();
        assert_eq!(
            diff(&d, &draft),
            Some(
                AccountPatchDto::builder()
                    .commodity_codes(vec!["USD".to_owned(), "AUD".to_owned()])
                    .build()
            )
        );
    }

    #[test]
    fn clearing_description_is_some_none() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.description = Some("Everyday banking".to_owned());
        let mut draft = RowDraft::from(&d);
        draft.description = String::new();
        let patch = diff(&d, &draft).expect("a change");
        assert_eq!(patch.description, Some(None));
        assert_eq!(patch, AccountPatchDto::builder().description(None).build());
    }

    #[test]
    fn moving_to_top_level_is_some_none() {
        let d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut draft = RowDraft::from(&d);
        draft.parent_id = None;
        assert_eq!(
            diff(&d, &draft),
            Some(AccountPatchDto::builder().parent_id(None).build())
        );
    }

    #[test]
    fn reordering_tags_is_not_a_change() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.tags = vec![TagInfo::new("t1", "joint"), TagInfo::new("t2", "work")];
        let mut draft = RowDraft::from(&d);
        draft.tags.reverse();
        assert_eq!(diff(&d, &draft), None);
        draft.tags.pop();
        assert_eq!(
            diff(&d, &draft),
            Some(
                AccountPatchDto::builder()
                    .tag_ids(vec!["t2".to_owned()])
                    .build()
            )
        );
    }

    #[test]
    fn acquisition_change_sends_whole_record() {
        let car = manual_asset();
        let mut draft = RowDraft::from(&car);
        if let Some(acq) = draft.acquisition.as_mut() {
            acq.cost = Some(dec!(11500));
        }
        let patch = diff(&car, &draft).expect("a change");
        assert_eq!(
            patch.acquisition,
            Some(
                AcquisitionDto::builder()
                    .date(date(2024, 3, 1))
                    .cost(dec!(11500))
                    .depreciation_policy(DepreciationPolicyDto::StraightLine {
                        annual_rate: dec!(0.20),
                    })
                    .build()
            )
        );
        assert_eq!(patch.name, None);
    }

    #[test]
    fn emptied_acquisition_is_no_change() {
        let mut car = manual_asset();
        car.acquisition = None;
        let mut draft = RowDraft::from(&car);
        draft.acquisition = Some(AcquisitionDto::default());
        assert_eq!(diff(&car, &draft), None);
    }

    fn tree_for_parents() -> Vec<AccountDetail> {
        let mut details = fixture();
        let mut old = account("old", "Old", Some("assets"), AccountType::Asset);
        old.closed_on = Some(date(2025, 6, 30));
        let mut gone = account("gone", "Gone", Some("assets"), AccountType::Asset);
        gone.archived = true;
        details.push(old);
        details.push(gone);
        details
    }

    fn parent_ids<'a>(parents: &[&'a AccountDetail]) -> Vec<&'a str> {
        parents.iter().map(|d| d.id.as_str()).collect()
    }

    #[test]
    fn valid_parents_excludes_own_subtree_and_other_types() {
        let details = tree_for_parents();
        assert_eq!(
            parent_ids(&valid_parents(&details, "bank", false, false)),
            vec!["assets", "gone", "old"]
        );
        assert_eq!(
            parent_ids(&valid_parents(&details, "food", true, true)),
            vec!["expenses"]
        );
    }

    #[test]
    fn valid_parents_excludes_closed_for_open_account() {
        let details = tree_for_parents();
        assert_eq!(
            parent_ids(&valid_parents(&details, "cheque", true, false)),
            vec!["assets", "bank", "savings", "gone"]
        );
        assert_eq!(
            parent_ids(&valid_parents(&details, "cheque", true, true)),
            vec!["assets", "bank", "savings"]
        );
    }

    #[test]
    fn valid_parents_of_unknown_account_is_empty() {
        assert_eq!(
            valid_parents(&fixture(), "missing", true, true),
            Vec::<&AccountDetail>::new()
        );
    }
}
