// Leptos-free label helpers for the budget row.
//
// Kept separate from `mod.rs` so the pure logic runs under a native
// `cargo nextest` (`mod.rs` sits under a wasm-only module tree and never
// runs there); see `components_tests` in `main.rs`.

/// The `#`-prefixed chip text for a row whose label is only a tag, else
/// `None`.
///
/// Core labels a filtered budget on its parent row's account by its tag
/// path relative to the parent's tag, with no account part. A revision name
/// replaces that label, so the label must also end the tag path at a segment
/// boundary.
///
/// # Arguments
///
/// * `label` - The row's label.
/// * `tag_filter` - The row's full tag path, if filtered.
/// * `account_id` - The row's account.
/// * `parent_account_id` - The parent row's account; `None` for a type root.
#[must_use]
pub(crate) fn tag_chip(
    label: &str,
    tag_filter: Option<&str>,
    account_id: &str,
    parent_account_id: Option<&str>,
) -> Option<String> {
    let tag = tag_filter?;
    let same_account = parent_account_id == Some(account_id);
    let ends_tag = tag
        .strip_suffix(label)
        .is_some_and(|rest| rest.is_empty() || rest.ends_with(':'));
    (same_account && ends_tag).then(|| format!("#{label}"))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::whole_path("person:a", Some("person:a"), Some("acct"), Some("#person:a"))]
    #[case::relative_to_parent_tag("a", Some("person:a"), Some("acct"), Some("#a"))]
    #[case::account_and_tag("Dining #household", Some("household"), Some("acct"), None)]
    #[case::revision_name("Treats", Some("household"), Some("acct"), None)]
    #[case::partial_segment("a", Some("person:anna"), Some("acct"), None)]
    #[case::other_account("household", Some("household"), Some("food"), None)]
    #[case::type_root("household", Some("household"), None, None)]
    #[case::unfiltered("Groceries", None, Some("acct"), None)]
    fn tag_only_labels_become_chips(
        #[case] label: &str,
        #[case] tag_filter: Option<&str>,
        #[case] parent_account_id: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            tag_chip(label, tag_filter, "acct", parent_account_id),
            expected.map(ToOwned::to_owned)
        );
    }
}
