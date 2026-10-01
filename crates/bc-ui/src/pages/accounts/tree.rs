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
        // Covers only future #[non_exhaustive] variants; every known variant
        // is matched explicitly above.
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

/// Returns the ids of every account below `id`, at any depth, excluding `id`.
///
/// Visits each account at most once, so a corrupt list with a cycle cannot
/// loop forever.
///
/// # Arguments
///
/// * `nodes` - The flat account list.
/// * `id` - The root account id.
#[must_use]
pub fn descendants_of(nodes: &[AccountNode], id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen: HashSet<&str> = HashSet::new();
    seen.insert(id);
    let mut frontier = vec![id];
    while let Some(parent) = frontier.pop() {
        for n in nodes {
            if n.parent_id.as_deref() == Some(parent) && seen.insert(n.id.as_str()) {
                out.push(n.id.clone());
                frontier.push(n.id.as_str());
            }
        }
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

/// Returns every account in sidebar order — roots by type then name, each
/// followed depth-first by its children by name — paired with its full path
/// (`Assets :: Bank :: Savings`).
///
/// Visits each account at most once, so a corrupt list with a cycle cannot
/// loop forever; an account unreachable from a root is left out.
///
/// # Arguments
///
/// * `nodes` - The flat account list.
#[must_use]
pub fn rail_entries(nodes: &[AccountNode]) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut stack: Vec<(AccountNode, String)> = ordered_roots(nodes)
        .into_iter()
        .rev()
        .map(|n| {
            let path = n.name.clone();
            (n, path)
        })
        .collect();
    while let Some((node, path)) = stack.pop() {
        if !seen.insert(node.id.clone()) {
            continue;
        }
        for child in children_of(nodes, &node.id).into_iter().rev() {
            let child_path = format!("{path} :: {}", child.name);
            stack.push((child, child_path));
        }
        out.push((node.id, path));
    }
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
    fn descendants_cover_every_level_below() {
        let mut ids = descendants_of(&fixture(), "assets");
        ids.sort();
        assert_eq!(ids, vec!["bank", "cheque", "savings"]);
    }

    #[test]
    fn descendants_of_a_leaf_are_empty() {
        assert_eq!(descendants_of(&fixture(), "savings"), Vec::<String>::new());
    }

    #[test]
    fn rail_lists_accounts_depth_first_with_full_paths() {
        assert_eq!(
            rail_entries(&fixture()),
            vec![
                ("assets".to_owned(), "Assets".to_owned()),
                ("bank".to_owned(), "Assets :: Bank".to_owned()),
                ("cheque".to_owned(), "Assets :: Bank :: Cheque".to_owned()),
                ("savings".to_owned(), "Assets :: Bank :: Savings".to_owned()),
                ("expenses".to_owned(), "Expenses".to_owned()),
                ("food".to_owned(), "Expenses :: Food".to_owned()),
            ]
        );
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
    fn roots_follow_type_order_then_name() {
        let ids: Vec<String> = ordered_roots(&fixture())
            .into_iter()
            .map(|n| n.id)
            .collect();
        assert_eq!(ids, vec!["assets".to_owned(), "expenses".to_owned()]);
    }
}
