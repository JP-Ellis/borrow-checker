//! The candidate SQL a [`Matcher`] compiles to: every transaction the
//! evaluator could accept, and possibly more.

use std::collections::HashSet;

use bc_models::AccountId;
use bc_query::filter::AmountPred;
use bc_query::filter::DateRange;
use bc_query::filter::NumRange;
use bc_query::filter::TextMatch;
use jiff::civil::Date;
use rust_decimal::prelude::ToPrimitive as _;

use super::escape_like;
use super::matcher::Leaf;
use super::matcher::Matcher;
use super::matcher::MetaTest;
use super::matcher::Node;
use super::matcher::TagTest;
use crate::BcResult;
use crate::db::to_db_str;
use crate::transaction::TxRow;
use crate::transaction::sql_placeholders;

/// Absolute widening applied to `REAL` bounds near zero.
const EPSILON: f64 = 0.0001;

/// Relative widening applied to `REAL` bounds. Neither `Decimal::to_f64` nor
/// SQLite's text-to-`REAL` cast is correctly rounded, so a large magnitude can
/// land several ulps from its bound.
const RELATIVE: f64 = 1e-9;

/// A value bound to a `?` placeholder, in placeholder order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Bind {
    /// A text value.
    Text(String),
    /// A `REAL` value.
    Real(f64),
}

/// A compiled candidate query.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Candidates {
    /// The `SELECT`, newest first.
    pub sql: String,
    /// Its binds, in placeholder order.
    pub binds: Vec<Bind>,
    /// Whether the SQL returns exactly the transactions the matcher accepts.
    pub exact: bool,
}

/// A boolean SQL expression over `t` and one leg alias.
///
/// An exact fragment holds exactly when the evaluator does and never
/// evaluates to NULL, so `NOT` inverts it faithfully.
#[derive(Clone, Debug, PartialEq)]
struct Fragment {
    /// The expression.
    sql: String,
    /// Its binds, in placeholder order.
    binds: Vec<Bind>,
    /// Whether it decides the node exactly.
    exact: bool,
}

impl Fragment {
    /// An exact fragment.
    fn exact(sql: impl Into<String>, binds: Vec<Bind>) -> Self {
        Self {
            sql: sql.into(),
            binds,
            exact: true,
        }
    }

    /// An over-matching fragment.
    fn loose(sql: impl Into<String>, binds: Vec<Bind>) -> Self {
        Self {
            sql: sql.into(),
            binds,
            exact: false,
        }
    }

    /// Holds on every row: what a node SQL cannot negate becomes.
    fn always() -> Self {
        Self::loose("1", Vec::new())
    }
}

/// Compilation state: the next free alias number.
struct Compiler {
    /// Suffix for the next alias.
    next: usize,
}

/// Compiles the candidate query for transactions dated in `[from, until)`.
///
/// # Arguments
///
/// * `matcher` - The query, already scoped by the caller when it has a scope.
/// * `from` - Inclusive lower date bound.
/// * `until` - Exclusive upper date bound.
/// * `scope` - Without a matcher, restricts to transactions with a leg on one
///   of these accounts; ignored with one.
///
/// # Errors
///
/// Returns [`crate::BcError`] if a reconciliation state cannot be encoded.
pub(crate) fn candidates(
    matcher: Option<&Matcher>,
    from: Option<Date>,
    until: Option<Date>,
    scope: &[AccountId],
) -> BcResult<Candidates> {
    let mut clauses: Vec<String> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();
    let mut exact = true;
    if let Some(day) = from {
        clauses.push("t.date >= ?".to_owned());
        binds.push(Bind::Text(day.to_string()));
    }
    if let Some(day) = until {
        clauses.push("t.date < ?".to_owned());
        binds.push(Bind::Text(day.to_string()));
    }
    match matcher {
        Some(m) => {
            let mut compiler = Compiler { next: 1 };
            let fragment = compiler.node(m.root(), "p0")?;
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM postings p0 WHERE p0.transaction_id = t.id AND ({}))",
                fragment.sql
            ));
            binds.extend(fragment.binds);
            exact = fragment.exact;
        }
        None if !scope.is_empty() => {
            clauses.push(format!(
                "EXISTS (SELECT 1 FROM postings s WHERE s.transaction_id = t.id AND s.account_id IN ({}))",
                sql_placeholders(scope.len())
            ));
            binds.extend(scope.iter().map(|id| Bind::Text(id.to_string())));
        }
        None => {}
    }
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    };
    Ok(Candidates {
        sql: format!(
            "SELECT t.id, t.date, t.description, t.reconciliation, t.created_at \
             FROM transactions t {filter} ORDER BY t.date DESC, t.id ASC"
        ),
        binds,
        exact,
    })
}

