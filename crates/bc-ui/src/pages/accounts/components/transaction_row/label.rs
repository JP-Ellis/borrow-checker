//! Pure-Rust helper for deriving the envelope column label.

/// Sentinel displayed when a transaction spans multiple account types.
pub(crate) const SPLIT_LABEL: &str = "split transaction";

/// Derives the envelope column label from a list of counterpart account names.
///
/// - Zero counterparts → `"—"`
/// - One counterpart → the account name verbatim
/// - Multiple counterparts with a common path prefix → recursive shell expansion,
///   e.g. `"Expenses :: {Groceries, Healthcare}"`
/// - Multiple counterparts with no common prefix → `"split transaction"`
///
/// # Arguments
///
/// * `counterpart_names` - Display names of all postings that are not the
///   currently-viewed account, in the order they appear on the transaction.
///
/// # Returns
///
/// A string ready to render in the Envelope column cell.
///
/// # Example
///
/// ```ignore
/// // pub(crate) — tested via unit tests in this module.
/// assert_eq!(
///     envelope_label(&["Expenses :: Groceries", "Expenses :: Healthcare"]),
///     "Expenses :: {Groceries, Healthcare}"
/// );
/// ```
pub(crate) fn envelope_label(counterpart_names: &[&str]) -> String {
    match counterpart_names {
        [] => "—".to_owned(),
        [single] => (*single).to_owned(),
        names => {
            let paths: Vec<Vec<&str>> = names.iter().map(|n| n.split(" :: ").collect()).collect();
            expand_trie(&paths).unwrap_or_else(|| SPLIT_LABEL.to_owned())
        }
    }
}

/// Recursively serialises a list of segmented account paths as a shell-expansion
/// string, e.g. `["A","B"]` + `["A","C"]` → `"A :: {B, C}"`.
///
/// # Arguments
///
/// * `paths` - Segmented account paths to expand.
///
/// # Returns
///
/// Returns `None` when there is no common prefix across paths (cross-type split).
fn expand_trie(paths: &[Vec<&str>]) -> Option<String> {
    let (first_path, rest_paths) = paths.split_first()?;

    if rest_paths.is_empty() {
        return Some(first_path.join(" :: "));
    }

    // Length of the shared prefix across all paths.
    let common_len = first_path
        .iter()
        .enumerate()
        .take_while(|&(i, seg)| rest_paths.iter().all(|p| p.get(i) == Some(seg)))
        .count();

    if common_len == 0 {
        return None;
    }

    let prefix = first_path.get(..common_len)?.join(" :: ");
    let tails: Vec<Vec<&str>> = paths
        .iter()
        // Safe: common_len ≤ min path length, so the slice always exists.
        .map(|p| p.get(common_len..).unwrap_or_default().to_vec())
        .collect();

    // Group tails by their first segment, preserving input order.
    let mut groups: Vec<(&str, Vec<Vec<&str>>)> = Vec::new();
    for tail in &tails {
        let Some((head, tail_rest)) = tail.split_first() else {
            continue;
        };
        let rest = tail_rest.to_vec();
        if let Some(g) = groups.iter_mut().find(|(k, _)| k == head) {
            g.1.push(rest);
        } else {
            groups.push((head, vec![rest]));
        }
    }

    if groups.is_empty() {
        return Some(prefix);
    }

    let group_strs: Vec<String> = groups
        .iter()
        .map(|(head, sub_tails)| {
            if sub_tails.iter().all(Vec::is_empty) {
                (*head).to_owned()
            } else {
                // Re-attach the head segment and recurse.
                let full: Vec<Vec<&str>> = sub_tails
                    .iter()
                    .map(|t| core::iter::once(*head).chain(t.iter().copied()).collect())
                    .collect();
                // Safe: all paths in `full` share `head` as their first segment.
                expand_trie(&full).unwrap_or_else(|| (*head).to_owned())
            }
        })
        .collect();

    #[expect(
        clippy::indexing_slicing,
        reason = "guarded by group_strs.len() == 1 check immediately above"
    )]
    Some(if group_strs.len() == 1 {
        format!("{prefix} :: {}", group_strs[0])
    } else {
        format!("{prefix} :: {{{}}}", group_strs.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::envelope_label;

    #[test]
    fn no_counterpart_returns_dash() {
        assert_eq!(envelope_label(&[]), "—");
    }

    #[test]
    fn single_counterpart_returned_as_is() {
        assert_eq!(
            envelope_label(&["Expenses :: Groceries"]),
            "Expenses :: Groceries"
        );
    }

    #[test]
    fn siblings_expand_with_braces() {
        assert_eq!(
            envelope_label(&[
                "Expenses :: Groceries",
                "Expenses :: Alcohol",
                "Expenses :: Snacks",
            ]),
            "Expenses :: {Groceries, Alcohol, Snacks}"
        );
    }

    #[test]
    fn deep_siblings_use_full_common_prefix() {
        assert_eq!(
            envelope_label(&[
                "Expenses :: Food :: Groceries",
                "Expenses :: Food :: Snacks",
            ]),
            "Expenses :: Food :: {Groceries, Snacks}"
        );
    }

    #[test]
    fn mixed_depth_within_same_type() {
        assert_eq!(
            envelope_label(&["Expenses :: Food :: Groceries", "Expenses :: Healthcare",]),
            "Expenses :: {Food :: Groceries, Healthcare}"
        );
    }

    #[test]
    fn deeply_nested_recursive_expansion() {
        assert_eq!(
            envelope_label(&[
                "Expenses :: Utilities :: Electricity :: Usage",
                "Expenses :: Utilities :: Electricity :: Connection",
                "Expenses :: Utilities :: Gas :: Usage",
                "Expenses :: Utilities :: Gas :: Connection",
            ]),
            "Expenses :: Utilities :: {Electricity :: {Usage, Connection}, Gas :: {Usage, Connection}}"
        );
    }

    #[test]
    fn cross_type_split_returns_placeholder() {
        assert_eq!(
            envelope_label(&["Expenses :: Groceries", "Income :: Interest"]),
            "split transaction"
        );
    }

    #[test]
    fn cross_type_three_way_split_returns_placeholder() {
        assert_eq!(
            envelope_label(&[
                "Expenses :: Groceries",
                "Expenses :: Healthcare",
                "Income :: Interest",
            ]),
            "split transaction"
        );
    }

    #[test]
    fn duplicate_counterparts_collapse_to_single() {
        assert_eq!(
            envelope_label(&["Expenses :: Groceries", "Expenses :: Groceries"]),
            "Expenses :: Groceries"
        );
    }

    #[test]
    fn shorter_path_prefix_of_longer_is_collapsed() {
        // When one path is a strict prefix of another, the algorithm drops the
        // shorter path's leaf and produces the longer path. This pins the
        // current behaviour so regressions are visible.
        assert_eq!(
            envelope_label(&["Expenses :: Food", "Expenses :: Food :: Groceries"]),
            "Expenses :: Food :: Groceries"
        );
    }
}
