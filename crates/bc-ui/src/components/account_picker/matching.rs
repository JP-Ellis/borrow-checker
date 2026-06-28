use bc_ipc::AccountRef;

/// Filters `accounts` to those whose display name contains `query`.
///
/// Matching is case-insensitive substring on the full account name. An empty
/// query returns every account. Input order is preserved.
///
/// # Arguments
///
/// * `accounts` - The candidate accounts.
/// * `query` - The user's search text.
///
/// # Returns
///
/// The matching accounts, in input order.
#[must_use]
pub fn filter_accounts(accounts: &[AccountRef], query: &str) -> Vec<AccountRef> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return accounts.to_vec();
    }
    accounts
        .iter()
        .filter(|a| a.name.to_lowercase().contains(&q))
        .cloned()
        .collect()
}

/// A run of an account name, flagged if it is part of a query match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    /// The substring of the account name.
    pub text: String,
    /// Whether this run is the highlighted query match.
    pub hit: bool,
}

/// Splits `name` into highlighted / plain runs around the first
/// case-insensitive occurrence of `query`.
///
/// An empty or whitespace-only `query` returns a single plain run.
/// If `query` is not found, returns a single plain run of the full name.
///
/// # Arguments
///
/// * `name` - The account name to segment.
/// * `query` - The user's search text.
///
/// # Returns
///
/// One to three [`Seg`] values covering the full name.
#[expect(
    clippy::string_slice,
    reason = "slice offsets are mapped back to original char boundaries before slicing"
)]
#[must_use]
pub fn match_segments(name: &str, query: &str) -> Vec<Seg> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return vec![Seg {
            text: name.to_owned(),
            hit: false,
        }];
    }
    let lower = name.to_lowercase();
    let Some(low_start) = lower.find(&q) else {
        return vec![Seg {
            text: name.to_owned(),
            hit: false,
        }];
    };
    let low_end = low_start.saturating_add(q.len());

    // `to_lowercase` maps each char independently, so the lowercased string is
    // the concatenation of each original char's lowercase expansion. Walk the
    // original alongside the lowercased cursor to map lowercased byte offsets
    // back to original char boundaries: a non-ASCII char may change byte length
    // (e.g. `İ` → `i̇`), so the raw `find` offsets need not be valid here.
    let mut boundaries: Vec<(usize, usize)> = Vec::new();
    let mut low_pos = 0_usize;
    for (orig_pos, ch) in name.char_indices() {
        boundaries.push((low_pos, orig_pos));
        for lc in ch.to_lowercase() {
            low_pos = low_pos.saturating_add(lc.len_utf8());
        }
    }
    boundaries.push((low_pos, name.len()));

    // Snap the start down and the end up to enclosing char boundaries so the
    // slices below are always valid even if a match straddles an expansion.
    let start = boundaries
        .iter()
        .rev()
        .find(|(lp, _)| *lp <= low_start)
        .map_or(0, |(_, op)| *op);
    let end = boundaries
        .iter()
        .find(|(lp, _)| *lp >= low_end)
        .map_or(name.len(), |(_, op)| *op);

    let mut out = Vec::new();
    if start > 0 {
        out.push(Seg {
            text: name[..start].to_owned(),
            hit: false,
        });
    }
    out.push(Seg {
        text: name[start..end].to_owned(),
        hit: true,
    });
    if end < name.len() {
        out.push(Seg {
            text: name[end..].to_owned(),
            hit: false,
        });
    }
    out
}

/// Splits an account name into `(prefix_including_separator, leaf)`.
///
/// Handles both the `" :: "` display separator and the `":"` path separator.
/// If no separator is found, the prefix is empty and the leaf is the full name.
///
/// # Arguments
///
/// * `name` - The account name to split.
///
/// # Returns
///
/// A tuple `(prefix, leaf)` where `prefix` includes the trailing separator.
#[expect(
    clippy::string_slice,
    reason = "rfind returns offsets on the ASCII separators, which are valid char boundaries"
)]
#[expect(
    clippy::arithmetic_side_effects,
    reason = "offsets just past the ASCII separators stay valid char boundaries bounded by name.len()"
)]
#[must_use]
pub fn split_leaf(name: &str) -> (String, String) {
    if let Some(i) = name.rfind(" :: ") {
        let cut = i + " :: ".len();
        return (name[..cut].to_owned(), name[cut..].to_owned());
    }
    if let Some(i) = name.rfind(':') {
        return (name[..=i].to_owned(), name[i + 1..].to_owned());
    }
    (String::new(), name.to_owned())
}

