//! Resolves an account or tag path, or a trailing run of its segments.

use crate::catalog::PathEntry;

/// How a path resolved.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Match<'c> {
    /// Exactly one entry.
    One(&'c PathEntry),
    /// No entry.
    Missing,
    /// Several entries end with the segments.
    Many(Vec<&'c PathEntry>),
}

/// Splits a colon-separated path; `None` when a segment is empty.
pub(crate) fn split(text: &str) -> Option<Vec<&str>> {
    let segments: Vec<&str> = text.split(':').collect();
    segments.iter().all(|s| !s.is_empty()).then_some(segments)
}

/// Resolves `segments` against `entries`, comparing case-insensitively.
///
/// A full-path match wins. Otherwise the segments must be the trailing run
/// of exactly one entry's path.
pub(crate) fn resolve<'c>(entries: &'c [PathEntry], segments: &[&str]) -> Match<'c> {
    let exact: Vec<&PathEntry> = entries
        .iter()
        .filter(|e| e.path.len() == segments.len() && ends_with(&e.path, segments))
        .collect();
    if let [one] = exact.as_slice() {
        return Match::One(one);
    }
    let endings: Vec<&PathEntry> = entries
        .iter()
        .filter(|e| ends_with(&e.path, segments))
        .collect();
    match endings.as_slice() {
        [] => Match::Missing,
        [one] => Match::One(one),
        _ => Match::Many(endings),
    }
}

/// Whether `path` ends with `segments`, case-insensitively.
fn ends_with(path: &[String], segments: &[&str]) -> bool {
    path.len() >= segments.len()
        && path
            .iter()
            .rev()
            .zip(segments.iter().rev())
            .all(|(p, s)| p.eq_ignore_ascii_case(s))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    /// A small invented account tree.
    fn accounts() -> Vec<PathEntry> {
        vec![
            PathEntry::new("a0", ["Assets"]),
            PathEntry::new("a1", ["Expenses", "Food"]),
            PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
            PathEntry::new("a3", ["Income", "Food"]),
            PathEntry::new("a4", ["Liabilities", "Assets"]),
        ]
    }

    /// The ids a match names.
    fn ids(m: &Match<'_>) -> Vec<String> {
        match m {
            Match::One(e) => vec![e.id.clone()],
            Match::Missing => vec![],
            Match::Many(es) => es.iter().map(|e| e.id.clone()).collect(),
        }
    }

    #[rstest]
    #[case("Groceries", &["a2"])]
    #[case("food:groceries", &["a2"])]
    #[case("Expenses:Food", &["a1"])]
    #[case("Food", &["a1", "a3"])]
    #[case("Assets", &["a0"])]
    #[case("Nowhere", &[])]
    fn resolves(#[case] text: &str, #[case] expected: &[&str]) {
        let entries = accounts();
        let segments = split(text).expect("non-empty segments");
        assert_eq!(ids(&resolve(&entries, &segments)), expected);
    }

    #[rstest]
    #[case("Expenses::Food")]
    #[case(":Food")]
    #[case("Food:")]
    fn split_rejects_empty_segments(#[case] text: &str) {
        assert_eq!(split(text), None);
    }
}
