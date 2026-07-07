//! Structured transaction search: query types, per-leg match attribution, and
//! the `Service::search` query surface.

use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::TagId;
use bc_models::Transaction;
use jiff::civil::Date;
use rust_decimal::Decimal;

/// Magnitude predicate for the amount dimension (parsed from `bc_ipc::AmountFilter`).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct AmountQuery {
    /// Inclusive lower bound on the magnitude.
    pub min: Option<Decimal>,
    /// Inclusive upper bound on the magnitude.
    pub max: Option<Decimal>,
    /// Restrict to a single commodity when set.
    pub commodity: Option<CommodityCode>,
}

/// A parsed transaction query: `bc_ipc::Filter` with ids resolved to domain types.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct TransactionQuery {
    /// Inclusive lower date bound.
    pub date_from: Option<Date>,
    /// Exclusive upper date bound.
    pub date_until: Option<Date>,
    /// Account ids; each matches its subtree; multiple union.
    pub accounts: Vec<AccountId>,
    /// Tag ids; multiple union.
    pub tags: Vec<TagId>,
    /// Case-insensitive substring over payee OR narration.
    pub text: Option<String>,
    /// Magnitude predicate.
    pub amount: Option<AmountQuery>,
    /// Exact reconciliation status.
    pub reconciliation: Option<Reconciliation>,
}

impl AmountQuery {
    /// Returns whether `amount`'s magnitude falls in `[min, max]` and, if a
    /// commodity is set, matches it. An elided (`None`) amount never matches.
    #[must_use]
    #[expect(
        clippy::shadow_reuse,
        reason = "narrowing Option<&Amount> to &Amount under the same name reads clearer than a fresh name"
    )]
    pub fn matches(&self, amount: Option<&Amount>) -> bool {
        let Some(amount) = amount else { return false };
        if let Some(c) = &self.commodity
            && amount.commodity() != c
        {
            return false;
        }
        let magnitude = amount.value().abs();
        if let Some(min) = self.min
            && magnitude < min
        {
            return false;
        }
        if let Some(max) = self.max
            && magnitude > max
        {
            return false;
        }
        true
    }
}

/// Returns the set of posting ids that match the active posting-scoped
/// predicates. An empty set means the transaction is excluded.
///
/// A leg matches iff, for every active posting-scoped dimension, it satisfies
/// that dimension — where the tag dimension is satisfied when a filter tag hits
/// at the transaction level (all legs) or on that leg. With no posting-scoped
/// dimension active, all legs match.
///
/// # Arguments
///
/// * `tx` - The hydrated transaction (all legs present).
/// * `accounts` - The resolved account-subtree id set, or `None` if inactive.
/// * `amount` - The amount predicate, or `None` if inactive.
/// * `tags` - The selected tag id set, or `None` if inactive.
#[must_use]
#[expect(
    clippy::implicit_hasher,
    reason = "callers always use the default std HashSet hasher"
)]
pub fn compute_matched_postings(
    tx: &Transaction,
    accounts: Option<&HashSet<AccountId>>,
    amount: Option<&AmountQuery>,
    tags: Option<&HashSet<TagId>>,
) -> HashSet<PostingId> {
    if accounts.is_none() && amount.is_none() && tags.is_none() {
        return tx.postings().iter().map(|p| p.id().clone()).collect();
    }

    let tx_level_tag_hit = tags.is_some_and(|set| tx.tag_ids().iter().any(|t| set.contains(t)));

    tx.postings()
        .iter()
        .filter(|p| leg_matches(p, accounts, amount, tags, tx_level_tag_hit))
        .map(|p| p.id().clone())
        .collect()
}

/// Whether a single posting satisfies every active posting-scoped dimension.
fn leg_matches(
    posting: &Posting,
    accounts: Option<&HashSet<AccountId>>,
    amount: Option<&AmountQuery>,
    tags: Option<&HashSet<TagId>>,
    tx_level_tag_hit: bool,
) -> bool {
    if let Some(set) = accounts
        && !set.contains(posting.account_id())
    {
        return false;
    }
    if let Some(q) = amount
        && !q.matches(posting.amount())
    {
        return false;
    }
    if let Some(set) = tags {
        let posting_hit = posting.tag_ids().iter().any(|t| set.contains(t));
        if !tx_level_tag_hit && !posting_hit {
            return false;
        }
    }
    true
}

/// A matched transaction plus the ids of the legs that satisfied the
/// posting-scoped predicates (all legs when the match was transaction-scoped).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct MatchedTransaction {
    /// The whole matched transaction (never pruned).
    pub transaction: Transaction,
    /// The ids of the legs that matched the posting-scoped predicates.
    pub matched_postings: HashSet<PostingId>,
}

#[cfg(test)]
mod match_tests {
    use std::collections::HashSet;

    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::TagId;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::AmountQuery;
    use super::compute_matched_postings;

