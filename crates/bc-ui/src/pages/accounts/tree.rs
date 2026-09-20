//! Pure helpers over the flat account list: hierarchy walks and root ordering.
//! Kept target-agnostic so it is native-testable.

use std::collections::HashSet;

use bc_ipc::AccountNode;
use bc_ipc::AccountType;

/// Sort key for the fixed type order of the sidebar's top level.
fn type_rank(ty: AccountType) -> u8 {
    match ty {
        AccountType::Asset => 0,
        AccountType::Liability => 1,
        AccountType::Equity => 2,
        AccountType::Income => 3,
        AccountType::Expense => 4,
        #[cfg_attr(
            target_arch = "wasm32",
            expect(
                clippy::wildcard_enum_match_arm,
                reason = "AccountType is #[non_exhaustive]; unknown variants sort last"
            )
        )]
        _ => 5,
    }
}

/// Returns the direct children of `parent`, sorted by name.
///
/// # Arguments
///
/// * `nodes` - The flat account list.
/// * `parent` - The parent account id.
#[must_use]
pub fn children_of(nodes: &[AccountNode], parent: &str) -> Vec<AccountNode> {
    let mut out: Vec<AccountNode> = nodes
        .iter()
        .filter(|n| n.parent_id.as_deref() == Some(parent))
        .cloned()
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Returns the ancestor ids of `id`, nearest first, excluding `id`.
///
/// Stops at the first missing parent, and on a cycle so a corrupt list
/// cannot loop forever.
///
/// # Arguments
///
/// * `nodes` - The flat account list.
/// * `id` - The starting account id.
#[must_use]
pub fn ancestors_of(nodes: &[AccountNode], id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    seen.insert(id);
    let mut cursor = nodes
        .iter()
        .find(|n| n.id == id)
        .and_then(|n| n.parent_id.as_deref());
    while let Some(pid) = cursor {
        if !seen.insert(pid) {
            break;
        }
        out.push(pid.to_owned());
        cursor = nodes
            .iter()
            .find(|n| n.id == pid)
            .and_then(|n| n.parent_id.as_deref());
    }
    out
}

/// Returns `id` plus every descendant id.
///
/// # Arguments
///
/// * `nodes` - The flat account list.
/// * `id` - The subtree root.
#[must_use]
pub fn descendants_of(nodes: &[AccountNode], id: &str) -> HashSet<String> {
    let mut out: HashSet<String> = HashSet::new();
    let mut frontier = vec![id.to_owned()];
    while let Some(current) = frontier.pop() {
        if !out.insert(current.clone()) {
            continue;
        }
        frontier.extend(
            nodes
                .iter()
                .filter(|n| n.parent_id.as_deref() == Some(current.as_str()))
                .map(|n| n.id.clone()),
        );
    }
    out
}

/// Returns the top-level accounts in sidebar order: by type (asset,
/// liability, equity, income, expense), then by name.
///
/// # Arguments
///
/// * `nodes` - The flat account list.
#[must_use]
pub fn ordered_roots(nodes: &[AccountNode]) -> Vec<AccountNode> {
    let mut out: Vec<AccountNode> = nodes
        .iter()
        .filter(|n| n.parent_id.is_none())
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        type_rank(a.account_type)
            .cmp(&type_rank(b.account_type))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    fn node(id: &str, name: &str, parent: Option<&str>, ty: AccountType) -> AccountNode {
        AccountNode::new(id, name, None::<&str>, None, parent, ty, vec![], None, None)
    }

    fn fixture() -> Vec<AccountNode> {
        vec![
            node("expenses", "Expenses", None, AccountType::Expense),
            node("assets", "Assets", None, AccountType::Asset),
            node("bank", "Bank", Some("assets"), AccountType::Asset),
            node("savings", "Savings", Some("bank"), AccountType::Asset),
            node("cheque", "Cheque", Some("bank"), AccountType::Asset),
            node("food", "Food", Some("expenses"), AccountType::Expense),
        ]
    }

    #[test]
    fn children_are_direct_and_sorted() {
        let ids: Vec<String> = children_of(&fixture(), "bank")
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(ids, vec!["cheque".to_owned(), "savings".to_owned()]);
        assert!(children_of(&fixture(), "savings").is_empty());
    }

    #[test]
    fn ancestors_nearest_first_excluding_self() {
        assert_eq!(
            ancestors_of(&fixture(), "savings"),
            vec!["bank".to_owned(), "assets".to_owned()]
        );
        assert!(ancestors_of(&fixture(), "assets").is_empty());
        assert!(ancestors_of(&fixture(), "missing").is_empty());
    }

    #[test]
    fn ancestors_stop_on_cycle() {
        let nodes = vec![
            node("a", "A", Some("b"), AccountType::Asset),
            node("b", "B", Some("a"), AccountType::Asset),
        ];
        assert_eq!(ancestors_of(&nodes, "a"), vec!["b".to_owned()]);
    }

    #[test]
    fn descendants_are_inclusive() {
        let set = descendants_of(&fixture(), "bank");
        let mut got: Vec<&str> = set.iter().map(String::as_str).collect();
        got.sort_unstable();
        assert_eq!(got, vec!["bank", "cheque", "savings"]);
        assert_eq!(descendants_of(&fixture(), "food").len(), 1);
    }

    #[test]
    fn roots_follow_type_order_then_name() {
        let ids: Vec<String> = ordered_roots(&fixture())
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(ids, vec!["assets".to_owned(), "expenses".to_owned()]);
    }
}
