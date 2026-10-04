//! Whole-query edits the pages make: splitting the top-level conjuncts,
//! joining and replacing terms, the budget's inert-term split and the
//! window that top-level `date` terms set.

use jiff::civil::Date;

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::ast::Field;
use crate::ast::Op;
use crate::ast::Term;
use crate::ast::Value;
use crate::catalog::Catalog;
use crate::catalog::MetaKey;
use crate::catalog::PathEntry;
use crate::currency::Commodity;
use crate::filter::DateRange;
use crate::filter::Pred;
use crate::filter::ResolvedExpr;
use crate::printer::print;
use crate::resolve::resolve;
use crate::span::Span;

/// The `status:` words that test balance; every other status word tests reconciliation.
const BALANCE_WORDS: [&str; 2] = ["balanced", "unbalanced"];

/// A query split into the conjuncts kept and the ones dropped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Stripped {
    /// What remains; `None` when every conjunct was dropped.
    pub kept: Option<Expr>,
    /// The dropped conjuncts, in query order.
    pub removed: Vec<Expr>,
}

impl Stripped {
    /// Creates a split.
    ///
    /// # Arguments
    ///
    /// * `kept` - What remains.
    /// * `removed` - The dropped conjuncts.
    #[must_use]
    pub const fn new(kept: Option<Expr>, removed: Vec<Expr>) -> Self {
        Self { kept, removed }
    }
}

/// What the budget page sends, and what it set aside (spec §3, IPC).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct BudgetQuery {
    /// The query without its top-level `date` and balance-`status` conjuncts.
    pub kept: Option<Expr>,
    /// The stripped conjuncts, printed canonically.
    pub stripped: Vec<String>,
    /// Whether a `date` or balance-`status` term remains nested in `kept`.
    pub nested: bool,
}

impl BudgetQuery {
    /// Creates a budget split.
    ///
    /// # Arguments
    ///
    /// * `kept` - The query the budget evaluates.
    /// * `stripped` - The stripped conjuncts, printed.
    /// * `nested` - Whether an inert term remains nested.
    #[must_use]
    pub const fn new(kept: Option<Expr>, stripped: Vec<String>, nested: bool) -> Self {
        Self {
            kept,
            stripped,
            nested,
        }
    }
}

/// A catalog with nothing in it: `date` terms resolve without one.
struct NoCatalog;

impl Catalog for NoCatalog {
    type Commodity = Commodity;

    fn accounts(&self) -> &[PathEntry] {
        &[]
    }

    fn tags(&self) -> &[PathEntry] {
        &[]
    }

    fn commodities(&self) -> &[Commodity] {
        &[]
    }

    fn meta_keys(&self) -> &[MetaKey] {
        &[]
    }
}

/// The top-level conjuncts of `expr`: the items of an `and`, else `expr` itself.
///
/// # Arguments
///
/// * `expr` - The query.
#[must_use]
pub fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::And(items, _) => items.iter().collect(),
        Expr::Or(..) | Expr::Not(..) | Expr::Term(_) | Expr::Word(_) => vec![expr],
    }
}

/// Joins `items` by `and`, flattening nested `and`s.
///
/// # Arguments
///
/// * `items` - The conjuncts.
///
/// # Returns
///
/// `None` when `items` is empty, the item itself when there is one.
#[must_use]
pub fn conjoin(items: Vec<Expr>) -> Option<Expr> {
    let mut flat: Vec<Expr> = Vec::with_capacity(items.len());
    for item in items {
        match item {
            Expr::And(inner, _) => flat.extend(inner),
            other @ (Expr::Or(..) | Expr::Not(..) | Expr::Term(_) | Expr::Word(_)) => {
                flat.push(other);
            }
        }
    }
    if flat.len() > 1 {
        return Some(Expr::And(flat, Span::default()));
    }
    flat.pop()
}

/// The term `field:<op>value` on a built-in field, with empty spans.
///
/// # Arguments
///
/// * `field` - The built-in field name.
/// * `op` - The operator.
/// * `value` - The value, unquoted; [`print()`] quotes it when needed.
#[must_use]
pub fn builtin_term(field: &str, op: Op, value: &str) -> Expr {
    let span = Span::default();
    Expr::Term(Term::new(
        Field::new(field, false, span),
        Criterion::Compare {
            op,
            value: Value::new(value, span),
            span,
        },
        span,
    ))
}

