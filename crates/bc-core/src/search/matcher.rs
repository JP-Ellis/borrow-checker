//! A resolved query with every account and tag set expanded to ids, evaluated
//! per leg (spec §2).

use std::collections::HashSet;
use std::hash::Hash;
use std::str::FromStr;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::MetaEntry;
use bc_models::MetaValue;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::TagId;
use bc_models::Transaction;
use bc_query::filter::AmountPred;
use bc_query::filter::DateRange;
use bc_query::filter::MetaPred;
use bc_query::filter::NumRange;
use bc_query::filter::Pred;
use bc_query::filter::ResolvedExpr;
use bc_query::filter::Status;
use bc_query::filter::TagPred;
use bc_query::filter::TextMatch;
use bc_query::filter::TimeRange;
use jiff::civil::Date;
use rust_decimal::Decimal;

use super::catalog::DbCatalog;
use super::catalog::under;
use crate::BcError;
use crate::BcResult;
use crate::residual::Residual;
use crate::residual::residual_of_postings;

/// One node of a [`Matcher`].
#[derive(Clone, Debug)]
pub(crate) enum Node {
    /// Holds when any child does.
    Or(Vec<Node>),
    /// Holds when every child does.
    And(Vec<Node>),
    /// Holds when the child does not, on this leg.
    Not(Box<Node>),
    /// Holds on every leg when some leg of the transaction satisfies the child.
    Any(Box<Node>),
    /// A single test.
    Leaf(Leaf),
}

/// A single test, with ids expanded.
#[derive(Clone, Debug)]
pub(crate) enum Leaf {
    /// The description, ASCII-folded.
    Description(TextMatch),
    /// The leg's account is one of these.
    Accounts(HashSet<AccountId>),
    /// A tag on the leg or its transaction.
    Tags(TagTest),
    /// The transaction's reconciliation state.
    Reconciliation(Reconciliation),
    /// Whether the transaction balances.
    Balanced(bool),
    /// The transaction date.
    Date(DateRange),
    /// One of the leg's amounts, by magnitude.
    Amount(AmountPred),
    /// One of the leg's amounts is in this commodity.
    Commodity(String),
    /// A metadata value on the leg or its transaction.
    Meta {
        /// The normalised key.
        key: String,
        /// What one of its values must satisfy.
        test: MetaTest,
    },
}

/// A tag test.
#[derive(Clone, Debug)]
pub(crate) enum TagTest {
    /// Any tag at all.
    Any,
    /// One of these tags.
    Set(HashSet<TagId>),
}

/// A metadata value test, by the key's registered type.
#[derive(Clone, Debug)]
pub(crate) enum MetaTest {
    /// Some value is present, mismatched ones included.
    Exists,
    /// A text value.
    Text(TextMatch),
    /// A number, signed.
    Number(NumRange),
    /// An amount, signed.
    Amount(AmountPred),
    /// A boolean.
    Boolean(bool),
    /// A date.
    Date(DateRange),
    /// A timestamp.
    Timestamp(TimeRange),
    /// An account value.
    Account {
        /// Live accounts at, or under, the path.
        ids: HashSet<AccountId>,
        /// The path, compared ASCII-case-insensitively against tombstones.
        path: Vec<String>,
        /// Whether paths beneath also match.
        subtree: bool,
    },
}

/// A resolved query ready to evaluate per leg and to compile to SQL.
#[derive(Clone, Debug)]
pub(crate) struct Matcher {
    /// The expression.
    root: Node,
}

/// A transaction with its residual derived once.
struct TxView<'t> {
    /// The transaction.
    tx: &'t Transaction,
    /// The elided leg's amounts: the attributable residual's components.
    residual: Vec<Amount>,
}

