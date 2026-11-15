// Pure row logic for the Settings accounts table: ordering, depth, display
// text, the edit draft with its patch or create arguments, and the row state
// a save updates.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim, which is why this header uses `//` rather than `//!`.

use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr as _;

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
use bc_ipc::AccountPatchDto;
use bc_ipc::AccountType;
use bc_ipc::AcquisitionDto;
use bc_ipc::BcError;
use bc_ipc::TagInfo;
use bc_ipc::commands::CreateAccountArgs;
use jiff::civil::Date;
use rust_decimal::Decimal;

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
/// patch that a save sends. A new account starts from [`Default`] or
/// [`RowDraft::new_account`], and [`create_args`] turns it into the create
/// request.
#[derive(Clone, Debug, PartialEq)]
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
    /// Cost text that does not parse as a number, kept as typed.
    pub unparsed_cost: Option<String>,
    /// Annual-rate text that does not parse as a number, kept as typed.
    pub unparsed_rate: Option<String>,
    /// Account type; on a new account `None` until a parent or a type is
    /// chosen. Fixed once saved.
    pub account_type: Option<AccountType>,
    /// How the account is maintained. Fixed once saved.
    pub kind: AccountKindDto,
}

impl Default for RowDraft {
    /// A blank new-account draft: a deposit account with no type yet.
    #[inline]
    fn default() -> Self {
        Self {
            name: String::new(),
            parent_id: None,
            description: String::new(),
            tags: Vec::new(),
            commodity_codes: Vec::new(),
            opened_on: None,
            closed_on: None,
            acquisition: None,
            unparsed_cost: None,
            unparsed_rate: None,
            account_type: None,
            kind: AccountKindDto::DepositAccount,
        }
    }
}

impl RowDraft {
    /// Whether a field holds text that does not parse, which blocks the
    /// row's save and keeps it dirty.
    #[must_use]
    pub(crate) fn has_unparsed(&self) -> bool {
        self.unparsed_cost.is_some() || self.unparsed_rate.is_some()
    }

    /// A blank new-account draft under `parent_id`, or of `account_type` at
    /// top level.
    ///
    /// # Arguments
    ///
    /// * `details` - Every saved account.
    /// * `account_type` - The type to start with; a known parent overrides it.
    /// * `parent_id` - The parent to start under; an unknown ID is dropped.
    #[must_use]
    pub(crate) fn new_account(
        details: &[AccountDetail],
        account_type: Option<AccountType>,
        parent_id: Option<String>,
    ) -> Self {
        let mut draft = Self {
            account_type,
            ..Self::default()
        };
        draft.set_parent(details, parent_id);
        draft
    }

    /// Sets a new account's kind. Any other kind than manual asset drops the
    /// acquisition record and its unparsed text, since only a manual asset
    /// holds one.
    ///
    /// # Arguments
    ///
    /// * `kind` - The chosen kind.
    pub(crate) fn set_kind(&mut self, kind: AccountKindDto) {
        self.kind = kind;
        if kind != AccountKindDto::ManualAsset {
            self.acquisition = None;
            self.unparsed_cost = None;
            self.unparsed_rate = None;
        }
    }

    /// Moves a new account under `parent_id`, which fixes its type to the
    /// parent's. Top level keeps the type, as does an ID that `new_parents`
    /// would not offer: unknown, closed or archived.
    ///
    /// # Arguments
    ///
    /// * `details` - Every saved account.
    /// * `parent_id` - The parent, or `None` for top level.
    pub(crate) fn set_parent(&mut self, details: &[AccountDetail], parent_id: Option<String>) {
        let parent =
            parent_id.and_then(|id| new_parents(details, None).into_iter().find(|d| d.id == id));
        self.parent_id = parent.map(|p| p.id.clone());
        if let Some(p) = parent {
            self.account_type = Some(p.account_type);
        }
    }
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
            unparsed_cost: None,
            unparsed_rate: None,
            account_type: Some(details.account_type),
            kind: details.kind,
        }
    }
}

/// Parses a decimal field: blank is `None`.
///
/// # Arguments
///
/// * `text` - The field's text, surrounding whitespace ignored.
///
/// # Errors
///
/// Returns the message to show beside the field when `text` is not a
/// number.
pub(crate) fn parse_decimal(text: &str) -> Result<Option<Decimal>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Decimal::from_str(trimmed)
        .map(Some)
        .map_err(|_not_a_number| format!("“{trimmed}” is not a number"))
}