/// Whether `expr` is a term on the built-in `field`, named in any case.
///
/// # Arguments
///
/// * `expr` - The expression.
/// * `field` - The built-in field name.
#[must_use]
pub fn is_builtin(expr: &Expr, field: &str) -> bool {
    matches!(expr, Expr::Term(term) if !term.field.meta && term.field.name.eq_ignore_ascii_case(field))
}

/// Whether `expr` is `field:<op>value` on the built-in `field`.
///
/// # Arguments
///
/// * `expr` - The expression.
/// * `field` - The built-in field name.
/// * `op` - The operator the term must use.
#[must_use]
pub fn is_builtin_with(expr: &Expr, field: &str, op: Op) -> bool {
    is_builtin(expr, field)
        && matches!(expr, Expr::Term(term) if matches!(&term.criterion, Criterion::Compare { op: written, .. } if *written == op))
}

/// The word a `status:word` or `status:=word` term names.
fn status_word(expr: &Expr) -> Option<&str> {
    if !is_builtin(expr, "status") {
        return None;
    }
    let Expr::Term(term) = expr else {
        return None;
    };
    match &term.criterion {
        Criterion::Compare {
            op: Op::Match | Op::Equal,
            value,
            ..
        } => Some(value.text.as_str()),
        Criterion::Compare { .. }
        | Criterion::Any(_)
        | Criterion::Range { .. }
        | Criterion::Group(..) => None,
    }
}

/// Whether `expr` is a `status:` term naming a balance state.
///
/// # Arguments
///
/// * `expr` - The expression.
#[must_use]
pub fn is_balance_term(expr: &Expr) -> bool {
    status_word(expr).is_some_and(|word| BALANCE_WORDS.iter().any(|b| b.eq_ignore_ascii_case(word)))
}

/// Whether `expr` is a `status:` term naming a reconciliation state.
///
/// # Arguments
///
/// * `expr` - The expression.
#[must_use]
pub fn is_reconciliation_term(expr: &Expr) -> bool {
    status_word(expr)
        .is_some_and(|word| !BALANCE_WORDS.iter().any(|b| b.eq_ignore_ascii_case(word)))
}

/// Whether `test` holds for `expr` or anything nested in it, `any:(…)` included.
fn contains(expr: &Expr, test: &dyn Fn(&Expr) -> bool) -> bool {
    if test(expr) {
        return true;
    }
    match expr {
        Expr::Or(items, _) | Expr::And(items, _) => items.iter().any(|item| contains(item, test)),
        Expr::Not(inner, _) => contains(inner, test),
        Expr::Term(term) => match &term.criterion {
            Criterion::Group(inner, _) => contains(inner, test),
            Criterion::Any(_) | Criterion::Compare { .. } | Criterion::Range { .. } => false,
        },
        Expr::Word(_) => false,
    }
}

/// Whether a term on the built-in `field` appears anywhere in `expr`.
///
/// # Arguments
///
/// * `expr` - The query.
/// * `field` - The built-in field name.
#[must_use]
pub fn mentions_builtin(expr: &Expr, field: &str) -> bool {
    contains(expr, &|e: &Expr| is_builtin(e, field))
}

/// Whether a balance `status:` term appears anywhere in `expr`.
///
/// # Arguments
///
/// * `expr` - The query.
#[must_use]
pub fn mentions_balance(expr: &Expr) -> bool {
    contains(expr, &is_balance_term)
}

/// Splits off the top-level conjuncts for which `drop` holds.
///
/// # Arguments
///
/// * `expr` - The query.
/// * `drop` - Selects the conjuncts to remove.
#[must_use]
pub fn strip(expr: &Expr, drop: impl Fn(&Expr) -> bool) -> Stripped {
    let (removed, kept): (Vec<&Expr>, Vec<&Expr>) = conjuncts(expr)
        .into_iter()
        .partition(|conjunct| drop(conjunct));
    Stripped::new(
        conjoin(kept.into_iter().cloned().collect()),
        removed.into_iter().cloned().collect(),
    )
}