impl<'t> TxView<'t> {
    /// Derives `tx`'s residual by weight.
    fn new(tx: &'t Transaction) -> Self {
        let residual = match residual_of_postings(tx.postings()) {
            Ok(Residual::Attributable(balances)) => balances
                .iter()
                .map(|(code, value)| Amount::new(value, code))
                .collect(),
            Ok(Residual::NotElided | Residual::Ambiguous) | Err(_) => Vec::new(),
        };
        Self { tx, residual }
    }

    /// The amounts `posting` stands for: its own, or the residual's components.
    fn amounts(&self, posting: &Posting) -> Vec<Amount> {
        posting
            .amount()
            .map_or_else(|| self.residual.clone(), |amount| vec![amount.clone()])
    }
}

impl Matcher {
    /// Expands `expr`'s account and tag references to id sets.
    ///
    /// A reference to an id the catalog does not hold expands to an empty set
    /// and matches nothing.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] when a catalog id does not parse.
    pub(crate) fn new(expr: &ResolvedExpr, catalog: &DbCatalog) -> BcResult<Self> {
        Ok(Self {
            root: node(expr, catalog)?,
        })
    }

    /// Joins the register's or budget's account scope as a per-leg conjunct.
    pub(crate) fn scoped(self, scope: &[AccountId]) -> Self {
        Self {
            root: Node::And(vec![
                Node::Leaf(Leaf::Accounts(scope.iter().cloned().collect())),
                self.root,
            ]),
        }
    }

    /// The expression.
    pub(crate) const fn root(&self) -> &Node {
        &self.root
    }

    /// The legs of `tx` the expression holds on; empty when it holds on none.
    pub(crate) fn matched_postings(&self, tx: &Transaction) -> HashSet<PostingId> {
        let view = TxView::new(tx);
        tx.postings()
            .iter()
            .filter(|posting| eval(&self.root, &view, posting, &view.amounts(posting)))
            .map(|posting| posting.id().clone())
            .collect()
    }

    /// Whether the expression holds on `posting`, a leg of `tx`.
    pub(crate) fn matches_leg(&self, tx: &Transaction, posting: &Posting) -> bool {
        let view = TxView::new(tx);
        eval(&self.root, &view, posting, &view.amounts(posting))
    }

    /// The amounts of `posting` the expression admits: its own amount when the
    /// leg matches, or each residual component of an elided leg that matches
    /// as the leg's only amount.
    pub(crate) fn components(&self, tx: &Transaction, posting: &Posting) -> Vec<Amount> {
        let view = TxView::new(tx);
        view.amounts(posting)
            .into_iter()
            .filter(|amount| eval(&self.root, &view, posting, core::slice::from_ref(amount)))
            .collect()
    }
}

/// Whether `node` holds on `posting`, whose amounts are `amounts`.
fn eval(node: &Node, view: &TxView<'_>, posting: &Posting, amounts: &[Amount]) -> bool {
    match node {
        Node::Or(items) => items.iter().any(|item| eval(item, view, posting, amounts)),
        Node::And(items) => items.iter().all(|item| eval(item, view, posting, amounts)),
        Node::Not(inner) => !eval(inner, view, posting, amounts),
        Node::Any(inner) => view
            .tx
            .postings()
            .iter()
            .any(|leg| eval(inner, view, leg, &view.amounts(leg))),
        Node::Leaf(leaf) => holds(leaf, view.tx, posting, amounts),
    }
}