/// Prepares `sql` with `binds` applied in order.
pub(crate) fn statement(
    sql: String,
    binds: Vec<Bind>,
) -> sqlx::query::QueryAs<'static, sqlx::Sqlite, TxRow, sqlx::sqlite::SqliteArguments> {
    let mut stmt = sqlx::query_as::<_, TxRow>(sqlx::AssertSqlSafe(sql));
    for bind in binds {
        stmt = match bind {
            Bind::Text(text) => stmt.bind(text),
            Bind::Real(real) => stmt.bind(real),
        };
    }
    stmt
}

impl Compiler {
    /// A fresh alias: `prefix` and a number unique in this query.
    fn alias(&mut self, prefix: &str) -> String {
        let n = self.next;
        self.next = n.saturating_add(1);
        format!("{prefix}{n}")
    }

    /// Compiles `node` over the leg alias `leg`.
    fn node(&mut self, node: &Node, leg: &str) -> BcResult<Fragment> {
        match node {
            Node::Or(items) => self.join(items, leg, " OR "),
            Node::And(items) => self.join(items, leg, " AND "),
            Node::Not(inner) => {
                let fragment = self.node(inner, leg)?;
                Ok(if fragment.exact {
                    Fragment::exact(format!("NOT ({})", fragment.sql), fragment.binds)
                } else {
                    Fragment::always()
                })
            }
            Node::Any(inner) => {
                let alias = self.alias("p");
                let fragment = self.node(inner, &alias)?;
                Ok(Fragment {
                    sql: format!(
                        "EXISTS (SELECT 1 FROM postings {alias} WHERE {alias}.transaction_id = t.id AND ({}))",
                        fragment.sql
                    ),
                    binds: fragment.binds,
                    exact: fragment.exact,
                })
            }
            Node::Leaf(leaf) => self.leaf(leaf, leg),
        }
    }

    /// Joins `items` with `op`.
    fn join(&mut self, items: &[Node], leg: &str, op: &str) -> BcResult<Fragment> {
        let mut parts: Vec<String> = Vec::with_capacity(items.len());
        let mut binds: Vec<Bind> = Vec::new();
        let mut exact = true;
        for item in items {
            let fragment = self.node(item, leg)?;
            parts.push(format!("({})", fragment.sql));
            binds.extend(fragment.binds);
            exact = exact && fragment.exact;
        }
        Ok(Fragment {
            sql: parts.join(op),
            binds,
            exact,
        })
    }

    /// Compiles one test.
    fn leaf(&mut self, leaf: &Leaf, leg: &str) -> BcResult<Fragment> {
        Ok(match leaf {
            Leaf::Description(test) => text("lower(t.description)", test),
            Leaf::Accounts(ids) => in_list(&format!("{leg}.account_id"), sorted(ids)),
            Leaf::Tags(test) => self.tags(test, leg),
            Leaf::Reconciliation(state) => {
                Fragment::exact("t.reconciliation = ?", vec![Bind::Text(to_db_str(*state)?)])
            }
            Leaf::Balanced(_) => Fragment::always(),
            Leaf::Date(range) => dates("t.date", *range, true),
            Leaf::Amount(pred) => amount(leg, pred),
            Leaf::Commodity(code) => Fragment::loose(
                format!("{leg}.amount IS NULL OR {leg}.commodity = ?"),
                vec![Bind::Text(code.clone())],
            ),
            Leaf::Meta { key, test } => self.meta(key, test, leg),
        })
    }

    /// A tag on the transaction or the leg.
    fn tags(&mut self, test: &TagTest, leg: &str) -> Fragment {
        let tt = self.alias("tt");
        let pt = self.alias("pt");
        match test {
            TagTest::Any => Fragment::exact(
                format!(
                    "EXISTS (SELECT 1 FROM transaction_tags {tt} WHERE {tt}.transaction_id = t.id) \
                     OR EXISTS (SELECT 1 FROM posting_tags {pt} WHERE {pt}.posting_id = {leg}.id)"
                ),
                Vec::new(),
            ),
            TagTest::Set(ids) => {
                let sorted_ids = sorted(ids);
                if sorted_ids.is_empty() {
                    return Fragment::exact("0", Vec::new());
                }
                let marks = sql_placeholders(sorted_ids.len());
                let mut binds: Vec<Bind> = sorted_ids.iter().cloned().map(Bind::Text).collect();
                binds.extend(sorted_ids.into_iter().map(Bind::Text));
                Fragment::exact(
                    format!(
                        "EXISTS (SELECT 1 FROM transaction_tags {tt} \
                                 WHERE {tt}.transaction_id = t.id AND {tt}.tag_id IN ({marks})) \
                         OR EXISTS (SELECT 1 FROM posting_tags {pt} \
                                    WHERE {pt}.posting_id = {leg}.id AND {pt}.tag_id IN ({marks}))"
                    ),
                    binds,
                )
            }
        }
    }

