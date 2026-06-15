//! Pure-Rust helper for deriving the category column label.

/// Sentinel displayed when a transaction spans multiple account types.
pub(crate) const SPLIT_LABEL: &str = "split transaction";

/// Derives the category column label from a list of counterpart account names.
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
/// A string ready to render in the Category column cell.
///
/// # Example
///
/// ```ignore
/// // pub(crate) — tested via unit tests in this module.
/// assert_eq!(
///     category_label(&["Expenses :: Groceries", "Expenses :: Healthcare"]),
///     "Expenses :: {Groceries, Healthcare}"
/// );
/// ```
pub(crate) fn category_label(counterpart_names: &[&str]) -> String {
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

    // If any path ends exactly at the shared prefix while others extend further,
    // the prefix itself is a leaf destination alongside its descendants.
    // Step back one level so everything appears as siblings:
    // e.g. ["Expenses :: Food", "Expenses :: Food :: Groceries"]
    //   →  "Expenses :: {Food, Food :: Groceries}"
    let any_empty = paths.iter().any(|p| p.len() == common_len);
    let any_deeper = paths.iter().any(|p| p.len() > common_len);
    if any_empty && any_deeper {
        // Step back one prefix level so the leaf and its descendants appear
        // as siblings. If there is no parent level, we cannot group them.
        let parent_len = common_len.checked_sub(1).filter(|&n| n > 0)?;
        let parent_prefix = first_path.get(..parent_len)?.join(" :: ");
        // Each sibling string is the path from parent_len onward.
        let mut iter = paths
            .iter()
            .map(|p| p.get(parent_len..).unwrap_or_default().join(" :: "));
        let siblings = join_iter(&mut iter, ", ");
        return Some(format!("{parent_prefix} :: {{{siblings}}}"));
    }

    // Group tails (suffix after the common prefix) by their first segment,
    // preserving input order.  Paths that end at exactly common_len are skipped
    // here — the any_empty && any_deeper branch above handles that case; the
    // remaining possibility (all equal to common_len) is caught after the loop.
    let mut groups: Vec<(&str, Vec<Vec<&str>>)> = Vec::new();
    for path in paths {
        let Some(tail) = path.get(common_len..) else {
            continue;
        };
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
        // All paths ended at the same prefix (e.g. duplicate accounts).
        return Some(prefix);
    }

    let mut group_iter = groups.iter().map(|(head, sub_tails)| {
        if sub_tails.iter().all(Vec::is_empty) {
            (*head).to_owned()
        } else {
            // Re-attach the head segment and recurse on the deeper paths.
            let full: Vec<Vec<&str>> = sub_tails
                .iter()
                .map(|t| core::iter::once(*head).chain(t.iter().copied()).collect())
                .collect();
            // Safe: all paths in `full` share `head` as their first segment.
            expand_trie(&full).unwrap_or_else(|| (*head).to_owned())
        }
    });

    Some(if groups.len() == 1 {
        format!("{prefix} :: {}", group_iter.next().unwrap_or_default())
    } else {
        format!("{prefix} :: {{{}}}", join_iter(&mut group_iter, ", "))
    })
}

/// Joins an iterator of strings with a separator without collecting first.
fn join_iter(iter: &mut impl Iterator<Item = String>, sep: &str) -> String {
    let mut out = String::new();
    for (i, s) in iter.enumerate() {
        if i > 0 {
            out.push_str(sep);
        }
        out.push_str(&s);
    }
    out
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::category_label;

    #[test]
    fn no_counterpart_returns_dash() {
        assert_eq!(category_label(&[]), "—");
    }

    #[test]
    fn single_counterpart_returned_as_is() {
        assert_eq!(
            category_label(&["Expenses :: Groceries"]),
            "Expenses :: Groceries"
        );
    }

    #[test]
    fn siblings_expand_with_braces() {
        assert_eq!(
            category_label(&[
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
            category_label(&[
                "Expenses :: Food :: Groceries",
                "Expenses :: Food :: Snacks",
            ]),
            "Expenses :: Food :: {Groceries, Snacks}"
        );
    }

    #[test]
    fn mixed_depth_within_same_type() {
        assert_eq!(
            category_label(&["Expenses :: Food :: Groceries", "Expenses :: Healthcare"]),
            "Expenses :: {Food :: Groceries, Healthcare}"
        );
    }

    #[test]
    fn deeply_nested_recursive_expansion() {
        assert_eq!(
            category_label(&[
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
            category_label(&["Expenses :: Groceries", "Income :: Interest"]),
            "split transaction"
        );
    }

    #[test]
    fn cross_type_three_way_split_returns_placeholder() {
        assert_eq!(
            category_label(&[
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
            category_label(&["Expenses :: Groceries", "Expenses :: Groceries"]),
            "Expenses :: Groceries"
        );
    }

    #[test]
    fn shorter_path_prefix_of_longer_shows_both() {
        // When one path is a strict prefix of another, both must appear as
        // siblings so neither destination is silently dropped.
        assert_eq!(
            category_label(&["Expenses :: Food", "Expenses :: Food :: Groceries"]),
            "Expenses :: {Food, Food :: Groceries}"
        );
    }
}