/// Whether `leaf` holds on `posting` of `tx`.
fn holds(leaf: &Leaf, tx: &Transaction, posting: &Posting, amounts: &[Amount]) -> bool {
    match leaf {
        Leaf::Description(test) => text_holds(test, tx.description()),
        Leaf::Accounts(ids) => ids.contains(posting.account_id()),
        Leaf::Tags(TagTest::Any) => !tx.tag_ids().is_empty() || !posting.tag_ids().is_empty(),
        Leaf::Tags(TagTest::Set(ids)) => tx
            .tag_ids()
            .iter()
            .chain(posting.tag_ids())
            .any(|tag| ids.contains(tag)),
        Leaf::Reconciliation(state) => tx.reconciliation() == *state,
        Leaf::Balanced(want) => tx.balanced() == *want,
        Leaf::Date(range) => date_in(range, tx.date()),
        Leaf::Amount(pred) => amounts
            .iter()
            .any(|amount| amount_holds(pred, amount, amount.value().abs())),
        Leaf::Commodity(code) => amounts
            .iter()
            .any(|amount| amount.commodity().as_str() == code),
        Leaf::Meta { key, test } => tx
            .metadata()
            .iter()
            .chain(posting.metadata().iter())
            .filter(|entry| entry.key().as_str() == key)
            .any(|entry| meta_holds(test, entry)),
    }
}

/// Whether `text`, ASCII-folded, satisfies `rule`, whose needle is folded already.
fn text_holds(rule: &TextMatch, text: &str) -> bool {
    let folded = text.to_ascii_lowercase();
    match rule {
        TextMatch::Contains(needle) => folded.contains(needle.as_str()),
        TextMatch::Equals(needle) => folded == *needle,
    }
}

/// Whether `date` lies in `range`.
fn date_in(range: &DateRange, date: Date) -> bool {
    range.from.is_none_or(|from| date >= from) && range.until.is_none_or(|until| date < until)
}

/// Whether `amount` is in `pred`'s commodity (if any) and `value` in its range.
fn amount_holds(pred: &AmountPred, amount: &Amount, value: Decimal) -> bool {
    pred.commodity
        .as_deref()
        .is_none_or(|code| amount.commodity().as_str() == code)
        && pred.range.contains(value)
}

/// Whether one metadata entry satisfies `test`. A mismatched entry holds text,
/// so only `Exists`, `Text` and a tombstoned `Account` path can match it.
fn meta_holds(test: &MetaTest, entry: &MetaEntry) -> bool {
    let value = entry.value();
    match test {
        MetaTest::Exists => true,
        MetaTest::Text(needle) => {
            matches!(value, MetaValue::Text(stored) if text_holds(needle, stored))
        }
        MetaTest::Number(range) => matches!(value, MetaValue::Number(n) if range.contains(*n)),
        MetaTest::Amount(pred) => {
            matches!(value, MetaValue::Amount(amount) if amount_holds(pred, amount, amount.value()))
        }
        MetaTest::Boolean(want) => matches!(value, MetaValue::Boolean(b) if b == want),
        MetaTest::Date(range) => matches!(value, MetaValue::Date(d) if date_in(range, *d)),
        MetaTest::Timestamp(range) => matches!(value, MetaValue::Timestamp(t)
            if range.from.is_none_or(|from| *t >= from) && range.until.is_none_or(|until| *t < until)),
        MetaTest::Account { ids, path, subtree } => match value {
            MetaValue::Account(id) => ids.contains(id),
            MetaValue::Text(stored) if entry.mismatched() => {
                let segments: Vec<String> = stored.split(':').map(str::to_owned).collect();
                under(&segments, path, *subtree, str::eq_ignore_ascii_case)
            }
            MetaValue::Text(_)
            | MetaValue::Number(_)
            | MetaValue::Boolean(_)
            | MetaValue::Date(_)
            | MetaValue::Timestamp(_)
            | MetaValue::Amount(_) => false,
        },
    }
}

/// Expands one expression node.
fn node(expr: &ResolvedExpr, catalog: &DbCatalog) -> BcResult<Node> {
    Ok(match expr {
        ResolvedExpr::Or(items) => Node::Or(nodes(items, catalog)?),
        ResolvedExpr::And(items) => Node::And(nodes(items, catalog)?),
        ResolvedExpr::Not(inner) => Node::Not(Box::new(node(inner, catalog)?)),
        ResolvedExpr::Any(inner) => Node::Any(Box::new(node(inner, catalog)?)),
        ResolvedExpr::Pred(pred) => Node::Leaf(leaf(pred, catalog)?),
    })
}