    /// A metadata value on the transaction or the leg.
    fn meta(&mut self, key: &str, test: &MetaTest, leg: &str) -> Fragment {
        let tm = self.alias("tm");
        let pm = self.alias("pm");
        let on_tx = value_test(&tm, test);
        let on_leg = value_test(&pm, test);
        let mut binds = vec![Bind::Text(key.to_owned())];
        binds.extend(on_tx.binds);
        binds.push(Bind::Text(key.to_owned()));
        binds.extend(on_leg.binds);
        Fragment {
            sql: format!(
                "EXISTS (SELECT 1 FROM transaction_metadata {tm} \
                         WHERE {tm}.transaction_id = t.id AND {tm}.key = ? AND ({})) \
                 OR EXISTS (SELECT 1 FROM posting_metadata {pm} \
                            WHERE {pm}.posting_id = {leg}.id AND {pm}.key = ? AND ({}))",
                on_tx.sql, on_leg.sql
            ),
            binds,
            exact: on_tx.exact,
        }
    }
}

/// Sorted string ids, so the SQL text and its binds are deterministic.
fn sorted<T>(ids: &HashSet<T>) -> Vec<String>
where
    T: ToString,
{
    let mut out: Vec<String> = ids.iter().map(ToString::to_string).collect();
    out.sort();
    out
}

/// `column IN (…)`; an empty list is `0`.
fn in_list(column: &str, ids: Vec<String>) -> Fragment {
    if ids.is_empty() {
        return Fragment::exact("0", Vec::new());
    }
    Fragment::exact(
        format!("{column} IN ({})", sql_placeholders(ids.len())),
        ids.into_iter().map(Bind::Text).collect(),
    )
}

/// A text test on an ASCII-folded column; exact.
fn text(column: &str, test: &TextMatch) -> Fragment {
    match test {
        TextMatch::Contains(needle) => Fragment::exact(
            format!("{column} LIKE ? ESCAPE '\\'"),
            vec![Bind::Text(format!("%{}%", escape_like(needle)))],
        ),
        TextMatch::Equals(needle) => {
            Fragment::exact(format!("{column} = ?"), vec![Bind::Text(needle.clone())])
        }
    }
}

/// `column` (ISO dates as text) within `range`.
fn dates(column: &str, range: DateRange, exact: bool) -> Fragment {
    let mut parts: Vec<String> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();
    if let Some(from) = range.from {
        parts.push(format!("{column} >= ?"));
        binds.push(Bind::Text(from.to_string()));
    }
    if let Some(until) = range.until {
        parts.push(format!("{column} < ?"));
        binds.push(Bind::Text(until.to_string()));
    }
    let sql = if parts.is_empty() {
        "1".to_owned()
    } else {
        parts.join(" AND ")
    };
    Fragment { sql, binds, exact }
}

/// `v` moved outward by [`EPSILON`] plus [`RELATIVE`] of its magnitude, in
/// the direction `sign` (`-1.0` for a lower bound, `1.0` for an upper one).
#[expect(
    clippy::float_arithmetic,
    reason = "widening a coarse SQL bound; exactness lives in the evaluator"
)]
fn widen(v: f64, sign: f64) -> f64 {
    v + sign * (EPSILON + v.abs() * RELATIVE)
}

/// `column` (a `REAL`) within `range`, each bound widened outward; exclusive
/// bounds become inclusive and a bound `f64` cannot hold is left open.
fn real_range(column: &str, range: &NumRange) -> (Vec<String>, Vec<Bind>) {
    let mut parts: Vec<String> = Vec::new();
    let mut binds: Vec<Bind> = Vec::new();
    let lo = range
        .lo
        .and_then(|bound| bound.value.to_f64())
        .filter(|v| v.is_finite())
        .map(|v| widen(v, -1.0));
    let hi = range
        .hi
        .and_then(|bound| bound.value.to_f64())
        .filter(|v| v.is_finite())
        .map(|v| widen(v, 1.0));
    if let Some(v) = lo {
        parts.push(format!("{column} >= ?"));
        binds.push(Bind::Real(v));
    }
    if let Some(v) = hi {
        parts.push(format!("{column} <= ?"));
        binds.push(Bind::Real(v));
    }
    (parts, binds)
}