/// Whether a row needs saving: its draft differs from `base`, or a field
/// holds text that does not parse.
#[must_use]
pub(crate) fn is_dirty(base: &AccountDetail, edited: &RowDraft) -> bool {
    edited.has_unparsed() || diff(base, edited).is_some()
}

/// The patch that turns `base` into `edited`, carrying only changed fields.
///
/// Name and description compare trimmed; a description of only whitespace
/// clears it.
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
    let name = edited.name.trim();
    if name != base.name {
        patch.name = Some(name.to_owned());
    }
    if edited.parent_id != base.parent_id {
        patch.parent_id = Some(edited.parent_id.clone());
    }
    let description = edited.description.trim();
    if description != base.description.as_deref().unwrap_or_default() {
        patch.description = Some((!description.is_empty()).then(|| description.to_owned()));
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

/// The parent picker's options for a new account, in tree order: saved
/// accounts that are open, active and of `account_type`, or of any type
/// while it is `None`. Top level is always valid and is not listed.
///
/// # Arguments
///
/// * `details` - Every saved account.
/// * `account_type` - The new account's type, if chosen.
#[must_use]
pub(crate) fn new_parents(
    details: &[AccountDetail],
    account_type: Option<AccountType>,
) -> Vec<&AccountDetail> {
    tree_order(details)
        .into_iter()
        .map(|(_, d)| d)
        .filter(|d| account_type.is_none_or(|t| d.account_type == t))
        .filter(|d| d.closed_on.is_none() && !d.archived)
        .collect()
}

/// The create request for a new account's draft.
///
/// Name and description are trimmed; a blank description is none. The
/// acquisition record goes only with a manual asset, and only when it holds
/// something.
///
/// # Arguments
///
/// * `draft` - The new account's draft.
///
/// # Errors
///
/// Returns the message naming the missing field: the name, or the parent or
/// type.
pub(crate) fn create_args(draft: &RowDraft) -> Result<CreateAccountArgs, String> {
    let name = draft.name.trim();
    if name.is_empty() {
        return Err("name is required".to_owned());
    }
    let Some(account_type) = draft.account_type else {
        return Err("pick a parent or a type".to_owned());
    };
    let description = draft.description.trim();
    let acquisition = draft
        .acquisition
        .clone()
        .filter(|a| draft.kind == AccountKindDto::ManualAsset && *a != AcquisitionDto::default());
    Ok(CreateAccountArgs::builder()
        .name(name.to_owned())
        .maybe_parent_id(draft.parent_id.clone())
        .account_type(account_type)
        .kind(draft.kind)
        .maybe_description((!description.is_empty()).then(|| description.to_owned()))
        .commodity_codes(draft.commodity_codes.clone())
        .tag_ids(draft.tags.iter().map(|t| t.id.clone()).collect())
        .maybe_opened_on(draft.opened_on)
        .maybe_acquisition(acquisition)
        .build())
}

/// The parent picker's options for `id`, in tree order: [`valid_parents`]
/// plus the account's saved parent, which a closed or archived parent may
/// have left out.
///
/// # Arguments
///
/// * `details` - Every saved account.
/// * `id` - The account being edited.
/// * `open` - Whether the account is open.
/// * `active` - Whether the account is not archived.
///
/// # Returns
///
/// The options; empty when `id` is not in `details`.
#[must_use]
pub(crate) fn parent_choices<'a>(
    details: &'a [AccountDetail],
    id: &str,
    open: bool,
    active: bool,
) -> Vec<&'a AccountDetail> {
    let valid: HashSet<&str> = valid_parents(details, id, open, active)
        .into_iter()
        .map(|d| d.id.as_str())
        .collect();
    let saved = details
        .iter()
        .find(|d| d.id == id)
        .and_then(|d| d.parent_id.as_deref());
    tree_order(details)
        .into_iter()
        .map(|(_, d)| d)
        .filter(|d| valid.contains(d.id.as_str()) || saved == Some(d.id.as_str()))
        .collect()
}

// MARK: Row state

/// One account row: its saved state, the user's draft, and the last save
/// error.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Row {
    /// The account as last loaded or saved; its `event_count` is the save base.
    pub base: AccountDetail,
    /// The user's edits; equal to `base` when the row is clean.
    pub draft: RowDraft,
    /// Why the last save or reload of this row failed.
    pub error: Option<BcError>,
}