/// Expands a list of nodes.
fn nodes(items: &[ResolvedExpr], catalog: &DbCatalog) -> BcResult<Vec<Node>> {
    items.iter().map(|item| node(item, catalog)).collect()
}

/// Expands one predicate.
fn leaf(pred: &Pred, catalog: &DbCatalog) -> BcResult<Leaf> {
    Ok(match pred {
        Pred::Description(test) => Leaf::Description(test.clone()),
        Pred::Account { id, subtree } => {
            Leaf::Accounts(parse_ids(catalog.account_ids(id, *subtree))?)
        }
        Pred::Tag(TagPred::Any) => Leaf::Tags(TagTest::Any),
        Pred::Tag(TagPred::Tag { id, subtree }) => {
            Leaf::Tags(TagTest::Set(parse_ids(catalog.tag_ids(id, *subtree))?))
        }
        Pred::Status(Status::Unreconciled) => Leaf::Reconciliation(Reconciliation::Unreconciled),
        Pred::Status(Status::Flagged) => Leaf::Reconciliation(Reconciliation::Flagged),
        Pred::Status(Status::Reconciled) => Leaf::Reconciliation(Reconciliation::Reconciled),
        Pred::Status(Status::Balanced) => Leaf::Balanced(true),
        Pred::Status(Status::Unbalanced) => Leaf::Balanced(false),
        Pred::Date(range) => Leaf::Date(*range),
        Pred::Amount(amount) => Leaf::Amount(amount.clone()),
        Pred::Commodity(code) => Leaf::Commodity(code.clone()),
        Pred::Meta { key, pred: meta } => Leaf::Meta {
            key: key.clone(),
            test: meta_test(meta, catalog)?,
        },
    })
}

/// Expands one metadata predicate.
fn meta_test(pred: &MetaPred, catalog: &DbCatalog) -> BcResult<MetaTest> {
    Ok(match pred {
        MetaPred::Exists => MetaTest::Exists,
        MetaPred::Text(test) => MetaTest::Text(test.clone()),
        MetaPred::Number(range) => MetaTest::Number(*range),
        MetaPred::Amount(amount) => MetaTest::Amount(amount.clone()),
        MetaPred::Boolean(want) => MetaTest::Boolean(*want),
        MetaPred::Date(range) => MetaTest::Date(*range),
        MetaPred::Timestamp(range) => MetaTest::Timestamp(*range),
        MetaPred::Account { path, subtree } => MetaTest::Account {
            ids: parse_ids(catalog.account_ids_by_path(path, *subtree))?,
            path: path.clone(),
            subtree: *subtree,
        },
    })
}