/// The built-in `amount:` test on a leg: magnitude, with elided legs admitted.
fn amount(leg: &str, pred: &AmountPred) -> Fragment {
    let (mut parts, mut binds) =
        real_range(&format!("ABS(CAST({leg}.amount AS REAL))"), &pred.range);
    if let Some(code) = &pred.commodity {
        parts.push(format!("{leg}.commodity = ?"));
        binds.push(Bind::Text(code.clone()));
    }
    let test = if parts.is_empty() {
        "1".to_owned()
    } else {
        parts.join(" AND ")
    };
    Fragment::loose(format!("{leg}.amount IS NULL OR ({test})"), binds)
}

/// A typed numeric metadata test: unflagged rows whose `value_num` is in
/// range, or NULL (a decimal `f64` cannot hold).
fn typed_real(m: &str, parts: &[String], binds: Vec<Bind>) -> Fragment {
    let test = if parts.is_empty() {
        "1".to_owned()
    } else {
        parts.join(" AND ")
    };
    Fragment::loose(
        format!("{m}.mismatched = 0 AND ({m}.value_num IS NULL OR ({test}))"),
        binds,
    )
}

/// The value test on one metadata row aliased `m`.
fn value_test(m: &str, test: &MetaTest) -> Fragment {
    match test {
        MetaTest::Exists => Fragment::exact("1", Vec::new()),
        MetaTest::Text(t) => text(&format!("lower({m}.value_text)"), t),
        MetaTest::Number(range) => {
            let (parts, binds) = real_range(&format!("{m}.value_num"), range);
            typed_real(m, &parts, binds)
        }
        MetaTest::Amount(pred) => {
            let (mut parts, mut binds) = real_range(&format!("{m}.value_num"), &pred.range);
            if let Some(code) = &pred.commodity {
                parts.push(format!("{m}.value_commodity = ?"));
                binds.push(Bind::Text(code.clone()));
            }
            typed_real(m, &parts, binds)
        }
        MetaTest::Boolean(want) => Fragment::loose(
            format!("{m}.mismatched = 0 AND {m}.value_text = ?"),
            vec![Bind::Text(want.to_string())],
        ),
        MetaTest::Date(range) => {
            let fragment = dates(&format!("{m}.value_text"), *range, false);
            Fragment::loose(
                format!("{m}.mismatched = 0 AND ({})", fragment.sql),
                fragment.binds,
            )
        }
        MetaTest::Timestamp(_) => Fragment::loose(format!("{m}.mismatched = 0"), Vec::new()),
        MetaTest::Account { ids, path, subtree } => {
            let folded = path
                .iter()
                .map(|s| s.to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(":");
            let mut parts = vec![format!("lower({m}.value_text) = ?")];
            let mut binds = vec![Bind::Text(folded.clone())];
            if *subtree {
                parts.push(format!("lower({m}.value_text) LIKE ? ESCAPE '\\'"));
                binds.push(Bind::Text(format!("{}:%", escape_like(&folded))));
            }
            let sorted_ids = sorted(ids);
            if !sorted_ids.is_empty() {
                parts.push(format!(
                    "{m}.value_account IN ({})",
                    sql_placeholders(sorted_ids.len())
                ));
                binds.extend(sorted_ids.into_iter().map(Bind::Text));
            }
            Fragment::loose(parts.join(" OR "), binds)
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::HashSet;

    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::MetaEntry;
    use bc_models::MetaKey;
    use bc_models::MetaValue;
    use bc_models::Metadata;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::TagId;
    use bc_models::TagPath;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use bc_query::parse;
    use bc_query::resolve;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    use super::*;
    use crate::search::DbCatalog;
    use crate::transaction::Service;

    /// Base queries; each also runs as `-(q)`, `any:(q)` and `-any:(q)`.
    const CORPUS: [&str; 44] = [
        "description:grocer",
        "description:=\"example grocer\"",
        "grocer",
        "account:Food",
        "account:=Expenses",
        "account:Assets",
        "tag:*",
        "tag:trip",
        "tag:=trip",
        "tag:work",
        "status:reconciled",
        "status:flagged",
        "status:unreconciled",
        "status:balanced",
        "status:unbalanced",
        "date:2026-01",
        "date:>=2026-03",
        "date:2026-02..2026-04",
        "amount:50",
        "amount:>25",
        "amount:<=10",
        "amount:>A$40",
        "amount:20..60",
        "commodity:USD",
        "commodity:AUD",
        "@payee:grocer",
        "@payee:*",
        "@receipt:=r-2",
        "@km:>1000",
        "@km:*",
        "@big:5",
        "@due:2026-05",
        "@paid:true",
        "@seen:2026-03-14",
        "@deposit:<0",
        "@owner:Assets",
        "@owner:=Assets:Bank",
        "@owner:=assets:gone",
        "any:(account:Fuel)",
        "account:Bank amount:>40",
        "account:Food or tag:trip",
        "account:Assets -any:(tag:work)",
        "-account:Bank",
        "status:flagged or amount:>1000",
    ];

    /// Queries pinned to the transactions the evaluator accepts.
    const PINNED: [(&str, &[&str]); 9] = [
        ("@big:5", &["One-sided import"]),
        ("@owner:=assets:gone", &["Gone account"]),
        ("@owner:=Assets:Bank", &["One-sided import"]),
        ("@km:>1000", &["Example Grocer"]),
        ("@km:*", &["Example Grocer", "Snack"]),
        ("@receipt:=r-2", &["Fuel and food"]),
        ("amount:30 account:Bank", &["Fuel and food"]),
        ("tag:trip", &["Fuel and food"]),
        ("status:unbalanced", &["Fuel and food", "One-sided import"]),
    ];

    /// Queries and whether each compiles exactly.
    const EXACTNESS: [(&str, bool); 15] = [
        ("description:x", true),
        ("account:Assets", true),
        ("tag:*", true),
        ("status:reconciled", true),
        ("date:2026", true),
        ("@payee:x", true),
        ("@payee:*", true),
        ("-description:x", true),
        ("any:(account:Assets)", true),
        ("amount:>5", false),
        ("commodity:AUD", false),
        ("status:balanced", false),
        ("@km:>5", false),
        ("-amount:>5", false),
        ("account:Assets or amount:>5", false),
    ];

    /// The accounts the ledger posts to.
    struct Accounts {
        /// `Assets:Bank`.
        bank: AccountId,
        /// `Assets:Cash`.
        cash: AccountId,
        /// `Expenses:Food`.
        food: AccountId,
        /// `Expenses:Fuel`.
        fuel: AccountId,
    }

    /// The tags the ledger uses.
    struct Tags {
        /// `work`.
        work: TagId,
        /// `trip:flights`.
        flights: TagId,
    }

    /// Creates an account under `parent`.
    async fn account(pool: &SqlitePool, name: &str, parent: Option<&AccountId>) -> AccountId {
        let accounts = crate::account::Service::new(pool.clone());
        let builder = accounts
            .create()
            .name(name)
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount);
        match parent {
            Some(p) => builder.parent_id(p).call().await,
            None => builder.call().await,
        }
        .expect("account")
    }

    /// Registers a commodity.
    async fn commodity(pool: &SqlitePool, code: &str, symbol: &str) {
        let c = bc_models::Commodity::builder()
            .code(code)
            .symbol(symbol)
            .name(code)
            .aliases(Vec::new())
            .decimals(2)
            .is_iso(true)
            .symbol_after(false)
            .build();
        crate::commodity::Service::new(pool.clone())
            .create(&c)
            .await
            .expect("commodity");
    }

    /// A metadata entry whose value fits its key.
    fn meta(key: &str, value: MetaValue) -> MetaEntry {
        MetaEntry::new(MetaKey::new(key).expect("key"), value)
    }

    /// A leg; `None` elides it.
    fn leg(account: &AccountId, amount: Option<(Decimal, &str)>, tags: Vec<TagId>) -> Posting {
        leg_with(account, amount, tags, Vec::new())
    }

    /// A leg carrying its own metadata; `None` elides it.
    fn leg_with(
        account: &AccountId,
        amount: Option<(Decimal, &str)>,
        tags: Vec<TagId>,
        metadata: Vec<MetaEntry>,
    ) -> Posting {
        let builder = Posting::builder()
            .id(PostingId::new())
            .account_id(account.clone())
            .tag_ids(tags)
            .metadata(Metadata::new(metadata));
        match amount {
            Some((value, code)) => builder.amount(Amount::new(value, code)).build(),
            None => builder.build(),
        }
    }

    /// A transaction.
    fn tx(
        description: &str,
        day: jiff::civil::Date,
        reconciliation: Reconciliation,
        postings: Vec<Posting>,
        tags: Vec<TagId>,
        metadata: Vec<MetaEntry>,
    ) -> Transaction {
        Transaction::builder()
            .id(TransactionId::new())
            .date(day)
            .description(description)
            .postings(postings)
            .tag_ids(tags)
            .metadata(Metadata::new(metadata))
            .reconciliation(reconciliation)
            .created_at(Timestamp::now())
            .build()
    }

    /// Creates the ledger's commodities, accounts and tags.
    async fn setup(pool: &SqlitePool) -> (Accounts, Tags) {
        commodity(pool, "AUD", "A$").await;
        commodity(pool, "USD", "$").await;
        let assets = account(pool, "Assets", None).await;
        let bank = account(pool, "Bank", Some(&assets)).await;
        let cash = account(pool, "Cash", Some(&assets)).await;
        let expenses = account(pool, "Expenses", None).await;
        let food = account(pool, "Food", Some(&expenses)).await;
        let fuel = account(pool, "Fuel", Some(&expenses)).await;
        let tags = crate::tag::Service::new(pool.clone());
        let work = tags
            .create_path(&"work".parse::<TagPath>().expect("path"))
            .await
            .expect("work");
        tags.create_path(&"trip".parse::<TagPath>().expect("path"))
            .await
            .expect("trip");
        let flights = tags
            .create_path(&"trip:flights".parse::<TagPath>().expect("path"))
            .await
            .expect("flights");
        (
            Accounts {
                bank,
                cash,
                food,
                fuel,
            },
            Tags { work, flights },
        )
    }

    /// The first three transactions: an elided leg, a multi-commodity
    /// residual with a repeated key, and a flagged one carrying every typed
    /// metadata value.
    fn early(a: &Accounts, t: &Tags) -> Vec<Transaction> {
        let seen: Timestamp = "2026-03-14T09:30:00Z".parse().expect("timestamp");
        vec![
            tx(
                "Example Grocer",
                date(2026, 1, 10),
                Reconciliation::Reconciled,
                vec![
                    leg(&a.food, Some((dec!(50), "AUD")), vec![]),
                    leg(&a.bank, None, vec![]),
                ],
                vec![t.work.clone()],
                vec![
                    meta("payee", MetaValue::Text("Example Grocer".to_owned())),
                    meta("km", MetaValue::Number(dec!(1200))),
                ],
            ),
            tx(
                "Fuel and food",
                date(2026, 2, 20),
                Reconciliation::Unreconciled,
                vec![
                    leg(&a.fuel, Some((dec!(30), "USD")), vec![t.flights.clone()]),
                    leg(&a.food, Some((dec!(20), "AUD")), vec![]),
                    leg(&a.bank, None, vec![]),
                ],
                vec![],
                vec![
                    meta("receipt", MetaValue::Text("R-1".to_owned())),
                    meta("receipt", MetaValue::Text("R-2".to_owned())),
                ],
            ),
            tx(
                "Snack",
                date(2026, 3, 14),
                Reconciliation::Flagged,
                vec![
                    leg(&a.food, Some((dec!(10), "AUD")), vec![]),
                    leg(&a.cash, Some((dec!(-10), "AUD")), vec![]),
                ],
                vec![],
                vec![
                    meta("km", MetaValue::Text("lots".to_owned())),
                    meta("due", MetaValue::Date(date(2026, 5, 1))),
                    meta("paid", MetaValue::Boolean(true)),
                    meta("seen", MetaValue::Timestamp(seen)),
                    meta("deposit", MetaValue::Amount(Amount::new(dec!(-20), "AUD"))),
                ],
            ),
        ]
    }

    /// The last three transactions: a one-sided import, an account value
    /// tombstoned below, and one whose metadata and tags sit on its legs.
    fn late(a: &Accounts, t: &Tags) -> Vec<Transaction> {
        vec![
            tx(
                "One-sided import",
                date(2026, 4, 2),
                Reconciliation::Unreconciled,
                vec![leg(&a.bank, Some((dec!(7), "AUD")), vec![])],
                vec![],
                vec![
                    meta("big", MetaValue::Number(dec!(5))),
                    meta("owner", MetaValue::Account(a.bank.clone())),
                ],
            ),
            tx(
                "Gone account",
                date(2026, 5, 9),
                Reconciliation::Unreconciled,
                vec![
                    leg(&a.food, Some((dec!(1500), "AUD")), vec![]),
                    leg(&a.cash, None, vec![]),
                ],
                vec![],
                vec![meta("owner", MetaValue::Account(a.cash.clone()))],
            ),
            tx(
                "Leg notes",
                date(2026, 6, 20),
                Reconciliation::Unreconciled,
                vec![
                    leg_with(
                        &a.food,
                        Some((dec!(12), "AUD")),
                        vec![t.work.clone()],
                        vec![
                            meta("payee", MetaValue::Text("Corner Grocer".to_owned())),
                            meta("deposit", MetaValue::Amount(Amount::new(dec!(15), "USD"))),
                            meta("paid", MetaValue::Boolean(false)),
                            meta("owner", MetaValue::Account(a.fuel.clone())),
                        ],
                    ),
                    leg_with(
                        &a.cash,
                        Some((dec!(-12), "AUD")),
                        vec![],
                        vec![meta("due", MetaValue::Date(date(2026, 5, 20)))],
                    ),
                ],
                vec![],
                vec![],
            ),
        ]
    }

    /// An invented ledger exercising every compile rule: elided legs, a
    /// multi-commodity residual, a one-sided transaction, nested tags, every
    /// metadata type on transactions and on legs, a repeated key, a
    /// mismatched value, a NULL `value_num` and a tombstoned account value.
    async fn ledger(pool: &SqlitePool) {
        let (accounts, tags) = setup(pool).await;
        let svc = Service::new(pool.clone());
        let rows = early(&accounts, &tags)
            .into_iter()
            .chain(late(&accounts, &tags));
        for row in rows {
            svc.create(row).await.expect("transaction");
        }
        // A decimal f64 cannot hold leaves `value_num` NULL; simulate it.
        sqlx::query("UPDATE transaction_metadata SET value_num = NULL WHERE key = 'big'")
            .execute(pool)
            .await
            .expect("null value_num");
        // Deleting an account tombstones the entry; simulate it.
        sqlx::query(
            "UPDATE transaction_metadata SET value_account = NULL, value_text = 'Assets:Gone' \
             WHERE key = 'owner' AND value_account = ?",
        )
        .bind(accounts.cash.to_string())
        .execute(pool)
        .await
        .expect("tombstone");
    }

    /// Every transaction, hydrated.
    async fn everything(pool: &SqlitePool) -> Vec<Transaction> {
        let rows: Vec<TxRow> = sqlx::query_as(
            "SELECT id, date, description, reconciliation, created_at FROM transactions",
        )
        .fetch_all(pool)
        .await
        .expect("rows");
        Service::new(pool.clone())
            .assemble_transactions(rows)
            .await
            .expect("hydrate")
            .collect()
    }

    /// The ids the candidate SQL returns and the ids the evaluator accepts.
    async fn both(
        pool: &SqlitePool,
        catalog: &DbCatalog,
        all: &[Transaction],
        text: &str,
    ) -> (HashSet<String>, HashSet<String>, bool) {
        let resolved = resolve(&parse(text).expect("parses"), catalog);
        let expr = resolved
            .expr
            .unwrap_or_else(|| panic!("{text}: {:?}", resolved.diagnostics));
        let matcher = Matcher::new(&expr, catalog).expect("matcher");
        let c = candidates(Some(&matcher), None, None, &[]).expect("compile");
        let ids = statement(c.sql, c.binds)
            .fetch_all(pool)
            .await
            .unwrap_or_else(|e| panic!("{text}: {e}"))
            .into_iter()
            .map(|row| row.0)
            .collect();
        let truth = all
            .iter()
            .filter(|t| !matcher.matched_postings(t).is_empty())
            .map(|t| t.id().to_string())
            .collect();
        (ids, truth, c.exact)
    }

    /// Asserts the candidates for `text` contain every true match, and equal
    /// them when the compile was exact.
    async fn check(pool: &SqlitePool, catalog: &DbCatalog, all: &[Transaction], text: &str) {
        let (ids, truth, exact) = both(pool, catalog, all, text).await;
        let missing: Vec<&String> = truth.difference(&ids).collect();
        assert!(
            missing.is_empty(),
            "{text}: SQL dropped true matches {missing:?}"
        );
        if exact {
            assert_eq!(ids, truth, "{text} compiled exact but SQL disagrees");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn candidates_contain_every_true_match(pool: SqlitePool) {
        ledger(&pool).await;
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        let all = everything(&pool).await;
        for base in CORPUS {
            for text in [
                base.to_owned(),
                format!("-({base})"),
                format!("any:({base})"),
                format!("-any:({base})"),
            ] {
                check(&pool, &catalog, &all, &text).await;
            }
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn pairs_of_queries_keep_every_true_match(pool: SqlitePool) {
        ledger(&pool).await;
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        let all = everything(&pool).await;
        for a in CORPUS {
            for b in CORPUS {
                for text in [
                    format!("({a}) -({b})"),
                    format!("-(({a}) ({b}))"),
                    format!("-(({a}) or -({b}))"),
                    format!("any:({a}) -any:(-({b}))"),
                ] {
                    check(&pool, &catalog, &all, &text).await;
                }
            }
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn pinned_matches(pool: SqlitePool) {
        ledger(&pool).await;
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        let all = everything(&pool).await;
        for (text, expected) in PINNED {
            let (_, truth, _) = both(&pool, &catalog, &all, text).await;
            let mut got: Vec<&str> = all
                .iter()
                .filter(|t| truth.contains(&t.id().to_string()))
                .map(Transaction::description)
                .collect();
            got.sort_unstable();
            assert_eq!(got, expected, "{text}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn exactness(pool: SqlitePool) {
        ledger(&pool).await;
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        for (text, exact) in EXACTNESS {
            let resolved = resolve(&parse(text).expect("parses"), &catalog);
            let expr = resolved
                .expr
                .unwrap_or_else(|| panic!("{text}: {:?}", resolved.diagnostics));
            let matcher = Matcher::new(&expr, &catalog).expect("matcher");
            let c = candidates(Some(&matcher), None, None, &[]).expect("compile");
            assert_eq!(c.exact, exact, "{text}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_negated_inexact_node_compiles_to_true(pool: SqlitePool) {
        ledger(&pool).await;
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        let resolved = resolve(&parse("-amount:>5").expect("parses"), &catalog);
        let matcher = Matcher::new(&resolved.expr.expect("resolves"), &catalog).expect("matcher");
        let c = candidates(Some(&matcher), None, None, &[]).expect("compile");
        assert!(c.sql.contains("AND (1)"), "{}", c.sql);
        assert_eq!(c.binds, Vec::new());
    }

    /// A transfer of `value` between two accounts, also recorded as metadata.
    fn transfer(a: &Accounts, description: &str, value: Decimal) -> Transaction {
        let mut back = value;
        back.set_sign_negative(true);
        tx(
            description,
            date(2026, 7, 1),
            Reconciliation::Unreconciled,
            vec![
                leg_with(
                    &a.bank,
                    Some((value, "AUD")),
                    vec![],
                    vec![meta("worth", MetaValue::Number(value))],
                ),
                leg(&a.cash, Some((back, "AUD")), vec![]),
            ],
            vec![],
            vec![],
        )
    }

    /// Each value's `Decimal::to_f64` lands two ulps from SQLite's cast of its
    /// text, on the side an absolute widening alone would cut off.
    #[sqlx::test(migrations = "./migrations")]
    async fn large_magnitudes_survive_real_rounding(pool: SqlitePool) {
        let (accounts, _) = setup(&pool).await;
        let svc = Service::new(pool.clone());
        svc.create(transfer(&accounts, "Rounds down", dec!(35193300206349.87)))
            .await
            .expect("transaction");
        svc.create(transfer(&accounts, "Rounds up", dec!(35189800994380.09)))
            .await
            .expect("transaction");
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        let all = everything(&pool).await;
        for text in [
            "amount:<=35193300206349.87",
            "amount:35193300206349.87",
            "amount:>=35189800994380.09",
            "amount:35189800994380.09",
            "@worth:<=35193300206349.87",
            "@worth:>=35189800994380.09",
        ] {
            let (ids, truth, _) = both(&pool, &catalog, &all, text).await;
            assert!(
                !truth.is_empty(),
                "{text}: the evaluator accepts a transfer"
            );
            assert_eq!(ids, truth, "{text}");
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn without_a_matcher_dates_and_scope_filter(pool: SqlitePool) {
        ledger(&pool).await;
        let catalog = DbCatalog::load(&pool).await.expect("catalog");
        let food: Vec<AccountId> = catalog
            .account_ids_by_path(&["Expenses".to_owned(), "Food".to_owned()], false)
            .into_iter()
            .map(|id| id.parse().expect("id"))
            .collect();
        let c = candidates(None, Some(date(2026, 2, 1)), Some(date(2026, 5, 9)), &food)
            .expect("compile");
        assert!(c.exact, "without a matcher the SQL is exact");
        let got: Vec<String> = statement(c.sql, c.binds)
            .fetch_all(&pool)
            .await
            .expect("rows")
            .into_iter()
            .map(|row| row.2)
            .collect();
        assert_eq!(got, vec!["Snack".to_owned(), "Fuel and food".to_owned()]);
    }
}
