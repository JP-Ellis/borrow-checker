use bc_ipc::TagInfo;

/// Filters `all` to tags whose path contains `query` and are not already in `selected`.
///
/// Matching is case-insensitive substring on the full tag path. An empty or
/// whitespace-only query returns every not-yet-selected tag. Input order is
/// preserved.
///
/// # Arguments
///
/// * `all` - The full list of known tags.
/// * `query` - The user's search text.
/// * `selected` - Tag paths already chosen; these are excluded from results.
///
/// # Returns
///
/// The matching, not-yet-selected tags in input order.
#[must_use]
pub fn filter_tags(all: &[TagInfo], query: &str, selected: &[String]) -> Vec<TagInfo> {
    let q = query.trim().to_lowercase();
    all.iter()
        .filter(|t| !selected.iter().any(|s| s == &t.path))
        .filter(|t| q.is_empty() || t.path.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// Returns `true` if any tag in `all` has a path that exactly matches the
/// trimmed `query` (case-sensitive).
///
/// # Arguments
///
/// * `all` - The full list of known tags.
/// * `query` - The text to compare against tag paths.
///
/// # Returns
///
/// `true` when an exact match is found, `false` otherwise.
#[must_use]
pub fn exact_path_exists(all: &[TagInfo], query: &str) -> bool {
    let q = query.trim();
    all.iter().any(|t| t.path == q)
}

#[cfg(test)]
mod tests {
    use bc_ipc::TagInfo;
    use pretty_assertions::assert_eq;

    use super::exact_path_exists;
    use super::filter_tags;

    fn tags() -> Vec<TagInfo> {
        vec![
            TagInfo::new("t1", "person:alice"),
            TagInfo::new("t2", "person:bob"),
            TagInfo::new("t3", "category:food"),
        ]
    }

    #[test]
    fn empty_query_returns_all_unselected() {
        let result = filter_tags(&tags(), "", &[]);
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn empty_query_excludes_selected() {
        let selected = vec!["person:alice".to_owned()];
        let result = filter_tags(&tags(), "", &selected);
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|t| t.path != "person:alice"));
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "asserted len == 1 immediately above"
    )]
    fn substring_is_case_insensitive() {
        let result = filter_tags(&tags(), "BOB", &[]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, "t2");
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "asserted len == 1 immediately above"
    )]
    fn selected_paths_excluded() {
        let selected = vec!["person:alice".to_owned(), "category:food".to_owned()];
        let result = filter_tags(&tags(), "person", &selected);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].path, "person:bob");
    }

    #[test]
    fn no_match_returns_empty() {
        let result = filter_tags(&tags(), "zzz", &[]);
        assert!(result.is_empty());
    }

    #[test]
    fn exact_path_exists_true() {
        assert!(exact_path_exists(&tags(), "person:alice"));
    }

    #[test]
    fn exact_path_exists_false() {
        assert!(!exact_path_exists(&tags(), "person:charlie"));
    }

    #[test]
    fn exact_path_exists_case_sensitive() {
        assert!(!exact_path_exists(&tags(), "Person:Alice"));
    }

    #[test]
    fn exact_path_exists_trims_query() {
        assert!(exact_path_exists(&tags(), "  person:alice  "));
    }
}