#[cfg(test)]
mod tests {
    use bc_ipc::AccountRef;
    use pretty_assertions::assert_eq;

    use super::Seg;
    use super::filter_accounts;
    use super::match_segments;
    use super::split_leaf;

    fn accts() -> Vec<AccountRef> {
        vec![
            AccountRef::new("a1", "Assets :: Checking"),
            AccountRef::new("a2", "Assets :: Savings"),
            AccountRef::new("e1", "Expenses :: Groceries"),
        ]
    }

    #[test]
    fn empty_query_returns_all() {
        assert_eq!(filter_accounts(&accts(), "").len(), 3);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "asserted len == 1 immediately above"
    )]
    fn substring_is_case_insensitive() {
        let r = filter_accounts(&accts(), "savings");
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].id, "a2");
    }

    #[test]
    fn matches_across_segments() {
        let r = filter_accounts(&accts(), "assets");
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn no_match_is_empty() {
        assert!(filter_accounts(&accts(), "zzz").is_empty());
    }

    #[test]
    fn segments_highlight_first_match() {
        let segs = match_segments("Expenses:Food:Groceries", "gro");
        assert_eq!(
            segs,
            vec![
                Seg {
                    text: "Expenses:Food:".into(),
                    hit: false
                },
                Seg {
                    text: "Gro".into(),
                    hit: true
                },
                Seg {
                    text: "ceries".into(),
                    hit: false
                },
            ]
        );
    }

    #[test]
    fn segments_match_at_start_has_no_prefix_run() {
        let segs = match_segments("Groceries", "gro");
        assert_eq!(
            segs,
            vec![
                Seg {
                    text: "Gro".into(),
                    hit: true
                },
                Seg {
                    text: "ceries".into(),
                    hit: false
                },
            ]
        );
    }

    #[test]
    fn segments_match_at_end_has_no_suffix_run() {
        let segs = match_segments("Food:Gro", "gro");
        assert_eq!(
            segs,
            vec![
                Seg {
                    text: "Food:".into(),
                    hit: false
                },
                Seg {
                    text: "Gro".into(),
                    hit: true
                },
            ]
        );
    }

    #[test]
    fn segments_no_match_is_single_plain_run() {
        let segs = match_segments("Assets:Cash", "zzz");
        assert_eq!(
            segs,
            vec![Seg {
                text: "Assets:Cash".into(),
                hit: false
            }]
        );
    }

    #[test]
    fn segments_non_ascii_length_changing_does_not_panic() {
        // `İ` (U+0130) lowercases to two chars (`i` + combining dot), so the
        // lowercased match offset does not line up with the original bytes.
        let segs = match_segments("İstanbul", "stan");
        assert_eq!(
            segs,
            vec![
                Seg {
                    text: "İ".into(),
                    hit: false
                },
                Seg {
                    text: "stan".into(),
                    hit: true
                },
                Seg {
                    text: "bul".into(),
                    hit: false
                },
            ]
        );
    }

    #[test]
    fn segments_non_ascii_multibyte_leaf() {
        // `ß` is a single lowercase char but two bytes; the leaf match must land
        // on the original char boundary, not a shifted byte offset.
        let segs = match_segments("Großmann", "mann");
        assert_eq!(
            segs,
            vec![
                Seg {
                    text: "Groß".into(),
                    hit: false
                },
                Seg {
                    text: "mann".into(),
                    hit: true
                },
            ]
        );
    }

    #[test]
    fn segments_accented_match_is_highlighted() {
        let segs = match_segments("Café :: Lait", "café");
        assert_eq!(
            segs,
            vec![
                Seg {
                    text: "Café".into(),
                    hit: true
                },
                Seg {
                    text: " :: Lait".into(),
                    hit: false
                },
            ]
        );
    }

    #[test]
    fn segments_empty_query_is_single_plain_run() {
        let segs = match_segments("Assets:Cash", "");
        assert_eq!(
            segs,
            vec![Seg {
                text: "Assets:Cash".into(),
                hit: false
            }]
        );
    }

    #[test]
    fn split_leaf_colon() {
        assert_eq!(
            split_leaf("Expenses:Food:Groceries"),
            ("Expenses:Food:".into(), "Groceries".into())
        );
    }

    #[test]
    fn split_leaf_spaced() {
        assert_eq!(
            split_leaf("Assets :: Checking"),
            ("Assets :: ".into(), "Checking".into())
        );
    }

    #[test]
    fn split_leaf_no_separator() {
        assert_eq!(split_leaf("Cash"), (String::new(), "Cash".into()));
    }
}