impl Row {
    /// A clean row over `base`.
    #[must_use]
    pub(crate) fn new(base: AccountDetail) -> Self {
        Self {
            draft: RowDraft::from(&base),
            base,
            error: None,
        }
    }

    /// Whether the row needs saving; see [`is_dirty`].
    #[must_use]
    pub(crate) fn is_dirty(&self) -> bool {
        is_dirty(&self.base, &self.draft)
    }
}

/// One unsaved new account: a local key, its draft, and the last create
/// error.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NewRow {
    /// Local key, distinct from every other draft's; never an account ID.
    pub key: u64,
    /// The account as the user has left it.
    pub draft: RowDraft,
    /// Why the last create of this draft failed.
    pub error: Option<BcError>,
}

/// The table's unsaved new accounts, in the order they were added.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Drafts {
    /// The key the next draft takes.
    next_key: u64,
    /// The drafts, oldest first.
    rows: Vec<NewRow>,
}

impl Drafts {
    /// Appends a blank draft; see [`RowDraft::new_account`].
    ///
    /// # Arguments
    ///
    /// * `details` - Every saved account.
    /// * `account_type` - The type to start with; a known parent overrides it.
    /// * `parent_id` - The parent to start under; an unknown ID is dropped.
    ///
    /// # Returns
    ///
    /// The new draft's key.
    pub(crate) fn add(
        &mut self,
        details: &[AccountDetail],
        account_type: Option<AccountType>,
        parent_id: Option<String>,
    ) -> u64 {
        let key = self.next_key;
        self.next_key = self.next_key.saturating_add(1);
        self.rows.push(NewRow {
            key,
            draft: RowDraft::new_account(details, account_type, parent_id),
            error: None,
        });
        key
    }

    /// Removes the draft `key`, if present.
    ///
    /// # Returns
    ///
    /// The removed draft.
    pub(crate) fn remove(&mut self, key: u64) -> Option<NewRow> {
        let at = self.rows.iter().position(|r| r.key == key)?;
        Some(self.rows.remove(at))
    }

    /// Removes every draft. Keys stay unused.
    pub(crate) fn clear(&mut self) {
        self.rows.clear();
    }

    /// The drafts, oldest first.
    #[must_use]
    pub(crate) fn rows(&self) -> &[NewRow] {
        &self.rows
    }

    /// The draft `key`, if present.
    #[must_use]
    pub(crate) fn get(&self, key: u64) -> Option<&NewRow> {
        self.rows.iter().find(|r| r.key == key)
    }

    /// The draft `key`, if present, for editing.
    #[must_use]
    pub(crate) fn get_mut(&mut self, key: u64) -> Option<&mut NewRow> {
        self.rows.iter_mut().find(|r| r.key == key)
    }
}

