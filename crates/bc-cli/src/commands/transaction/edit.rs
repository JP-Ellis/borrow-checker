//! Selecting a transaction and its postings, and applying the changes of
//! `transaction edit`.

use core::str::FromStr as _;
use std::collections::HashSet;

use super::changes;
use super::spec;
use crate::commands::meta;
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

/// The changes one `transaction edit` makes.
#[derive(Debug, Default)]
pub(super) struct Ops {
    /// Changes to the transaction itself.
    pub transaction: Option<changes::Resolved>,
    /// Postings to append.
    pub add: Vec<bc_models::Posting>,
    /// Stored postings to change, each with its scope label.
    pub set: Vec<(bc_models::PostingId, String, changes::Resolved)>,
    /// Postings to drop.
    pub remove: Vec<bc_models::PostingId>,
}

/// Parses `POSTING`: a posting ID, an account, or an account with an amount.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when no account matches or the amount is malformed.
pub(super) fn parse_selector(
    tokens: &[String],
    lookup: &spec::Lookup<'_>,
) -> CliResult<PostingSelector> {
    match tokens {
        [one] => {
            if let Ok(id) = bc_models::PostingId::from_str(one.trim()) {
                return Ok(PostingSelector::Id(id));
            }
            lookup(one)
                .map(PostingSelector::Account)
                .ok_or_else(|| CliError::Arg(format!("no account or posting ID '{one}'")))
        }
        [account, value, code] => {
            let id =
                lookup(account).ok_or_else(|| CliError::Arg(format!("no account '{account}'")))?;
            Ok(PostingSelector::AccountAmount(
                id,
                changes::amount_of(value, code)?,
            ))
        }
        _ => Err(CliError::Arg(
            "a posting is an ID, an account, or ACCOUNT AMOUNT COMMODITY".into(),
        )),
    }
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

/// Refuses a stored posting that `ids` names more than once.
///
/// # Errors
///
/// Returns [`CliError::Arg`] naming the posting.
pub(super) fn named_once<'i>(
    current: &bc_models::Transaction,
    ids: impl IntoIterator<Item = &'i bc_models::PostingId>,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<()> {
    let mut named: HashSet<&bc_models::PostingId> = HashSet::new();
    for id in ids {
        if !named.insert(id) {
            let label = current
                .postings()
                .iter()
                .find(|p| p.id() == id)
                .map_or_else(|| id.to_string(), describe);
            return Err(CliError::Arg(format!(
                "posting '{label}' is named by more than one --set or --remove"
            )));
        }
    }
    Ok(())
}

