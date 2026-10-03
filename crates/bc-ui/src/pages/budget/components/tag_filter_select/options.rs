use bc_ipc::TagInfo;

/// Builds the options a tag-filter select offers.
///
/// Every catalogue tag appears once, in catalogue order. A `current`
/// selection missing from the catalogue (not yet loaded, failed to load or
/// deleted) is appended so the select still shows it and a save keeps it.
/// Tags are matched by ID.
///
/// # Arguments
///
/// * `catalogue` - Every known tag.
/// * `current` - The selected tag, if any.
///
/// # Returns
///
/// The tags to render as options, excluding the "none" option.
#[must_use]
pub fn options(catalogue: &[TagInfo], current: Option<&TagInfo>) -> Vec<TagInfo> {
    let mut out = catalogue.to_vec();
    if let Some(cur) = current
        && !out.iter().any(|t| t.id == cur.id)
    {
        out.push(cur.clone());
    }
    out
}

/// Maps a select's chosen value back to a tag in `options`.
///
/// # Arguments
///
/// * `options` - The tags the select offers.
/// * `id` - The chosen option value; `""` is the "none" option.
///
/// # Returns
///
/// The tag with that ID, or `None` for `""` or an unknown ID.
#[must_use]
pub fn resolve(options: &[TagInfo], id: &str) -> Option<TagInfo> {
    if id.is_empty() {
        return None;
    }
    options.iter().find(|t| t.id == id).cloned()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::TagInfo;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::options;
    use super::resolve;

    fn catalogue() -> Vec<TagInfo> {
        vec![
            TagInfo::new("t1", "person:alice"),
            TagInfo::new("t2", "person:bob"),
        ]
    }

    #[test]
    fn empty_value_resolves_to_none() {
        assert_eq!(resolve(&catalogue(), ""), None);
    }

    #[test]
    fn known_id_resolves_to_its_tag() {
        assert_eq!(
            resolve(&catalogue(), "t2"),
            Some(TagInfo::new("t2", "person:bob"))
        );
    }

    #[test]
    fn unknown_id_resolves_to_none() {
        assert_eq!(resolve(&catalogue(), "zz"), None);
    }

    #[rstest]
    #[case::empty_catalogue(vec![])]
    #[case::catalogue_lacks_it(catalogue())]
    fn current_is_kept_when_the_catalogue_lacks_it(#[case] cat: Vec<TagInfo>) {
        let cur = TagInfo::new("gone", "old:tag");
        let out = options(&cat, Some(&cur));
        assert_eq!(out.last(), Some(&cur));
        assert_eq!(out.get(..cat.len()), Some(cat.as_slice()));
    }

    #[test]
    fn current_in_the_catalogue_is_not_duplicated() {
        // The selection holds the pre-rename path; the catalogue's entry wins.
        let cur = TagInfo::new("t1", "person:alice-old");
        let out = options(&catalogue(), Some(&cur));
        assert_eq!(out, catalogue());
    }

    #[test]
    fn no_current_returns_the_catalogue() {
        assert_eq!(options(&catalogue(), None), catalogue());
    }
}
