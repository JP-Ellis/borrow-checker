// Pure row logic for the Settings accounts table: ordering, depth and display
// text.
//
// Leptos-free and also mounted natively via `include!` in `main.rs`'s
// `pages_tests` shim, which is why this header uses `//` rather than `//!`.

use std::collections::HashMap;
use std::collections::HashSet;

use bc_ipc::AccountDetail;
use bc_ipc::AccountKindDto;
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

/// The commodity column text: codes comma-separated, in their stored order.
/// Empty when the account is unrestricted.
#[must_use]
pub(crate) fn commodities_text(details: &AccountDetail) -> String {
    details.commodity_codes.join(", ")
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::AccountType;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

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
    fn commodities_are_comma_separated_in_order() {
        let mut d = account("a", "A", None, AccountType::Asset);
        assert_eq!(commodities_text(&d), "");
        d.commodity_codes = vec!["AUD".to_owned(), "USD".to_owned()];
        assert_eq!(commodities_text(&d), "AUD, USD");
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
}