/// Applies `ops` to `current`, keeping every posting they do not name.
///
/// A set posting keeps its ID, so its import reference stays linked, and
/// changes only what its scope names.
///
/// # Errors
///
/// Returns [`CliError::Arg`] when one posting is named by more than one set or
/// remove, or when a set leaves a lot date or label with no cost.
pub(super) fn apply(
    current: &bc_models::Transaction,
    ops: &Ops,
    describe: &dyn Fn(&bc_models::Posting) -> String,
) -> CliResult<bc_models::Transaction> {
    named_once(
        current,
        ops.set.iter().map(|(id, ..)| id).chain(&ops.remove),
        describe,
    )?;

    let mut postings: Vec<bc_models::Posting> = current
        .postings()
        .iter()
        .filter(|p| !ops.remove.contains(p.id()))
        .map(|p| match ops.set.iter().find(|(id, ..)| id == p.id()) {
            Some((_, label, resolved)) => changes::set_posting(p, resolved, label),
            None => Ok(p.clone()),
        })
        .collect::<CliResult<Vec<_>>>()?;
    postings.extend(ops.add.iter().cloned());

    let (date, description, metadata, tag_ids) = match &ops.transaction {
        Some(tx) => (
            tx.changes.date.unwrap_or_else(|| current.date()),
            tx.changes
                .description
                .clone()
                .unwrap_or_else(|| current.description().to_owned()),
            meta::apply_changes(current.metadata(), &tx.entries, &tx.changes.clear_meta),
            changes::retag(current.tag_ids(), &tx.tags, &tx.untags),
        ),
        None => (
            current.date(),
            current.description().to_owned(),
            current.metadata().clone(),
            current.tag_ids().to_vec(),
        ),
    };

    Ok(bc_models::Transaction::builder()
        .id(current.id().clone())
        .date(date)
        .description(description)
        .metadata(metadata)
        .postings(postings)
        .tag_ids(tag_ids)
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
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::super::changes::Changes;
    use super::super::changes::Resolved;
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
            "--set G 20 AUD",
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

    fn tokens(texts: &[&str]) -> Vec<String> {
        texts.iter().map(|t| (*t).to_owned()).collect()
    }

    fn resolved(changes: Changes) -> Resolved {
        Resolved {
            changes,
            account: None,
            entries: Vec::new(),
            tags: Vec::new(),
            untags: Vec::new(),
        }
    }

    #[test]
    fn parse_selector_reads_each_form() {
        let groceries = bc_models::AccountId::new();
        let copy = groceries.clone();
        let lookup = move |text: &str| (text == "Expenses:Groceries").then(|| copy.clone());
        let id = bc_models::PostingId::new();

        assert_eq!(
            parse_selector(&tokens(&[&id.to_string()]), &lookup).expect("id"),
            PostingSelector::Id(id)
        );
        assert_eq!(
            parse_selector(&tokens(&["Expenses:Groceries"]), &lookup).expect("account"),
            PostingSelector::Account(groceries.clone())
        );
        assert_eq!(
            parse_selector(&tokens(&["Expenses:Groceries", "20", "AUD"]), &lookup)
                .expect("account and amount"),
            PostingSelector::AccountAmount(groceries, aud(dec!(20)))
        );
    }

    #[rstest]
    #[case::unknown_account(&["Expenses:Nowhere"], "no account or posting ID 'Expenses:Nowhere'")]
    #[case::unknown_account_with_amount(&["Expenses:Nowhere", "20", "AUD"], "no account 'Expenses:Nowhere'")]
    #[case::bad_amount(&["Expenses:Groceries", "abc", "AUD"], "invalid amount 'abc'")]
    #[case::two_tokens(&["Expenses:Groceries", "20"], "ACCOUNT AMOUNT COMMODITY")]
    fn parse_selector_rejects(#[case] texts: &[&str], #[case] expected: &str) {
        let lookup = |text: &str| (text == "Expenses:Groceries").then(bc_models::AccountId::new);
        let err = parse_selector(&tokens(texts), &lookup)
            .expect_err("rejects")
            .to_string();
        assert!(err.contains(expected), "got: {err}");
    }

    #[test]
    fn apply_routes_a_set_to_its_posting_and_keeps_the_rest() {
        let checking = posting(&bc_models::AccountId::new(), dec!(-50));
        let groceries = posting(&bc_models::AccountId::new(), dec!(50));
        let tx = transaction(vec![checking.clone(), groceries.clone()]);
        let changes = Changes {
            amount: Some(aud(dec!(45))),
            ..Changes::default()
        };
        let ops = Ops {
            set: vec![(
                groceries.id().clone(),
                "--set G".to_owned(),
                resolved(changes),
            )],
            ..Ops::default()
        };

        let updated = apply(&tx, &ops, &describe).expect("applies");
        let [first, second] = updated.postings() else {
            panic!("two postings expected");
        };
        assert_eq!(first, &checking);
        assert_eq!(second.id(), groceries.id());
        assert_eq!(second.account_id(), groceries.account_id());
        assert_eq!(second.amount(), Some(&aud(dec!(45))));
    }

    #[test]
    fn apply_changes_the_transaction_metadata_and_tags() {
        let kept = bc_models::TagId::new();
        let dropped = bc_models::TagId::new();
        let added = bc_models::TagId::new();
        let note = bc_models::MetaKey::new("note").expect("valid key");
        let invoice = bc_models::MetaKey::new("invoice").expect("valid key");
        let base = transaction(vec![posting(&bc_models::AccountId::new(), dec!(5))]);
        let tx = bc_models::Transaction::builder()
            .id(base.id().clone())
            .date(base.date())
            .description(base.description().to_owned())
            .postings(base.postings().to_vec())
            .metadata(bc_models::Metadata::new(vec![bc_models::MetaEntry::new(
                note.clone(),
                bc_models::MetaValue::Text("a".to_owned()),
            )]))
            .tag_ids(vec![kept.clone(), dropped.clone()])
            .reconciliation(base.reconciliation())
            .created_at(*base.created_at())
            .build();
        let entry = bc_models::MetaEntry::new(
            invoice.clone(),
            bc_models::MetaValue::Text("1502".to_owned()),
        );
        let ops = Ops {
            transaction: Some(Resolved {
                changes: Changes {
                    clear_meta: vec![note],
                    ..Changes::default()
                },
                account: None,
                entries: vec![entry.clone()],
                tags: vec![added.clone()],
                untags: vec![dropped],
            }),
            ..Ops::default()
        };

        let updated = apply(&tx, &ops, &describe).expect("applies");
        assert_eq!(updated.tag_ids(), [kept, added]);
        assert_eq!(updated.metadata(), &bc_models::Metadata::new(vec![entry]));
        assert_eq!(updated.postings(), tx.postings());
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

    #[rstest]
    #[case::set_and_remove(true)]
    #[case::set_twice(false)]
    fn apply_rejects_a_posting_named_twice(#[case] remove: bool) {
        let a = posting(&bc_models::AccountId::new(), dec!(-50));
        let tx = transaction(vec![a.clone()]);
        let set = |label: &str| {
            (
                a.id().clone(),
                label.to_owned(),
                resolved(Changes {
                    no_price: true,
                    ..Changes::default()
                }),
            )
        };
        let ops = if remove {
            Ops {
                set: vec![set("--set A")],
                remove: vec![a.id().clone()],
                ..Ops::default()
            }
        } else {
            Ops {
                set: vec![set("--set A"), set("--set B")],
                ..Ops::default()
            }
        };
        let err = apply(&tx, &ops, &describe).expect_err("twice").to_string();
        assert!(
            err.contains("named by more than one --set or --remove"),
            "got: {err}"
        );
        assert!(err.contains(&a.account_id().to_string()), "names it: {err}");
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