/// `base` with `add`'s conjuncts joined on by `and`, skipping any already present.
///
/// An `or` in `add` stays one conjunct; it prints parenthesised only when
/// joined with other conjuncts.
///
/// # Arguments
///
/// * `base` - The current query, if any.
/// * `add` - The expression to join.
#[must_use]
pub fn and_onto(base: Option<&Expr>, add: Expr) -> Expr {
    let mut items: Vec<Expr> =
        base.map_or_else(Vec::new, |b| conjuncts(b).into_iter().cloned().collect());
    for item in conjuncts(&add) {
        let text = print(item);
        if !items.iter().any(|existing| print(existing) == text) {
            items.push(item.clone());
        }
    }
    conjoin(items).unwrap_or(add)
}

/// `base` without the top-level conjuncts `drop` selects, with `add` joined on.
///
/// # Arguments
///
/// * `base` - The current query, if any.
/// * `drop` - Selects the conjuncts `add` replaces.
/// * `add` - The replacement.
#[must_use]
pub fn replace(base: Option<&Expr>, drop: impl Fn(&Expr) -> bool, add: Expr) -> Expr {
    let kept = base.and_then(|b| strip(b, drop).kept);
    and_onto(kept.as_ref(), add)
}

/// `base` with `add` offered as one more alternative of the first top-level
/// conjunct made only of `field` terms, or joined by `and` when none is.
///
/// # Arguments
///
/// * `base` - The current query, if any.
/// * `field` - The built-in field whose conjunct takes the alternative.
/// * `add` - The new alternative.
#[must_use]
pub fn or_merge(base: Option<&Expr>, field: &str, add: Expr) -> Expr {
    let Some(current) = base else {
        return add;
    };
    let only_field = |e: &Expr| match e {
        Expr::Or(items, _) => items.iter().all(|item| is_builtin(item, field)),
        Expr::Term(_) => is_builtin(e, field),
        Expr::And(..) | Expr::Not(..) | Expr::Word(_) => false,
    };
    let add_text = print(&add);
    let mut merged = false;
    let mut items: Vec<Expr> = Vec::new();
    for conjunct in conjuncts(current) {
        if merged || !only_field(conjunct) {
            items.push(conjunct.clone());
            continue;
        }
        merged = true;
        let mut alternatives: Vec<Expr> = match conjunct {
            Expr::Or(alts, _) => alts.clone(),
            Expr::And(..) | Expr::Not(..) | Expr::Term(_) | Expr::Word(_) => {
                vec![conjunct.clone()]
            }
        };
        if !alternatives.iter().any(|alt| print(alt) == add_text) {
            alternatives.push(add.clone());
        }
        items.push(if alternatives.len() > 1 {
            Expr::Or(alternatives, Span::default())
        } else {
            conjunct.clone()
        });
    }
    if !merged {
        return and_onto(Some(current), add);
    }
    conjoin(items).unwrap_or(add)
}

/// `base` without the top-level conjunct whose canonical text is `target`.
///
/// # Arguments
///
/// * `base` - The current query.
/// * `target` - The conjunct's printed text, as a chip carries it.
///
/// # Returns
///
/// What remains; `None` when nothing does.
#[must_use]
pub fn remove(base: &Expr, target: &str) -> Option<Expr> {
    strip(base, |conjunct| print(conjunct) == target).kept
}

/// Splits off the conjuncts the budget page ignores: top-level `date` and
/// balance-`status` terms (spec §3, IPC). Such terms nested in `or`, `-` or
/// `any:(…)` stay, and `nested` reports them.
///
/// # Arguments
///
/// * `expr` - The query.
#[must_use]
pub fn budget_query(expr: &Expr) -> BudgetQuery {
    let split = strip(expr, |c| is_builtin(c, "date") || is_balance_term(c));
    let nested = split
        .kept
        .as_ref()
        .is_some_and(|kept| mentions_builtin(kept, "date") || mentions_balance(kept));
    BudgetQuery::new(
        split.kept,
        split.removed.iter().map(print).collect(),
        nested,
    )
}

