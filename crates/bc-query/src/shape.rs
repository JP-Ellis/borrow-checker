//! Whole-query edits the pages make: splitting the top-level conjuncts,
//! joining and replacing terms, the budget's inert-term split and the
//! window that top-level `date` terms set.

use jiff::civil::Date;

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::ast::Op;
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

/// What the budget page sends, and what it set aside.
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
/// A parenthesised `and` inside an `and` is flattened, since `(a b) c` means
/// `a b c`.
///
/// # Arguments
///
/// * `expr` - The query.
#[must_use]
pub fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::And(items, _) => items.iter().flat_map(conjuncts).collect(),
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

/// The word a `status:word` term names.
fn status_word(expr: &Expr) -> Option<&str> {
    if !is_builtin(expr, "status") {
        return None;
    }
    let Expr::Term(term) = expr else {
        return None;
    };
    match &term.criterion {
        Criterion::Compare {
            op: Op::Match,
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

/// `base` with the top-level conjunct printed as `target` replaced in place by
/// `add`'s conjuncts, or dropped when `add` is `None`.
///
/// When no conjunct prints as `target`, `add` joins `base` by `and`. A
/// conjunct that would then appear twice keeps its first place.
///
/// # Arguments
///
/// * `base` - The current query.
/// * `target` - The conjunct's canonical text, as its chip carries it.
/// * `add` - The replacement; `None` drops the conjunct.
///
/// # Returns
///
/// What results; `None` when nothing remains.
#[must_use]
pub fn replace_conjunct(base: &Expr, target: &str, add: Option<Expr>) -> Option<Expr> {
    let items = conjuncts(base);
    let Some(at) = items.iter().position(|item| print(item) == target) else {
        return Some(match add {
            Some(expr) => and_onto(Some(base), expr),
            None => base.clone(),
        });
    };
    let replacement: Vec<Expr> = add.as_ref().map_or_else(Vec::new, |expr| {
        conjuncts(expr).into_iter().cloned().collect()
    });
    let mut spliced: Vec<Expr> = Vec::with_capacity(items.len().saturating_add(replacement.len()));
    for (index, item) in items.into_iter().enumerate() {
        if index == at {
            spliced.extend(replacement.iter().cloned());
        } else {
            spliced.push(item.clone());
        }
    }
    let mut seen: Vec<String> = Vec::new();
    spliced.retain(|item| {
        let text = print(item);
        if seen.contains(&text) {
            false
        } else {
            seen.push(text);
            true
        }
    });
    conjoin(spliced)
}

/// Splits off the conjuncts the budget page ignores: top-level `date` and
/// balance-`status` terms. Such terms nested in `or`, `-` or
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
    #[case("a (b or c) d", "b or c", Some("x"), Some("a x d"))]
    #[case("a (b or c) d", "b or c", Some("x y"), Some("a x y d"))]
    #[case("a (b or c) d", "d", Some("p or q"), Some("a (b or c) (p or q)"))]
    #[case("a (b or c) d", "d", None, Some("a (b or c)"))]
    #[case("a (b or c) d", "zz", Some("x"), Some("a (b or c) d x"))]
    #[case("a (b or c) d", "zz", None, Some("a (b or c) d"))]
    #[case("a (b or c) d", "d", Some("a"), Some("a (b or c)"))]
    #[case("a", "a", None, None)]
    #[case("a or b", "a or b", Some("c"), Some("c"))]
    fn replaces_a_conjunct_in_place(
        #[case] base: &str,
        #[case] target: &str,
        #[case] add: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let got = replace_conjunct(
            &parse(base).expect("base parses"),
            target,
            add.map(|text| parse(text).expect("add parses")),
        );
        assert_eq!(got.as_ref().map(print).as_deref(), expected);
    }

    #[rstest]
    #[case("a b", vec!["a", "b"])]
    #[case("a or b", vec!["a or b"])]
    #[case("(a or b) c", vec!["a or b", "c"])]
    #[case("-a", vec!["-a"])]
    #[case("(a b) c", vec!["a", "b", "c"])]
    #[case("a (b (c d))", vec!["a", "b", "c", "d"])]
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
    #[case("status:balanced", true)]
    #[case("STATUS:Unbalanced", true)]
    #[case("status:=unbalanced", false)]
    #[case("status:reconciled", false)]
    #[case("@status:balanced", false)]
    #[case("status:*", false)]
    fn balance_terms(#[case] text: &str, #[case] expected: bool) {
        assert_eq!(is_balance_term(&q(text)), expected);
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
    #[case("(date:2026-03 rent) x", "rent x", vec!["date:2026-03"], false)]
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
}