    fn posting(acc: &AccountId, value: rust_decimal::Decimal, tags: Vec<TagId>) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(acc.clone())
            .amount(Amount::new(value, CommodityCode::new("AUD")))
            .tag_ids(tags)
            .build()
    }

    fn tx(postings: Vec<Posting>, tx_tags: Vec<TagId>) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 6, 1))
            .description("Test")
            .postings(postings)
            .reconciliation(Reconciliation::Unreconciled)
            .tag_ids(tx_tags)
            .created_at(Timestamp::now())
            .build()
    }

    #[test]
    fn no_posting_scoped_dims_matches_all_legs() {
        let a = AccountId::new();
        let b = AccountId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![],
        );
        let matched = compute_matched_postings(&t, None, None, None);
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn account_dim_prunes_to_matching_leg() {
        let a = AccountId::new();
        let b = AccountId::new();
        let leg_a = posting(&a, dec!(100), vec![]);
        let leg_b = posting(&b, dec!(-100), vec![]);
        let want = leg_a.id().clone();
        let t = tx(vec![leg_a, leg_b], vec![]);
        let mut set = HashSet::new();
        set.insert(a.clone());
        let matched = compute_matched_postings(&t, Some(&set), None, None);
        assert_eq!(matched.into_iter().collect::<Vec<_>>(), vec![want]);
    }

    #[test]
    fn amount_dim_matches_by_magnitude_ignoring_sign() {
        let a = AccountId::new();
        let b = AccountId::new();
        // −100 leg should match a [50, 150] magnitude window.
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![],
        );
        let q = AmountQuery {
            min: Some(dec!(50)),
            max: Some(dec!(150)),
            commodity: None,
        };
        let matched = compute_matched_postings(&t, None, Some(&q), None);
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn tx_level_tag_matches_whole_transaction() {
        let a = AccountId::new();
        let b = AccountId::new();
        let tag = TagId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![tag.clone()],
        );
        let mut set = HashSet::new();
        set.insert(tag);
        let matched = compute_matched_postings(&t, None, None, Some(&set));
        assert_eq!(matched.len(), 2);
    }

    #[test]
    fn posting_level_tag_prunes_to_tagged_leg() {
        let a = AccountId::new();
        let b = AccountId::new();
        let tag = TagId::new();
        let tagged = posting(&a, dec!(100), vec![tag.clone()]);
        let want = tagged.id().clone();
        let t = tx(vec![tagged, posting(&b, dec!(-100), vec![])], vec![]);
        let mut set = HashSet::new();
        set.insert(tag);
        let matched = compute_matched_postings(&t, None, None, Some(&set));
        assert_eq!(matched.into_iter().collect::<Vec<_>>(), vec![want]);
    }

    #[test]
    fn conjunction_excludes_when_no_leg_satisfies_all() {
        // Tag hits at tx level (all legs) but the account dim matches no leg → empty.
        let a = AccountId::new();
        let b = AccountId::new();
        let other = AccountId::new();
        let tag = TagId::new();
        let t = tx(
            vec![
                posting(&a, dec!(100), vec![]),
                posting(&b, dec!(-100), vec![]),
            ],
            vec![tag.clone()],
        );
        let mut accounts = HashSet::new();
        accounts.insert(other);
        let mut tags = HashSet::new();
        tags.insert(tag);
        let matched = compute_matched_postings(&t, Some(&accounts), None, Some(&tags));
        assert!(matched.is_empty());
    }
}

#[cfg(test)]
#[cfg(feature = "ipc")]
mod tests {
    use bc_models::AccountId;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

    use super::TransactionQuery;

    #[test]
    fn try_from_filter_parses_ids_and_scalars() {
        let acc = AccountId::new();
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec![acc.to_string()];
        filter.text = Some("coffee".to_owned());
        let mut amount_filter = bc_ipc::AmountFilter::default();
        amount_filter.min = Some(Decimal::new(5, 0));
        amount_filter.commodity = Some("AUD".to_owned());
        filter.amount = Some(amount_filter);

        let query = TransactionQuery::try_from(filter).expect("valid filter");
        assert_eq!(query.accounts, vec![acc]);
        assert_eq!(query.text.as_deref(), Some("coffee"));
        let amount = query.amount.expect("amount present");
        assert_eq!(amount.min, Some(Decimal::new(5, 0)));
        assert_eq!(
            amount.commodity.map(|c| c.as_str().to_owned()),
            Some("AUD".to_owned())
        );
    }

    #[test]
    fn try_from_filter_rejects_bad_account_id() {
        let mut filter = bc_ipc::Filter::default();
        filter.accounts = vec!["not-a-valid-id!!".to_owned()];
        let err = TransactionQuery::try_from(filter).expect_err("invalid account id must fail");
        assert!(matches!(err, crate::BcError::BadData(_)));
    }
}