/// Folds a fresh account list into the rows, so derived fields such as a
/// renamed parent's descendants' paths are current.
///
/// A clean row takes the fresh detail outright. A dirty or failed row keeps
/// its draft and error, and takes the fresh detail as its base only when its
/// event count is unchanged: a newer base would hide a conflict from the next
/// save. An account new to the list joins as a clean row.
///
/// # Arguments
///
/// * `rows` - The table's rows, updated in place.
/// * `fresh` - The account list as just read from the backend.
pub(crate) fn merge_fresh(rows: &mut Vec<Row>, fresh: Vec<AccountDetail>) {
    for detail in fresh {
        match rows.iter_mut().find(|r| r.base.id == detail.id) {
            Some(r) if r.error.is_none() && !r.is_dirty() => *r = Row::new(detail),
            Some(r) => {
                if r.base.event_count == detail.event_count {
                    r.base = detail;
                }
            }
            None => rows.push(Row::new(detail)),
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::DepreciationPolicyDto;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
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
    fn correcting_a_close_date_sends_only_the_date() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.closed_on = Some(date(2025, 6, 30));
        let mut draft = RowDraft::from(&d);
        draft.closed_on = Some(date(2025, 5, 31));
        assert_eq!(
            diff(&d, &draft),
            Some(
                AccountPatchDto::builder()
                    .closed_on(date(2025, 5, 31))
                    .build()
            )
        );
    }

    /// The patch cannot clear a close date; reopening does that.
    #[test]
    fn clearing_a_close_date_sends_nothing() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.closed_on = Some(date(2025, 6, 30));
        let mut draft = RowDraft::from(&d);
        draft.closed_on = None;
        assert_eq!(diff(&d, &draft), None);
    }

    #[test]
    fn emptied_acquisition_is_no_change() {
        let mut car = manual_asset();
        car.acquisition = None;
        let mut draft = RowDraft::from(&car);
        draft.acquisition = Some(AcquisitionDto::default());
        assert_eq!(diff(&car, &draft), None);
    }

    #[rstest]
    #[case("", Ok(None))]
    #[case("  ", Ok(None))]
    #[case("12000", Ok(Some(dec!(12000))))]
    #[case(" 0.25 ", Ok(Some(dec!(0.25))))]
    #[case("12,000", Err("“12,000” is not a number".to_owned()))]
    #[case("abc", Err("“abc” is not a number".to_owned()))]
    fn decimal_fields_parse_or_explain(
        #[case] text: &str,
        #[case] expected: Result<Option<Decimal>, String>,
    ) {
        assert_eq!(parse_decimal(text), expected);
    }

    #[test]
    fn unparsed_cost_keeps_an_otherwise_clean_row_dirty() {
        let car = manual_asset();
        let mut draft = RowDraft::from(&car);
        assert!(!is_dirty(&car, &draft));
        draft.unparsed_cost = Some("12,000".to_owned());
        assert!(is_dirty(&car, &draft));
        assert_eq!(diff(&car, &draft), None);
        draft.unparsed_cost = None;
        draft.unparsed_rate = Some("a fifth".to_owned());
        assert!(is_dirty(&car, &draft));
    }

    #[test]
    fn name_and_description_compare_trimmed() {
        let mut d = account("bank", "Bank", Some("assets"), AccountType::Asset);
        d.description = Some("Everyday banking".to_owned());
        let mut draft = RowDraft::from(&d);
        draft.name = " Bank ".to_owned();
        draft.description = "Everyday banking  ".to_owned();
        assert_eq!(diff(&d, &draft), None);
        draft.name = "  Banking ".to_owned();
        draft.description = "   ".to_owned();
        assert_eq!(
            diff(&d, &draft),
            Some(
                AccountPatchDto::builder()
                    .name("Banking")
                    .description(None)
                    .build()
            )
        );
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
    fn valid_parents_excludes_archived_for_active_account() {
        let details = tree_for_parents();
        assert_eq!(
            parent_ids(&valid_parents(&details, "cheque", false, true)),
            vec!["assets", "bank", "savings", "old"]
        );
    }

    #[test]
    fn parent_choices_keep_a_saved_archived_parent_only() {
        let mut details = tree_for_parents();
        details.push(account("kid", "Kid", Some("gone"), AccountType::Asset));
        assert_eq!(
            parent_ids(&valid_parents(&details, "kid", true, true)),
            vec!["assets", "bank", "cheque", "savings"]
        );
        assert_eq!(
            parent_ids(&parent_choices(&details, "kid", true, true)),
            vec!["assets", "bank", "cheque", "savings", "gone"]
        );
        assert_eq!(
            parent_ids(&parent_choices(&details, "cheque", false, true)),
            vec!["assets", "bank", "savings", "kid", "old"],
            "an active account is not offered an archived parent it does not have"
        );
    }

    #[test]
    fn valid_parents_of_unknown_account_is_empty() {
        assert_eq!(
            valid_parents(&fixture(), "missing", true, true),
            Vec::<&AccountDetail>::new()
        );
    }

    #[test]
    fn parent_choices_keep_a_saved_parent_that_is_no_longer_valid() {
        let mut details = tree_for_parents();
        details.push(account("kid", "Kid", Some("old"), AccountType::Asset));
        assert_eq!(
            parent_ids(&valid_parents(&details, "kid", true, true)),
            vec!["assets", "bank", "cheque", "savings"]
        );
        assert_eq!(
            parent_ids(&parent_choices(&details, "kid", true, true)),
            vec!["assets", "bank", "cheque", "savings", "old"]
        );
        assert_eq!(
            parent_ids(&parent_choices(&details, "cheque", true, true)),
            parent_ids(&valid_parents(&details, "cheque", true, true))
        );
    }

    // MARK: Merging a fresh list

    fn renamed(mut d: AccountDetail, path: &str) -> AccountDetail {
        d.path = path.to_owned();
        d
    }

    #[test]
    fn merge_gives_a_clean_row_the_fresh_detail() {
        let bank = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut rows = vec![Row::new(bank.clone())];
        let mut fresh = renamed(bank, "Holdings :: Bank");
        fresh.event_count = 3;
        merge_fresh(&mut rows, vec![fresh.clone()]);
        assert_eq!(rows, vec![Row::new(fresh)]);
    }

    #[test]
    fn merge_gives_a_dirty_row_an_unchanged_base_and_keeps_its_draft() {
        let bank = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut row = Row::new(bank.clone());
        row.draft.description = "Edited".to_owned();
        let mut rows = vec![row.clone()];
        let fresh = renamed(bank, "Holdings :: Bank");
        merge_fresh(&mut rows, vec![fresh.clone()]);
        assert_eq!(rows, vec![Row { base: fresh, ..row }]);
    }

    #[test]
    fn merge_keeps_the_old_base_of_a_dirty_row_whose_count_moved() {
        let bank = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut row = Row::new(bank.clone());
        row.draft.description = "Edited".to_owned();
        let before = row.clone();
        let mut rows = vec![row];
        let mut fresh = renamed(bank, "Holdings :: Bank");
        fresh.description = Some("Edited elsewhere".to_owned());
        fresh.event_count = 1;
        merge_fresh(&mut rows, vec![fresh]);
        assert_eq!(rows, vec![before]);
    }

    #[test]
    fn merge_keeps_the_error_of_a_failed_row() {
        let bank = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut row = Row::new(bank.clone());
        row.error = Some(BcError::Validation("sibling clash".to_owned()));
        let mut rows = vec![row.clone()];
        let fresh = renamed(bank, "Holdings :: Bank");
        merge_fresh(&mut rows, vec![fresh.clone()]);
        assert_eq!(rows, vec![Row { base: fresh, ..row }]);
    }

    // MARK: Adding

    #[test]
    fn create_needs_name_and_parent_or_type() {
        let mut draft = RowDraft::default();
        assert_eq!(
            create_args(&draft).err(),
            Some("name is required".to_owned())
        );
        draft.name = "   ".to_owned();
        assert_eq!(
            create_args(&draft).err(),
            Some("name is required".to_owned())
        );
        draft.name = " Travel ".to_owned();
        assert_eq!(
            create_args(&draft).err(),
            Some("pick a parent or a type".to_owned())
        );
        draft.account_type = Some(AccountType::Expense);
        draft.description = "  ".to_owned();
        draft.tags = vec![TagInfo::new("t2", "work"), TagInfo::new("t1", "joint")];
        let args = create_args(&draft).expect("a complete draft");
        assert_eq!(args.name, "Travel");
        assert_eq!(args.parent_id, None);
        assert_eq!(args.account_type, AccountType::Expense);
        assert_eq!(args.kind, AccountKindDto::DepositAccount);
        assert_eq!(args.description, None);
        assert_eq!(args.tag_ids, vec!["t2".to_owned(), "t1".to_owned()]);
    }

    #[test]
    fn create_type_comes_from_parent() {
        let details = fixture();
        let mut draft =
            RowDraft::new_account(&details, Some(AccountType::Asset), Some("food".to_owned()));
        assert_eq!(draft.parent_id.as_deref(), Some("food"));
        assert_eq!(draft.account_type, Some(AccountType::Expense));
        draft.name = "Snacks".to_owned();
        let args = create_args(&draft).expect("a complete draft");
        assert_eq!(args.parent_id.as_deref(), Some("food"));
        assert_eq!(args.account_type, AccountType::Expense);

        draft.set_parent(&details, Some("bank".to_owned()));
        assert_eq!(draft.account_type, Some(AccountType::Asset));
        // Moving to top level keeps the type the parent gave.
        draft.set_parent(&details, None);
        assert_eq!(draft.parent_id, None);
        assert_eq!(draft.account_type, Some(AccountType::Asset));

        let unknown = RowDraft::new_account(
            &details,
            Some(AccountType::Income),
            Some("missing".to_owned()),
        );
        assert_eq!(unknown.parent_id, None);
        assert_eq!(unknown.account_type, Some(AccountType::Income));
    }

    #[test]
    fn a_closed_or_archived_parent_is_dropped() {
        let details = tree_for_parents();
        for id in ["old", "gone"] {
            let typed =
                RowDraft::new_account(&details, Some(AccountType::Income), Some(id.to_owned()));
            assert_eq!(typed.parent_id, None, "{id}");
            assert_eq!(typed.account_type, Some(AccountType::Income), "{id}");
            let untyped = RowDraft::new_account(&details, None, Some(id.to_owned()));
            assert_eq!(untyped.parent_id, None, "{id}");
            assert_eq!(untyped.account_type, None, "{id}");
        }
    }

    #[test]
    fn acquisition_only_for_manual_asset() {
        let record = AcquisitionDto::builder()
            .date(date(2026, 10, 1))
            .cost(dec!(9000))
            .build();
        let mut draft = RowDraft {
            name: "Boat".to_owned(),
            account_type: Some(AccountType::Asset),
            acquisition: Some(record.clone()),
            ..RowDraft::default()
        };
        assert_eq!(create_args(&draft).expect("complete").acquisition, None);
        draft.kind = AccountKindDto::ManualAsset;
        assert_eq!(
            create_args(&draft).expect("complete").acquisition,
            Some(record)
        );
        draft.acquisition = Some(AcquisitionDto::default());
        assert_eq!(create_args(&draft).expect("complete").acquisition, None);
    }

    #[test]
    fn leaving_manual_asset_drops_the_acquisition() {
        let record = AcquisitionDto::builder().cost(dec!(9000)).build();
        let mut draft = RowDraft::default();
        draft.set_kind(AccountKindDto::ManualAsset);
        draft.acquisition = Some(record.clone());
        draft.unparsed_rate = Some("a fifth".to_owned());
        draft.set_kind(AccountKindDto::ManualAsset);
        assert_eq!(draft.acquisition, Some(record));
        assert!(draft.has_unparsed());
        draft.set_kind(AccountKindDto::Receivable);
        assert_eq!(draft.kind, AccountKindDto::Receivable);
        assert_eq!(draft.acquisition, None);
        assert!(!draft.has_unparsed());
    }

    #[test]
    fn new_parents_are_open_active_and_of_the_type() {
        let details = tree_for_parents();
        assert_eq!(
            parent_ids(&new_parents(&details, Some(AccountType::Asset))),
            vec!["assets", "bank", "cheque", "savings"]
        );
        assert_eq!(
            parent_ids(&new_parents(&details, None)),
            vec![
                "assets", "bank", "cheque", "savings", "cards", "expenses", "food"
            ]
        );
    }

    #[test]
    fn drafts_take_keys_that_are_never_reused() {
        let details = fixture();
        let mut drafts = Drafts::default();
        let first = drafts.add(&details, None, None);
        let second = drafts.add(&details, Some(AccountType::Income), None);
        assert_ne!(first, second);
        assert_eq!(drafts.remove(first).map(|r| r.key), Some(first));
        assert_eq!(drafts.remove(first), None);
        let third = drafts.add(&details, None, Some("bank".to_owned()));
        assert!(third != first && third != second);
        assert_eq!(
            drafts.rows().iter().map(|r| r.key).collect::<Vec<_>>(),
            vec![second, third]
        );
        assert_eq!(
            drafts.get(third).map(|r| r.draft.account_type),
            Some(Some(AccountType::Asset))
        );
        if let Some(r) = drafts.get_mut(second) {
            r.draft.name = "Bonus".to_owned();
        }
        assert_eq!(
            drafts
                .get(second)
                .map(|r| (r.draft.name.as_str(), r.draft.kind)),
            Some(("Bonus", AccountKindDto::DepositAccount))
        );
        drafts.clear();
        assert_eq!(drafts.rows().len(), 0);
        assert!(drafts.add(&details, None, None) > third);
    }

    #[test]
    fn merge_adds_a_new_account_as_a_clean_row() {
        let bank = account("bank", "Bank", Some("assets"), AccountType::Asset);
        let mut rows = vec![Row::new(bank.clone())];
        let new = account("broker", "Broker", Some("assets"), AccountType::Asset);
        merge_fresh(&mut rows, vec![bank.clone(), new.clone()]);
        assert_eq!(rows, vec![Row::new(bank), Row::new(new)]);
    }
}
