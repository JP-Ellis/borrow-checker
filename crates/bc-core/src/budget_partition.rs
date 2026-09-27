//! Assigns each posting under the budget tree to exactly one bucket.
//!
//! A budget counts every posting in its scope (inclusive, as Fava does). For
//! display, each posting also has an owner: the most specific budget that
//! matches it. Envelopes show what they own as *unallocated*.

use std::collections::HashMap;

use crate::budget::PostingKey;

/// The set of postings a budget can match, as account and tag chains.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Scope {
    /// Account ids from the type root down to the budget's account.
    pub account_chain: Vec<bc_models::AccountId>,
    /// Tag ids from the tag root down to the filter tag; `None` = unfiltered.
    pub tag_chain: Option<Vec<bc_models::TagId>>,
}

impl Scope {
    /// `true` when every posting `other` can match, `self` can match too.
    pub(crate) fn contains(&self, other: &Self) -> bool {
        other.account_chain.starts_with(&self.account_chain)
            && match (&self.tag_chain, &other.tag_chain) {
                (None, _) => true,
                (Some(mine), Some(theirs)) => theirs.starts_with(mine),
                (Some(_), None) => false,
            }
    }
}

/// `true` when `b` is a strictly narrower budget inside `a`.
pub(crate) fn more_specific(b: &Scope, a: &Scope) -> bool {
    a != b && a.contains(b)
}

/// For each scope, the index of the innermost other scope strictly containing it.
pub(crate) fn envelope_parents(scopes: &[Scope]) -> Vec<Option<usize>> {
    scopes
        .iter()
        .map(|s| {
            scopes
                .iter()
                .enumerate()
                .filter(|&(_, p)| more_specific(s, p))
                .min_by_key(|&(i, p)| {
                    (
                        core::cmp::Reverse(p.account_chain.len()),
                        core::cmp::Reverse(p.tag_chain.as_ref().map_or(0, Vec::len)),
                        i,
                    )
                })
                .map(|(i, _)| i)
        })
        .collect()
}

/// Who a posting belongs to for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// Exactly one most specific budget matches.
    Budget(usize),
    /// Several matching budgets are incomparable; the posting counts in each.
    Shared(Vec<usize>),
}

/// Resolves each posting's owner from the budgets that matched it.
pub(crate) fn owners(
    scopes: &[Scope],
    matches: &HashMap<PostingKey, Vec<usize>>,
) -> HashMap<PostingKey, Owner> {
    matches
        .iter()
        .filter_map(|(key, matched)| {
            let mut minimal: Vec<usize> = matched
                .iter()
                .copied()
                .filter(|&i| {
                    !matched.iter().any(|&j| {
                        scopes
                            .get(j)
                            .zip(scopes.get(i))
                            .is_some_and(|(sj, si)| more_specific(sj, si))
                    })
                })
                .collect();
            minimal.sort_unstable();
            minimal.dedup();
            let owner = match minimal.as_slice() {
                [] => return None,
                [only] => Owner::Budget(*only),
                _ => Owner::Shared(minimal),
            };
            Some((key.clone(), owner))
        })
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;

    use bc_models::AccountId;
    use bc_models::TagId;
    use pretty_assertions::assert_eq;

    use super::*;

    fn ids<const N: usize>() -> [AccountId; N] {
        core::array::from_fn(|_| AccountId::new())
    }

    fn key(n: &str) -> PostingKey {
        PostingKey {
            posting_id: n.to_owned(),
            commodity: "AUD".to_owned(),
        }
    }

    #[test]
    fn containment_follows_accounts_and_tags() {
        let [exp, food, groc] = ids::<3>();
        let household = TagId::new();
        let food_all = Scope {
            account_chain: vec![exp.clone(), food.clone()],
            tag_chain: None,
        };
        let food_hh = Scope {
            account_chain: vec![exp.clone(), food.clone()],
            tag_chain: Some(vec![household.clone()]),
        };
        let groc_all = Scope {
            account_chain: vec![exp.clone(), food.clone(), groc.clone()],
            tag_chain: None,
        };
        let groc_hh = Scope {
            account_chain: vec![exp, food, groc],
            tag_chain: Some(vec![household]),
        };
        assert!(more_specific(&groc_all, &food_all));
        assert!(more_specific(&food_hh, &food_all));
        assert!(more_specific(&groc_hh, &food_hh));
        assert!(
            !more_specific(&groc_all, &food_hh),
            "overlap is not containment"
        );
        assert!(!more_specific(&food_all, &food_all));
    }

    #[test]
    fn envelope_parent_is_innermost() {
        let [exp, food, groc] = ids::<3>();
        let scopes = vec![
            Scope {
                account_chain: vec![exp.clone()],
                tag_chain: None,
            },
            Scope {
                account_chain: vec![exp.clone(), food.clone()],
                tag_chain: None,
            },
            Scope {
                account_chain: vec![exp, food, groc],
                tag_chain: None,
            },
        ];
        assert_eq!(envelope_parents(&scopes), vec![None, Some(0), Some(1)]);
    }

    #[test]
    fn owner_is_most_specific_and_siblings_share() {
        let [haircuts] = ids::<1>();
        let (a, b) = (TagId::new(), TagId::new());
        let scopes = vec![
            Scope {
                account_chain: vec![haircuts.clone()],
                tag_chain: None,
            },
            Scope {
                account_chain: vec![haircuts.clone()],
                tag_chain: Some(vec![a]),
            },
            Scope {
                account_chain: vec![haircuts],
                tag_chain: Some(vec![b]),
            },
        ];
        let matches = HashMap::from([
            (key("only-envelope"), vec![0]),
            (key("tagged-a"), vec![0, 1]),
            (key("tagged-both"), vec![0, 1, 2]),
        ]);
        let got = owners(&scopes, &matches);
        assert_eq!(got.get(&key("only-envelope")), Some(&Owner::Budget(0)));
        assert_eq!(got.get(&key("tagged-a")), Some(&Owner::Budget(1)));
        assert_eq!(
            got.get(&key("tagged-both")),
            Some(&Owner::Shared(vec![1, 2]))
        );
    }

    #[test]
    fn equal_scopes_share_and_nest_under_neither() {
        // Nothing stops two unfiltered budgets on one account: `tag_filter`
        // lives on revisions, with no uniqueness constraint.
        let [widgets] = ids::<1>();
        let scope = Scope {
            account_chain: vec![widgets],
            tag_chain: None,
        };
        let scopes = vec![scope.clone(), scope];
        let matches = HashMap::from([(key("both"), vec![0, 1])]);
        assert_eq!(
            owners(&scopes, &matches).get(&key("both")),
            Some(&Owner::Shared(vec![0, 1]))
        );
        assert_eq!(envelope_parents(&scopes), vec![None, None]);
    }
}