/// Parses catalog ids.
///
/// # Errors
///
/// Returns [`BcError::BadData`] naming the first id that does not parse.
fn parse_ids<T>(ids: Vec<&str>) -> BcResult<HashSet<T>>
where
    T: FromStr + Eq + Hash,
    T::Err: core::fmt::Display,
{
    ids.into_iter()
        .map(|id| {
            id.parse::<T>().map_err(|e| {
                BcError::BadData(format!("invalid id '{id}' in the query catalog: {e}"))
            })
        })
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashMap;

    use bc_models::MetaKey as ModelKey;
    use bc_models::Metadata;
    use bc_models::TransactionId;
    use bc_query::catalog::MetaKey;
    use bc_query::catalog::MetaType;
    use bc_query::catalog::PathEntry;
    use bc_query::currency::Commodity;
    use bc_query::parse;
    use bc_query::resolve;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;

    /// Every account path the fixtures use.
    const ACCOUNTS: [&str; 14] = [
        "Liability",
        "Liability:CC",
        "Asset",
        "Asset:Me",
        "Asset:Partner",
        "Asset:Holiday",
        "Asset:Shared",
        "Assets",
        "Assets:Bank",
        "Assets:Cash",
        "Expenses",
        "Expenses:Food",
        "Expenses:Fuel",
        "Expenses:Food:Snacks",
    ];

    /// Every tag path the fixtures use.
    const TAGS: [&str; 4] = ["me", "partner", "trip", "trip:flights"];

    /// An invented ledger: ids by path, and its catalog.
    struct Ledger {
        /// Account ids by path.
        accounts: HashMap<&'static str, AccountId>,
        /// Tag ids by path.
        tags: HashMap<&'static str, TagId>,
        /// The catalog the queries resolve against.
        catalog: DbCatalog,
    }

    impl Ledger {
        fn new() -> Self {
            let accounts: HashMap<&'static str, AccountId> =
                ACCOUNTS.iter().map(|p| (*p, AccountId::new())).collect();
            let tags: HashMap<&'static str, TagId> =
                TAGS.iter().map(|p| (*p, TagId::new())).collect();
            let entries = |ids: Vec<(String, &str)>| -> Vec<PathEntry> {
                ids.into_iter()
                    .map(|(id, path)| PathEntry::new(id, path.split(':')))
                    .collect()
            };
            let catalog = DbCatalog::from_parts(
                entries(
                    accounts
                        .iter()
                        .map(|(p, id)| (id.to_string(), *p))
                        .collect(),
                ),
                entries(tags.iter().map(|(p, id)| (id.to_string(), *p)).collect()),
                vec![
                    Commodity::new("AUD", Some("A$"), &[]),
                    Commodity::new("USD", Some("$"), &[]),
                ],
                vec![
                    MetaKey::new("deposit", MetaType::Amount, 0),
                    MetaKey::new("due", MetaType::Date, 0),
                    MetaKey::new("km", MetaType::Number, 1),
                    MetaKey::new("owner", MetaType::Account, 0),
                    MetaKey::new("paid", MetaType::Boolean, 0),
                    MetaKey::new("payee", MetaType::Text, 0),
                    MetaKey::new("receipt", MetaType::Text, 0),
                    MetaKey::new("seen", MetaType::Timestamp, 0),
                ],
            );
            Self {
                accounts,
                tags,
                catalog,
            }
        }

        fn account(&self, path: &str) -> AccountId {
            self.accounts.get(path).cloned().expect("fixture account")
        }

        fn tag(&self, path: &str) -> TagId {
            self.tags.get(path).cloned().expect("fixture tag")
        }

        /// The path of the account `id`.
        fn path_of(&self, id: &AccountId) -> &'static str {
            self.accounts
                .iter()
                .find(|(_, v)| *v == id)
                .map(|(k, _)| *k)
                .expect("fixture id")
        }

        /// A leg on `path`; `amount` `None` elides it.
        fn leg(
            &self,
            path: &str,
            amount: Option<(Decimal, &str)>,
            tags: &[&str],
            meta: Vec<MetaEntry>,
        ) -> Posting {
            let builder = Posting::builder()
                .id(PostingId::new())
                .account_id(self.account(path))
                .tag_ids(tags.iter().map(|t| self.tag(t)).collect())
                .metadata(Metadata::new(meta));
            match amount {
                Some((value, code)) => builder.amount(Amount::new(value, code)).build(),
                None => builder.build(),
            }
        }

        /// The matcher for `query`, which must resolve without errors.
        fn matcher(&self, query: &str) -> Matcher {
            let resolved = resolve(&parse(query).expect("parses"), &self.catalog);
            let expr = resolved
                .expr
                .unwrap_or_else(|| panic!("{query}: {:?}", resolved.diagnostics));
            Matcher::new(&expr, &self.catalog).expect("matcher")
        }

        /// The paths of the legs of `tx` that `query` matches, sorted.
        fn hits(&self, tx: &Transaction, query: &str) -> Vec<&'static str> {
            let matched = self.matcher(query).matched_postings(tx);
            let mut paths: Vec<&'static str> = tx
                .postings()
                .iter()
                .filter(|p| matched.contains(p.id()))
                .map(|p| self.path_of(p.account_id()))
                .collect();
            paths.sort_unstable();
            paths
        }
    }

    /// A metadata entry whose value fits its key.
    fn meta(key: &str, value: MetaValue) -> MetaEntry {
        MetaEntry::new(ModelKey::new(key).expect("key"), value)
    }

    /// A transaction dated 2026-03-14.
    fn tx(
        description: &str,
        postings: Vec<Posting>,
        tags: Vec<TagId>,
        metadata: Vec<MetaEntry>,
    ) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(date(2026, 3, 14))
            .description(description)
            .postings(postings)
            .tag_ids(tags)
            .metadata(Metadata::new(metadata))
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build()
    }

    /// Spec §2's worked example: one card payment split across four asset legs.
    fn split(l: &Ledger) -> Transaction {
        tx(
            "Shared holiday booking",
            vec![
                l.leg("Liability:CC", Some((dec!(5000), "AUD")), &[], vec![]),
                l.leg("Asset:Me", Some((dec!(-1000), "AUD")), &["me"], vec![]),
                l.leg(
                    "Asset:Partner",
                    Some((dec!(-1000), "AUD")),
                    &["partner"],
                    vec![],
                ),
                l.leg("Asset:Holiday", Some((dec!(-2000), "AUD")), &[], vec![]),
                l.leg("Asset:Shared", Some((dec!(-1000), "AUD")), &[], vec![]),
            ],
            vec![],
            vec![],
        )
    }

    /// Food against an elided bank leg, carrying one value of every key type.
    fn groceries(l: &Ledger) -> Transaction {
        let seen: Timestamp = "2026-03-14T09:30:00Z".parse().expect("timestamp");
        tx(
            "Example Grocer weekly shop",
            vec![
                l.leg(
                    "Expenses:Food",
                    Some((dec!(50), "AUD")),
                    &[],
                    vec![meta("receipt", MetaValue::Text("R-1".to_owned()))],
                ),
                l.leg("Assets:Bank", None, &[], vec![]),
            ],
            vec![l.tag("trip")],
            vec![
                meta("payee", MetaValue::Text("Example Grocer".to_owned())),
                meta("receipt", MetaValue::Text("R-2".to_owned())),
                meta("km", MetaValue::Number(dec!(1200))),
                meta("due", MetaValue::Date(date(2026, 5, 1))),
                meta("paid", MetaValue::Boolean(true)),
                meta("seen", MetaValue::Timestamp(seen)),
                meta("owner", MetaValue::Account(l.account("Assets:Bank"))),
                meta("deposit", MetaValue::Amount(Amount::new(dec!(-20), "AUD"))),
            ],
        )
    }

    /// Two commodities against one elided leg: a multi-commodity residual.
    fn mixed(l: &Ledger) -> Transaction {
        tx(
            "Fuel and food",
            vec![
                l.leg(
                    "Expenses:Fuel",
                    Some((dec!(30), "USD")),
                    &["trip:flights"],
                    vec![],
                ),
                l.leg("Expenses:Food", Some((dec!(20), "AUD")), &[], vec![]),
                l.leg("Assets:Bank", None, &[], vec![]),
            ],
            vec![],
            vec![],
        )
    }

    /// Two elided legs: the residual is ambiguous.
    fn ambiguous(l: &Ledger) -> Transaction {
        tx(
            "Ambiguous",
            vec![
                l.leg("Expenses:Food", Some((dec!(10), "AUD")), &[], vec![]),
                l.leg("Assets:Bank", None, &[], vec![]),
                l.leg("Assets:Cash", None, &[], vec![]),
            ],
            vec![],
            vec![],
        )
    }

    /// Non-ASCII text, a mismatched number and a tombstoned account value.
    fn odd(l: &Ledger) -> Transaction {
        tx(
            "Café Zoë",
            vec![
                l.leg("Expenses:Food", Some((dec!(5), "AUD")), &[], vec![]),
                l.leg("Assets:Bank", Some((dec!(-5), "AUD")), &[], vec![]),
            ],
            vec![],
            vec![
                MetaEntry::mismatch(ModelKey::new("km").expect("key"), "lots"),
                MetaEntry::mismatch(ModelKey::new("owner").expect("key"), "Assets:Gone"),
            ],
        )
    }

    #[rstest]
    #[case("account:Asset tag:me", vec!["Asset:Me"])]
    #[case("account:Asset -tag:me -tag:partner", vec!["Asset:Holiday", "Asset:Shared"])]
    #[case("-tag:me -tag:partner", vec!["Asset:Holiday", "Asset:Shared", "Liability:CC"])]
    #[case("account:Asset -any:(tag:me)", vec![])]
    #[case("any:(tag:me)", vec!["Asset:Holiday", "Asset:Me", "Asset:Partner", "Asset:Shared", "Liability:CC"])]
    #[case("tag:*", vec!["Asset:Me", "Asset:Partner"])]
    fn worked_example(#[case] query: &str, #[case] expected: Vec<&str>) {
        let l = Ledger::new();
        assert_eq!(l.hits(&split(&l), query), expected);
    }

    #[test]
    fn scope_joins_as_a_per_leg_conjunct() {
        let l = Ledger::new();
        let t = split(&l);
        let scoped_negated = l
            .matcher("-tag:me -tag:partner")
            .scoped(&[l.account("Asset:Shared")]);
        let hits: Vec<&str> = t
            .postings()
            .iter()
            .filter(|p| scoped_negated.matched_postings(&t).contains(p.id()))
            .map(|p| l.path_of(p.account_id()))
            .collect();
        assert_eq!(hits, vec!["Asset:Shared"]);
        let scoped_tagged = l.matcher("tag:me").scoped(&[l.account("Asset:Shared")]);
        assert!(scoped_tagged.matched_postings(&t).is_empty());
    }

    #[rstest]
    // Elided legs resolve through the residual by weight.
    #[case(groceries, "amount:50 account:Assets", vec!["Assets:Bank"])]
    #[case(groceries, "amount:50", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "amount:>50", vec![])]
    #[case(groceries, "amount:>=50", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "commodity:AUD account:Assets:Bank", vec!["Assets:Bank"])]
    #[case(mixed, "amount:30 account:Assets:Bank", vec!["Assets:Bank"])]
    #[case(mixed, "amount:>A$25 account:Assets:Bank", vec![])]
    #[case(mixed, "commodity:USD", vec!["Assets:Bank", "Expenses:Fuel"])]
    #[case(ambiguous, "amount:10", vec!["Expenses:Food"])]
    #[case(ambiguous, "commodity:AUD", vec!["Expenses:Food"])]
    // Account and tag subtrees.
    #[case(mixed, "account:Expenses", vec!["Expenses:Food", "Expenses:Fuel"])]
    #[case(mixed, "account:=Expenses:Food", vec!["Expenses:Food"])]
    #[case(mixed, "tag:trip", vec!["Expenses:Fuel"])]
    #[case(mixed, "tag:=trip", vec![])]
    #[case(groceries, "tag:trip", vec!["Assets:Bank", "Expenses:Food"])]
    // Transaction-level fields hold on every leg.
    #[case(groceries, "status:unreconciled", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "status:reconciled", vec![])]
    #[case(groceries, "status:balanced", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(ambiguous, "status:unbalanced", vec!["Assets:Bank", "Assets:Cash", "Expenses:Food"])]
    #[case(groceries, "date:2026-03", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "date:<2026-03-14", vec![])]
    #[case(groceries, "grocer", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "description:=\"example grocer weekly shop\"", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(odd, "description:café", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(odd, "description:CAFÉ", vec![])]
    // Metadata: leg or transaction, repeated keys, every type.
    #[case(groceries, "@payee:grocer", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@receipt:=r-1", vec!["Expenses:Food"])]
    #[case(groceries, "@receipt:=r-2", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "-@receipt:=r-1", vec!["Assets:Bank"])]
    #[case(groceries, "@km:>1000", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@km:>2000", vec![])]
    #[case(groceries, "@due:2026-05", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@due:<2026-05-01", vec![])]
    #[case(groceries, "@paid:true", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@paid:false", vec![])]
    #[case(groceries, "@seen:2026-03-14", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@seen:2026-03-15", vec![])]
    #[case(groceries, "@deposit:<0", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@deposit:>A$0", vec![])]
    #[case(groceries, "@owner:Assets", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(groceries, "@owner:=Assets", vec![])]
    // Mismatched entries are skipped by typed tests and counted by `*`.
    #[case(odd, "@km:>0", vec![])]
    #[case(odd, "@km:*", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(odd, "-@km:*", vec![])]
    // Tombstones compare their stored path ASCII-case-insensitively.
    #[case(odd, "@owner:=assets:gone", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(odd, "@owner:Assets", vec!["Assets:Bank", "Expenses:Food"])]
    #[case(odd, "@owner:=Assets:Bank", vec![])]
    fn evaluates_per_leg(
        #[case] fixture: fn(&Ledger) -> Transaction,
        #[case] query: &str,
        #[case] expected: Vec<&str>,
    ) {
        let l = Ledger::new();
        assert_eq!(l.hits(&fixture(&l), query), expected);
    }

    #[test]
    fn components_split_an_elided_leg_by_commodity() {
        let l = Ledger::new();
        let t = mixed(&l);
        let bank = t
            .postings()
            .iter()
            .find(|p| p.amount().is_none())
            .expect("elided leg");
        let food = t
            .postings()
            .iter()
            .find(|p| l.path_of(p.account_id()) == "Expenses:Food")
            .expect("food");

        assert_eq!(
            l.matcher("amount:>25").components(&t, bank),
            vec![Amount::new(dec!(-30), "USD")]
        );
        assert_eq!(l.matcher("account:Assets").components(&t, bank).len(), 2);
        assert_eq!(
            l.matcher("amount:20").components(&t, food),
            vec![Amount::new(dec!(20), "AUD")]
        );
        assert_eq!(
            l.matcher("amount:30").components(&t, food),
            Vec::<Amount>::new()
        );
        assert!(l.matcher("amount:>25").matches_leg(&t, bank));
        assert!(!l.matcher("amount:>25").matches_leg(&t, food));

        let a = ambiguous(&l);
        let elided = a
            .postings()
            .iter()
            .find(|p| p.amount().is_none())
            .expect("elided");
        assert_eq!(
            l.matcher("account:Assets").components(&a, elided),
            Vec::<Amount>::new()
        );
    }

    #[test]
    fn an_unknown_id_matches_nothing() {
        let l = Ledger::new();
        let expr = ResolvedExpr::Pred(Pred::Account {
            id: AccountId::new().to_string(),
            subtree: true,
        });
        let matcher = Matcher::new(&expr, &l.catalog).expect("matcher");
        assert!(matcher.matched_postings(&split(&l)).is_empty());
    }

    #[test]
    fn a_malformed_id_is_bad_data() {
        let catalog = DbCatalog::from_parts(
            vec![PathEntry::new("not-an-id!!", ["Broken"])],
            vec![],
            vec![],
            vec![],
        );
        let expr = ResolvedExpr::Pred(Pred::Account {
            id: "not-an-id!!".to_owned(),
            subtree: false,
        });
        assert!(matches!(
            Matcher::new(&expr, &catalog),
            Err(BcError::BadData(_))
        ));
    }
}
