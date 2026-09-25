//! Selecting a transaction and its postings, and applying posting operations
//! for `transaction edit`.

use core::str::FromStr as _;
use std::collections::HashSet;

use super::leg;
use super::spec;
use crate::error::CliError;
use crate::error::CliResult;

/// Names one posting of a transaction.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum PostingSelector {
    /// The posting with this ID.
    Id(bc_models::PostingId),
    /// The one posting on this account.
    Account(bc_models::AccountId),
    /// The one posting on this account with this amount.
    AccountAmount(bc_models::AccountId, bc_models::Amount),
}

impl PostingSelector {
    /// Whether `posting` is one this selector names.
    fn matches(&self, posting: &bc_models::Posting) -> bool {
        match self {
            Self::Id(id) => posting.id() == id,
            Self::Account(account) => posting.account_id() == account,
            Self::AccountAmount(account, want) => {
                posting.account_id() == account
                    && posting.amount().is_some_and(|have| {
                        have.value() == want.value() && have.commodity() == want.commodity()
                    })
            }
        }
    }
}

/// The posting changes one `transaction edit` makes.
#[derive(Debug, Default)]
pub(super) struct Ops {
    /// Postings to append.
    pub add: Vec<bc_models::Posting>,
    /// Postings whose account, amount, cost and price the paired spec replaces.
    pub set: Vec<(bc_models::PostingId, bc_models::Posting)>,
    /// Postings to drop.
    pub remove: Vec<bc_models::PostingId>,
}

/// Parses `POSTING`: a posting ID, an account, or `ACCOUNT:AMOUNT:COMMODITY`.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when the text is none of the three forms.
pub(super) fn parse_selector(text: &str, lookup: &spec::Lookup<'_>) -> CliResult<PostingSelector> {
    if let Ok(id) = bc_models::PostingId::from_str(text.trim()) {
        return Ok(PostingSelector::Id(id));
    }
    if let Some(account) = lookup(text) {
        return Ok(PostingSelector::Account(account));
    }
    let (account, amount) = spec::split_account(text, lookup, |right| {
        let parsed = leg::parse_leg(right)?;
        if parsed.cost.is_some() || parsed.price.is_some() {
            return Err("a posting selector names an amount, not a cost or price".into());
        }
        Ok(spec::amount_of(parsed.units))
    })
    .map_err(|e| CliError::Arg(format!("invalid posting '{text}': {e}")))?;
    Ok(PostingSelector::AccountAmount(account, amount))
}

