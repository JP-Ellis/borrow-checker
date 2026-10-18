//! Short, unique handles for transactions in a working export.

use std::collections::HashMap;

use bc_models::TransactionId;

/// The fewest characters a handle ever has.
const MIN_LEN: usize = 3;

/// Assigns each transaction the shortest handle no other ID in `ids` shares.
///
/// A handle is a suffix of the ID's `TypeID` suffix. The leading characters of
/// a `UUIDv7` encode its timestamp and are shared by IDs created together, so
/// handles come from the random tail. Each handle has at least three
/// characters.
///
/// # Arguments
///
/// * `ids` - The IDs to abbreviate. Duplicates collapse to one entry.
///
/// # Returns
///
/// A map from each ID to its handle.
#[must_use]
pub fn abbreviate(ids: &[TransactionId]) -> HashMap<TransactionId, String> {
    let mut keyed: Vec<(String, &TransactionId)> =
        ids.iter().map(|id| (reversed_suffix(id), id)).collect();
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    keyed.dedup_by(|a, b| a.1 == b.1);

    let mut handles = HashMap::with_capacity(keyed.len());
    let mut previous: Option<&str> = None;
    let mut rest = keyed.iter().peekable();
    while let Some((reversed, id)) = rest.next() {
        let shared_before = previous.map_or(0, |p| common_prefix_len(reversed, p));
        let shared_after = rest
            .peek()
            .map_or(0, |(next, _)| common_prefix_len(reversed, next));
        let len = shared_before
            .max(shared_after)
            .saturating_add(1)
            .max(MIN_LEN)
            .min(reversed.len());
        let handle: String = reversed
            .chars()
            .take(len)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        handles.insert((*id).clone(), handle);
        previous = Some(reversed);
    }
    handles
}

/// Returns the `TypeID` suffix of `id`, reversed.
fn reversed_suffix(id: &TransactionId) -> String {
    let text = id.to_string();
    let suffix = text.split_once('_').map_or(text.as_str(), |(_, s)| s);
    suffix.chars().rev().collect()
}

/// Counts the leading characters two strings share.
fn common_prefix_len(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashSet;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn id(suffix: &str) -> TransactionId {
        format!("transaction_{suffix}")
            .parse()
            .expect("a 26-character base32 suffix parses")
    }

    #[test]
    fn one_id_gets_three_characters() {
        let a = id("01h455vb4pex5vsknk084sn02q");
        let handles = abbreviate(std::slice::from_ref(&a));
        assert_eq!(handles.get(&a).map(String::as_str), Some("02q"));
    }

    #[rstest]
    #[case::one_id_shares_nothing(&["01h455vb4pex5vsknk084sn02q"], 3)]
    #[case::two_share_two(
        &["01h455vb4pex5vsknk084sna2q", "01h455vb4pex5vsknk084snb2q"],
        3
    )]
    #[case::two_share_four(
        &["01h455vb4pex5vsknk084xs02q", "01h455vb4pex5vsknk084ys02q"],
        5
    )]
    fn handle_length_follows_the_shared_tail(#[case] suffixes: &[&str], #[case] expected: usize) {
        let ids: Vec<TransactionId> = suffixes.iter().map(|s| id(s)).collect();
        let handles = abbreviate(&ids);
        assert_eq!(handles.len(), ids.len());
        assert!(handles.values().all(|h| h.len() == expected));
        let unique: HashSet<&String> = handles.values().collect();
        assert_eq!(unique.len(), ids.len());
    }

    #[test]
    fn a_batch_of_fresh_ids_is_unique() {
        let ids: Vec<TransactionId> = std::iter::repeat_with(TransactionId::new)
            .take(1000)
            .collect();
        let handles = abbreviate(&ids);
        assert_eq!(handles.len(), 1000);
        assert!(handles.values().all(|h| h.len() >= 3));
        let unique: HashSet<&String> = handles.values().collect();
        assert_eq!(unique.len(), 1000);
    }
}