/// The later of two optional lower bounds; `None` is unbounded.
fn later(a: Option<Date>, b: Option<Date>) -> Option<Date> {
    a.max(b)
}

/// The earlier of two optional upper bounds; `None` is unbounded.
fn earlier(a: Option<Date>, b: Option<Date>) -> Option<Date> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (Some(x), None) => Some(x),
        (None, y) => y,
    }
}

/// The dates every top-level `date` conjunct allows: the intersection of their
/// periods. Terms that fail to resolve, and `date` terms nested in `or`, `-` or
/// `any:(…)`, set no bound.
///
/// # Arguments
///
/// * `expr` - The query.
#[must_use]
pub fn date_window(expr: &Expr) -> DateRange {
    let mut from = None;
    let mut until = None;
    for conjunct in conjuncts(expr) {
        if !is_builtin(conjunct, "date") {
            continue;
        }
        let Some(ResolvedExpr::Pred(Pred::Date(range))) = resolve(conjunct, &NoCatalog).expr else {
            continue;
        };
        from = later(from, range.from);
        until = earlier(until, range.until);
    }
    DateRange::new(from, until)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::parse;

    /// Parses `text`, which every case writes valid.
    fn q(text: &str) -> Expr {
        parse(text).expect("query parses")
    }

    /// Prints an optional expression; `None` prints as the empty string.
    fn shown(expr: Option<&Expr>) -> String {
        expr.map(print).unwrap_or_default()
    }

    #[rstest]
    #[case("a b", vec!["a", "b"])]
    #[case("a or b", vec!["a or b"])]
    #[case("(a or b) c", vec!["a or b", "c"])]
    #[case("-a", vec!["-a"])]
    fn conjuncts_split_the_top_level_and(#[case] text: &str, #[case] expected: Vec<&str>) {
        let expr = q(text);
        let got: Vec<String> = conjuncts(&expr).into_iter().map(print).collect();
        let want: Vec<String> = expected.into_iter().map(str::to_owned).collect();
        assert_eq!(got, want);
    }

    #[rstest]
    #[case(None, "tag:x", "tag:x")]
    #[case(Some("a"), "b", "a b")]
    #[case(Some("a b"), "a", "a b")]
    #[case(Some("a"), "x or y", "a (x or y)")]
    #[case(Some("a"), "c d", "a c d")]
    fn and_onto_joins_and_dedupes(
        #[case] base: Option<&str>,
        #[case] add: &str,
        #[case] expected: &str,
    ) {
        let parsed = base.map(q);
        assert_eq!(print(&and_onto(parsed.as_ref(), q(add))), expected);
    }

    #[rstest]
    #[case(None, "account:A", "account:A")]
    #[case(Some("account:A"), "account:B", "account:A or account:B")]
    #[case(Some("tag:x account:A"), "account:B", "tag:x (account:A or account:B)")]
    #[case(
        Some("(account:A or account:B) tag:x"),
        "account:C",
        "(account:A or account:B or account:C) tag:x"
    )]
    #[case(Some("tag:x"), "account:A", "tag:x account:A")]
    #[case(Some("account:A"), "account:A", "account:A")]
    #[case(Some("-account:A"), "account:B", "-account:A account:B")]
    fn or_merge_offers_an_alternative(
        #[case] base: Option<&str>,
        #[case] add: &str,
        #[case] expected: &str,
    ) {
        let parsed = base.map(q);
        assert_eq!(
            print(&or_merge(parsed.as_ref(), "account", q(add))),
            expected
        );
    }

    #[test]
    fn replace_drops_the_same_family_and_appends() {
        let reconciled = q("status:reconciled amount:>=5");
        let out = replace(
            Some(&reconciled),
            is_reconciliation_term,
            q("status:flagged"),
        );
        assert_eq!(print(&out), "amount:>=5 status:flagged");
        let unbalanced = q("status:unbalanced status:reconciled");
        let swapped = replace(Some(&unbalanced), is_balance_term, q("status:balanced"));
        assert_eq!(print(&swapped), "status:reconciled status:balanced");
    }

    #[test]
    fn replace_by_operator_keeps_other_operators() {
        let base = q("date:>=2026-01-01 date:<2026-06-01");
        let out = replace(
            Some(&base),
            |c| is_builtin_with(c, "date", Op::Ge),
            q("date:>=2026-02-01"),
        );
        assert_eq!(print(&out), "date:<2026-06-01 date:>=2026-02-01");
    }

    #[rstest]
    #[case("a b c", "b", "a c")]
    #[case("a", "a", "")]
    #[case("(x or y) z", "x or y", "z")]
    #[case("a b", "nope", "a b")]
    fn remove_drops_one_conjunct(#[case] base: &str, #[case] target: &str, #[case] expected: &str) {
        assert_eq!(shown(remove(&q(base), target).as_ref()), expected);
    }

    #[rstest]
    #[case("x or date:2026", true)]
    #[case("any:(date:2026)", true)]
    #[case("-date:2026", true)]
    #[case("DATE:2026", true)]
    #[case("@date:2026", false)]
    #[case("coffee", false)]
    fn mentions_builtin_looks_everywhere(#[case] text: &str, #[case] expected: bool) {
        assert_eq!(mentions_builtin(&q(text), "date"), expected);
    }

    #[rstest]
    #[case("status:balanced", true, false)]
    #[case("STATUS:=Unbalanced", true, false)]
    #[case("status:reconciled", false, true)]
    #[case("@status:balanced", false, false)]
    #[case("status:*", false, false)]
    fn status_families(#[case] text: &str, #[case] balance: bool, #[case] reconciliation: bool) {
        let expr = q(text);
        assert_eq!(
            (is_balance_term(&expr), is_reconciliation_term(&expr)),
            (balance, reconciliation)
        );
    }

    #[rstest]
    #[case(
        "date:2026 coffee status:unbalanced",
        "coffee",
        vec!["date:2026", "status:unbalanced"],
        false
    )]
    #[case("coffee or date:2026", "coffee or date:2026", vec![], true)]
    #[case("date:2026", "", vec!["date:2026"], false)]
    #[case("status:reconciled", "status:reconciled", vec![], false)]
    #[case("-status:balanced x", "-status:balanced x", vec![], true)]
    #[case("any:(date:2026) x", "any:(date:2026) x", vec![], true)]
    fn budget_query_strips_top_level_inert_terms(
        #[case] text: &str,
        #[case] kept: &str,
        #[case] stripped: Vec<&str>,
        #[case] nested: bool,
    ) {
        let split = budget_query(&q(text));
        assert_eq!(shown(split.kept.as_ref()), kept);
        assert_eq!(
            split.stripped,
            stripped.into_iter().map(str::to_owned).collect::<Vec<_>>()
        );
        assert_eq!(split.nested, nested);
    }

    #[rstest]
    #[case("date:2026-03", Some(date(2026, 3, 1)), Some(date(2026, 4, 1)))]
    #[case(
        "date:>=2026-01-15 date:<2026-02-01 x",
        Some(date(2026, 1, 15)),
        Some(date(2026, 2, 1))
    )]
    #[case(
        "date:2026 date:2026-06",
        Some(date(2026, 6, 1)),
        Some(date(2026, 7, 1))
    )]
    #[case("x or date:2026", None, None)]
    #[case("-date:2026", None, None)]
    #[case("date:nonsense", None, None)]
    #[case("x", None, None)]
    fn date_window_intersects_top_level_date_terms(
        #[case] text: &str,
        #[case] from: Option<Date>,
        #[case] until: Option<Date>,
    ) {
        assert_eq!(date_window(&q(text)), DateRange::new(from, until));
    }

    #[test]
    fn builtin_term_quotes_values_that_need_it() {
        assert_eq!(
            print(&builtin_term("account", Op::Match, "Assets:Smart Access")),
            "account:\"Assets:Smart Access\""
        );
        assert_eq!(
            print(&builtin_term("amount", Op::Ge, "USD300")),
            "amount:>=USD300"
        );
    }
}