/// Lists a transaction's postings, each next to its ID, for an error message.
fn listing(
    tx: &bc_models::Transaction,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> String {
    tx.postings()
        .iter()
        .map(|p| format!("{} {}", p.id(), describe(p)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Finds the one posting of `tx` that `selector` names.
///
/// # Errors
///
/// Returns [`CliError::Arg`] listing the transaction's postings when none or
/// several match. `text` is the selector as the user wrote it.
pub(super) fn select_posting<'t>(
    tx: &'t bc_models::Transaction,
    selector: &PostingSelector,
    text: &str,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<&'t bc_models::PostingId> {
    let mut matched = tx.postings().iter().filter(|p| selector.matches(p));
    match (matched.next(), matched.next()) {
        (Some(posting), None) => Ok(posting.id()),
        (None, _) => Err(CliError::Arg(format!(
            "no posting matches '{text}'; the transaction holds: {}",
            listing(tx, describe)
        ))),
        (Some(_), Some(_)) => Err(CliError::Arg(format!(
            "several postings match '{text}'; pass one of the posting IDs below to pick one: {}",
            listing(tx, describe)
        ))),
    }
}

/// Whether `tx` holds a posting on exactly `account`, with `amount` when given.
pub(super) fn touches(
    tx: &bc_models::Transaction,
    account: &bc_models::AccountId,
    amount: Option<rust_decimal::Decimal>,
) -> bool {
    tx.postings().iter().any(|p| {
        p.account_id() == account
            && amount.is_none_or(|want| p.amount().is_some_and(|have| have.value() == want))
    })
}

/// Applies `ops` to `current`, keeping every posting they do not name.
///
/// A set posting keeps its ID, metadata, tags and spread, so its import
/// reference stays linked; the spec supplies account, amount, cost and price.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when one posting is named by more than one set or
/// remove.
pub(super) fn apply(
    current: &bc_models::Transaction,
    ops: &Ops,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<bc_models::Transaction> {
    let mut named: HashSet<&bc_models::PostingId> = HashSet::new();
    for id in ops.set.iter().map(|(id, _)| id).chain(&ops.remove) {
        if !named.insert(id) {
            let label = current
                .postings()
                .iter()
                .find(|p| p.id() == id)
                .map_or_else(|| id.to_string(), describe);
            return Err(CliError::Arg(format!(
                "posting '{label}' is named more than once by --set-posting and --remove-posting"
            )));
        }
    }

    let mut postings: Vec<bc_models::Posting> = current
        .postings()
        .iter()
        .filter(|p| !ops.remove.contains(p.id()))
        .map(|p| match ops.set.iter().find(|(id, _)| id == p.id()) {
            Some((_, spec)) => bc_models::Posting::builder()
                .id(p.id().clone())
                .account_id(spec.account_id().clone())
                .maybe_amount(spec.amount().cloned())
                .maybe_cost(spec.cost().cloned())
                .maybe_price(spec.price().cloned())
                .metadata(p.metadata().clone())
                .tag_ids(p.tag_ids().to_vec())
                .maybe_spread_from(p.spread_from())
                .maybe_spread_until(p.spread_until())
                .build(),
            None => p.clone(),
        })
        .collect();
    postings.extend(ops.add.iter().cloned());

    Ok(bc_models::Transaction::builder()
        .id(current.id().clone())
        .date(current.date())
        .description(current.description().to_owned())
        .metadata(current.metadata().clone())
        .postings(postings)
        .tag_ids(current.tag_ids().to_vec())
        .reconciliation(current.reconciliation())
        .created_at(*current.created_at())
        .build())
}

/// The removed postings that carry a live import reference.
///
/// `imported` holds the posting IDs, as strings, that
/// `SourceService::provenance_by_posting` returned for `current`.
pub(super) fn imported_removals<'t>(
    current: &'t bc_models::Transaction,
    ops: &Ops,
    imported: &HashSet<String>,
) -> Vec<&'t bc_models::Posting> {
    current
        .postings()
        .iter()
        .filter(|p| ops.remove.contains(p.id()) && imported.contains(&p.id().to_string()))
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashSet;

    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::Ops;
    use super::PostingSelector;
    use super::apply;
    use super::imported_removals;
    use super::parse_selector;
    use super::select_posting;
    use super::touches;

    fn aud(value: rust_decimal::Decimal) -> bc_models::Amount {
        bc_models::Amount::new(value, bc_models::CommodityCode::new("AUD"))
    }

    fn posting(account: &bc_models::AccountId, value: rust_decimal::Decimal) -> bc_models::Posting {
        bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(account.clone())
            .amount(aud(value))
            .build()
    }

    fn transaction(postings: Vec<bc_models::Posting>) -> bc_models::Transaction {
        bc_models::Transaction::builder()
            .id(bc_models::TransactionId::new())
            .date(date(2026, 3, 1))
            .description("Grocery shopping")
            .postings(postings)
            .reconciliation(bc_models::Reconciliation::Reconciled)
            .created_at(jiff::Timestamp::UNIX_EPOCH)
            .build()
    }

    fn describe(p: &bc_models::Posting) -> String {
        p.account_id().to_string()
    }

    #[test]
    fn touches_compares_amounts_numerically() {
        let checking = bc_models::AccountId::new();
        let tx = transaction(vec![posting(&checking, dec!(-50.00))]);
        assert!(touches(&tx, &checking, Some(dec!(-50))));
        assert!(touches(&tx, &checking, None));
        assert!(!touches(&tx, &checking, Some(dec!(50))));
    }

    #[test]
    fn touches_ignores_a_parent_account() {
        let parent = bc_models::AccountId::new();
        let child = bc_models::AccountId::new();
        let tx = transaction(vec![posting(&child, dec!(-50))]);
        assert!(!touches(&tx, &parent, None));
    }

    #[test]
    fn touches_skips_an_elided_posting_when_an_amount_is_given() {
        let checking = bc_models::AccountId::new();
        let elided = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(checking.clone())
            .build();
        let tx = transaction(vec![elided]);
        assert!(!touches(&tx, &checking, Some(dec!(-50))));
        assert!(touches(&tx, &checking, None));
    }

    #[test]
    fn selector_by_account_needs_an_amount_when_the_account_repeats() {
        let groceries = bc_models::AccountId::new();
        let first = posting(&groceries, dec!(30));
        let second = posting(&groceries, dec!(20));
        let first_id = first.id().clone();
        let tx = transaction(vec![first, second.clone()]);

        let err = select_posting(
            &tx,
            &PostingSelector::Account(groceries.clone()),
            "G",
            &describe,
        )
        .expect_err("ambiguous");
        let message = err.to_string();
        assert!(
            message.contains("several postings match 'G'"),
            "got: {message}"
        );
        assert!(message.contains(&first_id.to_string()), "got: {message}");
        assert!(message.contains(&second.id().to_string()), "got: {message}");

        let picked = select_posting(
            &tx,
            &PostingSelector::AccountAmount(groceries, aud(dec!(20.00))),
            "G:20:AUD",
            &describe,
        )
        .expect("one match");
        assert_eq!(picked, second.id());
    }

    #[test]
    fn selector_with_no_match_lists_the_postings() {
        let only = posting(&bc_models::AccountId::new(), dec!(5));
        let only_id = only.id().clone();
        let tx = transaction(vec![only]);
        let err = select_posting(
            &tx,
            &PostingSelector::Account(bc_models::AccountId::new()),
            "X",
            &describe,
        )
        .expect_err("no match")
        .to_string();
        assert!(err.contains("no posting matches 'X'"), "got: {err}");
        assert!(err.contains(&only_id.to_string()), "got: {err}");
    }

    #[test]
    fn parse_selector_reads_each_form() {
        let groceries = bc_models::AccountId::new();
        let copy = groceries.clone();
        let lookup = move |text: &str| (text == "Expenses:Groceries").then(|| copy.clone());
        let id = bc_models::PostingId::new();

        assert_eq!(
            parse_selector(&id.to_string(), &lookup).expect("id"),
            PostingSelector::Id(id)
        );
        assert_eq!(
            parse_selector("Expenses:Groceries", &lookup).expect("account"),
            PostingSelector::Account(groceries.clone())
        );
        assert_eq!(
            parse_selector("Expenses:Groceries:20:AUD", &lookup).expect("account and amount"),
            PostingSelector::AccountAmount(groceries, aud(dec!(20)))
        );
        let err = parse_selector("Expenses:Groceries:20:AUD@1:USD", &lookup)
            .expect_err("price")
            .to_string();
        assert!(err.contains("an amount, not a cost or price"), "got: {err}");
    }

    #[test]
    fn apply_set_keeps_the_id_tags_and_spread() {
        let groceries = bc_models::AccountId::new();
        let household = bc_models::AccountId::new();
        let tag = bc_models::TagId::new();
        let kept = bc_models::Posting::builder()
            .id(bc_models::PostingId::new())
            .account_id(groceries)
            .amount(aud(dec!(50)))
            .tag_ids(vec![tag.clone()])
            .spread_from(date(2026, 3, 1))
            .spread_until(date(2026, 3, 31))
            .build();
        let tx = transaction(vec![kept.clone()]);
        let ops = Ops {
            set: vec![(kept.id().clone(), posting(&household, dec!(45)))],
            ..Ops::default()
        };

        let updated = apply(&tx, &ops, &describe).expect("applies");
        let [only] = updated.postings() else {
            panic!("one posting expected");
        };
        assert_eq!(only.id(), kept.id());
        assert_eq!(only.account_id(), &household);
        assert_eq!(only.amount(), Some(&aud(dec!(45))));
        assert_eq!(only.tag_ids(), [tag]);
        assert_eq!(only.spread_from(), Some(date(2026, 3, 1)));
        assert_eq!(only.spread_until(), Some(date(2026, 3, 31)));
    }

    #[test]
    fn apply_adds_and_removes() {
        let a = posting(&bc_models::AccountId::new(), dec!(-50));
        let b = posting(&bc_models::AccountId::new(), dec!(50));
        let c = posting(&bc_models::AccountId::new(), dec!(50));
        let tx = transaction(vec![a.clone(), b.clone()]);
        let ops = Ops {
            add: vec![c.clone()],
            remove: vec![b.id().clone()],
            ..Ops::default()
        };
        let updated = apply(&tx, &ops, &describe).expect("applies");
        let ids: Vec<_> = updated.postings().iter().map(|p| p.id().clone()).collect();
        assert_eq!(ids, [a.id().clone(), c.id().clone()]);
        assert_eq!(updated.id(), tx.id());
        assert_eq!(updated.description(), tx.description());
    }

    #[test]
    fn apply_rejects_a_posting_named_twice() {
        let a = posting(&bc_models::AccountId::new(), dec!(-50));
        let tx = transaction(vec![a.clone()]);
        let ops = Ops {
            set: vec![(
                a.id().clone(),
                posting(&bc_models::AccountId::new(), dec!(-50)),
            )],
            remove: vec![a.id().clone()],
            ..Ops::default()
        };
        let err = apply(&tx, &ops, &describe).expect_err("twice").to_string();
        assert!(err.contains("named more than once"), "got: {err}");
    }

    #[test]
    fn imported_removals_names_only_imported_postings() {
        let imported = posting(&bc_models::AccountId::new(), dec!(-50));
        let manual = posting(&bc_models::AccountId::new(), dec!(50));
        let tx = transaction(vec![imported.clone(), manual.clone()]);
        let ops = Ops {
            remove: vec![imported.id().clone(), manual.id().clone()],
            ..Ops::default()
        };
        let sourced: HashSet<String> = HashSet::from([imported.id().to_string()]);
        let named: Vec<_> = imported_removals(&tx, &ops, &sourced)
            .into_iter()
            .map(|p| p.id().clone())
            .collect();
        assert_eq!(named, [imported.id().clone()]);
    }
}
