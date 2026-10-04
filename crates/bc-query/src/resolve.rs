//! Types each term of a parsed [`Expr`] against a [`Catalog`].

use bc_expr::evaluate;
use bc_expr::is_literal;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::ast::Op;
use crate::ast::Term;
use crate::ast::Value;
use crate::catalog::Catalog;
use crate::catalog::PathEntry;
use crate::currency::MarkerError;
use crate::currency::resolve_marker;
use crate::currency::split_marked_amount;
use crate::filter::AmountPred;
use crate::filter::Bound;
use crate::filter::DateRange;
use crate::filter::NumRange;
use crate::filter::Pred;
use crate::filter::ResolvedExpr;
use crate::filter::Status;
use crate::filter::TagPred;
use crate::filter::TextMatch;
use crate::path;
use crate::period;
use crate::printer::print;
use crate::span::Span;

/// The built-in field names, for "did you mean".
const BUILTINS: [&str; 8] = [
    "description",
    "account",
    "tag",
    "status",
    "date",
    "amount",
    "commodity",
    "any",
];

/// How serious a [`Diagnostic`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "every consumer is in this workspace and matches Severity exhaustively"
)]
pub enum Severity {
    /// The query cannot run; the palette blocks commit.
    Error,
    /// The query runs but likely not as meant.
    Warning,
    /// The query runs as written; another form may be meant.
    Hint,
}

/// One message about a span of the query.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    /// How serious it is.
    pub severity: Severity,
    /// The message for the hint line.
    pub message: String,
    /// The text it concerns.
    pub span: Span,
}

impl Diagnostic {
    /// Creates a diagnostic.
    ///
    /// # Arguments
    ///
    /// * `severity` - How serious it is.
    /// * `message` - The message for the hint line.
    /// * `span` - The text it concerns.
    #[must_use]
    pub fn new(severity: Severity, message: impl Into<String>, span: Span) -> Self {
        Self {
            severity,
            message: message.into(),
            span,
        }
    }
}

/// The outcome of [`resolve()`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Resolved {
    /// The typed expression; `None` when any diagnostic is an error.
    pub expr: Option<ResolvedExpr>,
    /// Every error, warning and hint, in the order found.
    pub diagnostics: Vec<Diagnostic>,
}

impl Resolved {
    /// Creates an outcome, dropping `expr` when any diagnostic is an error.
    ///
    /// # Arguments
    ///
    /// * `expr` - The typed expression, if one was built.
    /// * `diagnostics` - Every error, warning and hint.
    #[must_use]
    pub fn new(expr: Option<ResolvedExpr>, diagnostics: Vec<Diagnostic>) -> Self {
        let mut resolved = Self {
            expr: None,
            diagnostics,
        };
        if !resolved.has_errors() {
            resolved.expr = expr;
        }
        resolved
    }

    /// Whether any diagnostic is an error.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// Types every term of `expr` against `catalog`.
///
/// Resolution continues past an error so every problem is reported at once.
///
/// # Arguments
///
/// * `expr` - The parsed query.
/// * `catalog` - The ledger facts to resolve against.
#[must_use]
pub fn resolve<C>(expr: &Expr, catalog: &C) -> Resolved
where
    C: Catalog,
{
    let mut resolver = Resolver {
        catalog,
        diagnostics: Vec::new(),
        in_any: false,
    };
    let typed = resolver.expr(expr);
    Resolved::new(typed, resolver.diagnostics)
}

/// Whether an amount keeps its sign.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Sign {
    /// A leg amount: compared by magnitude.
    Magnitude,
    /// A metadata amount or number: compared signed.
    #[expect(dead_code, reason = "consumed by metadata keys")]
    Signed,
}

/// Resolution state.
struct Resolver<'c, C> {
    /// The ledger facts.
    catalog: &'c C,
    /// Diagnostics so far.
    diagnostics: Vec<Diagnostic>,
    /// Whether the cursor is inside `any:(…)`.
    in_any: bool,
}

/// Wraps a predicate.
const fn pred(p: Pred) -> ResolvedExpr {
    ResolvedExpr::Pred(p)
}

/// ASCII-folds text, matching SQLite's `lower()`.
fn fold(text: &str) -> String {
    text.to_ascii_lowercase()
}

/// The field as written in messages: `account:` or `@payee:`.
fn label(term: &Term) -> String {
    let sigil = if term.field.meta { "@" } else { "" };
    format!("{sigil}{}:", term.field.name.to_ascii_lowercase())
}

/// Names the criterion form a field rejected.
fn rejection(term: &Term) -> String {
    let field = label(term);
    match &term.criterion {
        Criterion::Any(_) => format!("'{field}' has no '*'"),
        Criterion::Compare { op, .. } => format!("'{field}' has no '{}'", op.as_str()),
        Criterion::Range { .. } => format!("'{field}' has no range"),
        Criterion::Group(..) => format!("'{field}' takes no sub-expression"),
    }
}

/// The message for a path with an empty segment.
fn empty_segment(text: &str) -> String {
    format!("'{text}' has an empty segment")
}

/// The message for a path several entries end with, listed in order.
fn ambiguous_path(text: &str, found: &[&PathEntry]) -> String {
    let mut names: Vec<String> = found.iter().map(|e| e.display()).collect();
    names.sort();
    format!("'{text}' is ambiguous: {}", names.join(", "))
}

/// The message for a currency marker that names no commodity.
fn unknown_currency(marker: &str) -> String {
    format!("unknown currency '{marker}'")
}

/// The message for a currency marker that names several commodities.
fn ambiguous_currency(marker: &str) -> String {
    format!("'{marker}' matches more than one currency; use its code")
}

/// The edit distance between `a` and `b`.
fn distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b_chars.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut current = Vec::with_capacity(previous.len());
        current.push(i.saturating_add(1));
        for (j, cb) in b_chars.iter().enumerate() {
            let substitute = previous
                .get(j)
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(usize::from(ca != *cb));
            let delete = previous
                .get(j.saturating_add(1))
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            let insert = current
                .last()
                .copied()
                .unwrap_or(usize::MAX)
                .saturating_add(1);
            current.push(substitute.min(delete).min(insert));
        }
        previous = current;
    }
    previous.last().copied().unwrap_or_default()
}

/// The candidate within edit distance 2 of `word`, closest first.
fn suggest<'a>(word: &str, candidates: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .map(|c| (distance(word, c), c))
        .filter(|(d, _)| *d <= 2)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// A one-value comparison as a numeric range.
fn compare_range(op: Op, value: Decimal) -> NumRange {
    let at = |inclusive| Some(Bound::new(value, inclusive));
    match op {
        Op::Match | Op::Equal => NumRange::new(at(true), at(true)),
        Op::Gt => NumRange::new(at(false), None),
        Op::Ge => NumRange::new(at(true), None),
        Op::Lt => NumRange::new(None, at(false)),
        Op::Le => NumRange::new(None, at(true)),
    }
}

/// Parses a status word without reporting.
fn status_word(text: &str) -> Option<Status> {
    match fold(text).as_str() {
        "unreconciled" => Some(Status::Unreconciled),
        "flagged" => Some(Status::Flagged),
        "reconciled" => Some(Status::Reconciled),
        "balanced" => Some(Status::Balanced),
        "unbalanced" => Some(Status::Unbalanced),
        _ => None,
    }
}

impl<C> Resolver<'_, C>
where
    C: Catalog,
{
    /// Records a diagnostic.
    fn push(&mut self, severity: Severity, message: impl Into<String>, span: Span) {
        self.diagnostics
            .push(Diagnostic::new(severity, message, span));
    }

    /// Records an error and yields `None`.
    fn fail<T>(&mut self, message: impl Into<String>, span: Span) -> Option<T> {
        self.push(Severity::Error, message, span);
        None
    }

    /// Records that `term`'s field rejects its criterion form.
    fn reject<T>(&mut self, term: &Term) -> Option<T> {
        self.fail(rejection(term), term.criterion.span())
    }

    /// Resolves an expression.
    fn expr(&mut self, expr: &Expr) -> Option<ResolvedExpr> {
        match expr {
            Expr::Or(items, _) => self.all(items).map(ResolvedExpr::Or),
            Expr::And(items, _) => {
                self.check_status_conflicts(items);
                self.all(items).map(ResolvedExpr::And)
            }
            Expr::Not(inner, span) => {
                self.hint_negated_account(inner, *span);
                self.expr(inner).map(|r| ResolvedExpr::Not(Box::new(r)))
            }
            Expr::Term(term) => self.term(term),
            Expr::Word(value) => Some(pred(Pred::Description(TextMatch::Contains(fold(
                &value.text,
            ))))),
        }
    }

    /// Resolves every item, so each reports its own errors.
    fn all(&mut self, items: &[Expr]) -> Option<Vec<ResolvedExpr>> {
        let resolved: Vec<Option<ResolvedExpr>> =
            items.iter().map(|item| self.expr(item)).collect();
        resolved.into_iter().collect()
    }

    /// Resolves a `field:criterion` term.
    fn term(&mut self, term: &Term) -> Option<ResolvedExpr> {
        if term.field.meta {
            return self.fail("metadata keys are not resolved yet", term.field.span);
        }
        match term.field.name.to_ascii_lowercase().as_str() {
            "description" => self.text(term).map(|m| pred(Pred::Description(m))),
            "account" => self.account(term),
            "tag" => self.tag(term),
            "status" => self.status(term).map(|s| pred(Pred::Status(s))),
            "date" => self.date(term).map(|r| pred(Pred::Date(r))),
            "amount" => self
                .amount(term, Sign::Magnitude)
                .map(|a| pred(Pred::Amount(a))),
            "commodity" => self.commodity(term),
            "any" => self.any(term),
            other => {
                let message = match suggest(other, BUILTINS.into_iter()) {
                    Some(s) => format!("unknown field '{other}' (did you mean '{s}'?)"),
                    None => format!("unknown field '{other}'"),
                };
                self.fail(message, term.field.span)
            }
        }
    }

    /// A text field: substring, whole value, or a range read as literal text.
    fn text(&mut self, term: &Term) -> Option<TextMatch> {
        match &term.criterion {
            Criterion::Compare {
                op: Op::Match,
                value,
                ..
            } => Some(TextMatch::Contains(fold(&value.text))),
            Criterion::Compare {
                op: Op::Equal,
                value,
                ..
            } => Some(TextMatch::Equals(fold(&value.text))),
            Criterion::Range { lo, hi, .. } => {
                let lo_text = lo.as_ref().map_or("", |v| v.text.as_str());
                let hi_text = hi.as_ref().map_or("", |v| v.text.as_str());
                Some(TextMatch::Contains(fold(&format!("{lo_text}..{hi_text}"))))
            }
            Criterion::Compare { .. } | Criterion::Any(_) | Criterion::Group(..) => {
                self.reject(term)
            }
        }
    }

    /// A path field's value and whether descendants match.
    fn path_value<'t>(&mut self, term: &'t Term) -> Option<(&'t Value, bool)> {
        match &term.criterion {
            Criterion::Compare {
                op: Op::Match,
                value,
                ..
            } => Some((value, true)),
            Criterion::Compare {
                op: Op::Equal,
                value,
                ..
            } => Some((value, false)),
            Criterion::Compare { .. }
            | Criterion::Any(_)
            | Criterion::Range { .. }
            | Criterion::Group(..) => self.reject(term),
        }
    }

    /// Looks a path up in `entries`, reporting a missing or ambiguous one.
    fn lookup<'e>(
        &mut self,
        entries: &'e [PathEntry],
        value: &Value,
        noun: &str,
    ) -> Option<&'e PathEntry> {
        let Some(segments) = path::split(&value.text) else {
            return self.fail(empty_segment(&value.text), value.span);
        };
        match path::resolve(entries, &segments) {
            path::Match::One(entry) => Some(entry),
            path::Match::Missing => {
                self.fail(format!("no {noun} matches '{}'", value.text), value.span)
            }
            path::Match::Many(found) => self.fail(ambiguous_path(&value.text, &found), value.span),
        }
    }

    /// `account:`.
    fn account(&mut self, term: &Term) -> Option<ResolvedExpr> {
        let catalog = self.catalog;
        let (value, subtree) = self.path_value(term)?;
        let entry = self.lookup(catalog.accounts(), value, "account")?;
        Some(pred(Pred::Account {
            id: entry.id.clone(),
            subtree,
        }))
    }

    /// `tag:`.
    fn tag(&mut self, term: &Term) -> Option<ResolvedExpr> {
        if let Criterion::Any(_) = term.criterion {
            return Some(pred(Pred::Tag(TagPred::Any)));
        }
        let catalog = self.catalog;
        let (value, subtree) = self.path_value(term)?;
        let entry = self.lookup(catalog.tags(), value, "tag")?;
        Some(pred(Pred::Tag(TagPred::Tag {
            id: entry.id.clone(),
            subtree,
        })))
    }

    /// `status:`.
    fn status(&mut self, term: &Term) -> Option<Status> {
        let Criterion::Compare {
            op: Op::Match,
            value,
            ..
        } = &term.criterion
        else {
            return self.reject(term);
        };
        match status_word(&value.text) {
            Some(status) => Some(status),
            None => self.fail(
                "'status:' takes unreconciled, flagged, reconciled, balanced or unbalanced",
                value.span,
            ),
        }
    }

    /// Reads a calendar period as `[start, end)`; `end` is `None` when the
    /// period runs to the calendar's last day.
    fn period(&mut self, value: &Value) -> Option<(Date, Option<Date>)> {
        match period::parse(&value.text) {
            Some(p) => Some((p.start, p.end)),
            None => self.fail(
                format!(
                    "'{}' is not a date; use YYYY, YYYY-MM or YYYY-MM-DD",
                    value.text
                ),
                value.span,
            ),
        }
    }

    /// Reads each written end of a range with `read`, reporting a range with
    /// neither end. Both ends are read so each reports its own error.
    fn range_ends<R>(
        &mut self,
        lo: Option<&Value>,
        hi: Option<&Value>,
        span: Span,
        mut read: impl FnMut(&mut Self, &Value) -> Option<R>,
    ) -> Option<(Option<R>, Option<R>)> {
        if lo.is_none() && hi.is_none() {
            return self.fail("a range needs at least one end", span);
        }
        let lo_read = lo.map(|v| read(self, v));
        let hi_read = hi.map(|v| read(self, v));
        if [&lo_read, &hi_read]
            .into_iter()
            .any(|end| end.as_ref().is_some_and(Option::is_none))
        {
            return None;
        }
        Some((lo_read.flatten(), hi_read.flatten()))
    }

    /// Warns that a range admits nothing.
    fn warn_empty_range(&mut self, span: Span) {
        self.push(Severity::Warning, "this range is empty", span);
    }

    /// An interval field: a value is a span `[start, end)`; operators act on
    /// its bounds and a range runs from one start to the other's end.
    ///
    /// An `end` of `None` means the value runs to the calendar's end, so
    /// nothing comes after it.
    fn interval<T>(
        &mut self,
        term: &Term,
        mut read: impl FnMut(&mut Self, &Value) -> Option<(T, Option<T>)>,
    ) -> Option<(Option<T>, Option<T>)>
    where
        T: Copy + PartialOrd,
    {
        match &term.criterion {
            Criterion::Compare { op, value, .. } => {
                let (start, end) = read(self, value)?;
                match op {
                    Op::Match | Op::Equal => Some((Some(start), end)),
                    Op::Gt => match end {
                        Some(e) => Some((Some(e), None)),
                        None => {
                            self.fail(format!("nothing comes after '{}'", value.text), value.span)
                        }
                    },
                    Op::Ge => Some((Some(start), None)),
                    Op::Lt => Some((None, Some(start))),
                    Op::Le => Some((None, end)),
                }
            }
            Criterion::Range { lo, hi, span } => {
                let (lo_read, hi_read) =
                    self.range_ends(lo.as_ref(), hi.as_ref(), *span, &mut read)?;
                let from = lo_read.map(|(start, _)| start);
                let until = hi_read.and_then(|(_, end)| end);
                if let (Some(f), Some(u)) = (from, until)
                    && f >= u
                {
                    self.warn_empty_range(*span);
                }
                Some((from, until))
            }
            Criterion::Any(_) | Criterion::Group(..) => self.reject(term),
        }
    }

    /// `date:` and date-typed keys.
    fn date(&mut self, term: &Term) -> Option<DateRange> {
        self.interval(term, Self::period)
            .map(|(from, until)| DateRange::new(from, until))
    }

    /// Reads a plain decimal literal.
    fn number(&mut self, text: &str, span: Span) -> Option<Decimal> {
        let trimmed = text.trim();
        if !is_literal(trimmed) {
            return self.fail(format!("'{text}' is not a number"), span);
        }
        match evaluate(trimmed) {
            Ok(value) => Some(value),
            Err(_overflow) => self.fail(format!("'{text}' is not a number"), span),
        }
    }

    /// Reads an amount with an optional currency marker.
    fn money(&mut self, value: &Value, sign: Sign) -> Option<(Decimal, Option<String>)> {
        let catalog = self.catalog;
        let (number_text, code) = match split_marked_amount(catalog.commodities(), &value.text) {
            Ok((n, c)) => (n, Some(c)),
            Err(MarkerError::Missing) => (value.text.clone(), None),
            Err(MarkerError::Unknown(m)) => return self.fail(unknown_currency(&m), value.span),
            Err(MarkerError::Ambiguous(m)) => {
                return self.fail(ambiguous_currency(&m), value.span);
            }
        };
        let number = self.number(&number_text, value.span)?;
        if sign == Sign::Magnitude && number.is_sign_negative() {
            return self.fail("'amount:' compares magnitudes; drop the sign", value.span);
        }
        Some((number, code))
    }

    /// A numeric field: a comparison or an inclusive range, each value read
    /// by `read` as a number and an optional commodity.
    fn ranged(
        &mut self,
        term: &Term,
        mut read: impl FnMut(&mut Self, &Value) -> Option<(Decimal, Option<String>)>,
    ) -> Option<(NumRange, Option<String>)> {
        match &term.criterion {
            Criterion::Compare { op, value, .. } => {
                let (number, code) = read(self, value)?;
                Some((compare_range(*op, number), code))
            }
            Criterion::Range { lo, hi, span } => {
                let (lo_read, hi_read) =
                    self.range_ends(lo.as_ref(), hi.as_ref(), *span, &mut read)?;
                let lo_code = lo_read.as_ref().and_then(|(_, c)| c.clone());
                let hi_code = hi_read.as_ref().and_then(|(_, c)| c.clone());
                if let (Some(a), Some(b)) = (&lo_code, &hi_code)
                    && a != b
                {
                    return self.fail("a range's ends name different currencies", *span);
                }
                let range = NumRange::new(
                    lo_read.map(|(n, _)| Bound::new(n, true)),
                    hi_read.map(|(n, _)| Bound::new(n, true)),
                );
                if range.is_empty() {
                    self.warn_empty_range(*span);
                }
                Some((range, lo_code.or(hi_code)))
            }
            Criterion::Any(_) | Criterion::Group(..) => self.reject(term),
        }
    }

    /// `amount:` and amount-typed keys.
    fn amount(&mut self, term: &Term, sign: Sign) -> Option<AmountPred> {
        self.ranged(term, |r, v| r.money(v, sign))
            .map(|(range, commodity)| AmountPred::new(range, commodity))
    }

    /// `commodity:`.
    fn commodity(&mut self, term: &Term) -> Option<ResolvedExpr> {
        let value = match &term.criterion {
            Criterion::Compare {
                op: Op::Match | Op::Equal,
                value,
                ..
            } => value,
            Criterion::Compare { .. }
            | Criterion::Any(_)
            | Criterion::Range { .. }
            | Criterion::Group(..) => {
                return self.reject(term);
            }
        };
        let catalog = self.catalog;
        match resolve_marker(catalog.commodities(), &value.text) {
            Ok(code) => Some(pred(Pred::Commodity(code))),
            Err(MarkerError::Missing | MarkerError::Unknown(_)) => {
                self.fail(unknown_currency(&value.text), value.span)
            }
            Err(MarkerError::Ambiguous(m)) => self.fail(ambiguous_currency(&m), value.span),
        }
    }

    /// `any:(…)`.
    fn any(&mut self, term: &Term) -> Option<ResolvedExpr> {
        let Criterion::Group(inner, _) = &term.criterion else {
            return self.fail(
                "'any:' takes a parenthesised expression, e.g. any:(account:Bank)",
                term.criterion.span(),
            );
        };
        let outer = self.in_any;
        self.in_any = true;
        let resolved = self.expr(inner);
        self.in_any = outer;
        resolved.map(|r| ResolvedExpr::Any(Box::new(r)))
    }

    /// Hints at `-any:(…)` for a negated `account:` outside `any:`.
    fn hint_negated_account(&mut self, inner: &Expr, span: Span) {
        if self.in_any {
            return;
        }
        if let Expr::Term(term) = inner
            && !term.field.meta
            && term.field.name.eq_ignore_ascii_case("account")
            && let Criterion::Compare { value, .. } = &term.criterion
        {
            let printed = print(inner);
            self.push(
                Severity::Hint,
                format!(
                    "legs not on {}; to exclude transactions, use -any:({printed})",
                    value.text
                ),
                span,
            );
        }
    }

    /// Warns when an `and` asks for two different statuses of one kind.
    fn check_status_conflicts(&mut self, items: &[Expr]) {
        let mut seen: Vec<(Status, Span)> = Vec::new();
        for item in items {
            if let Expr::Term(term) = item
                && !term.field.meta
                && term.field.name.eq_ignore_ascii_case("status")
                && let Criterion::Compare {
                    op: Op::Match,
                    value,
                    ..
                } = &term.criterion
                && let Some(status) = status_word(&value.text)
            {
                seen.push((status, term.span));
            }
        }
        for kind in [true, false] {
            let of_kind: Vec<(Status, Span)> = seen
                .iter()
                .copied()
                .filter(|(s, _)| s.is_reconciliation() == kind)
                .collect();
            let Some(&(first, first_span)) = of_kind.first() else {
                continue;
            };
            if let Some(&(_, other_span)) = of_kind.iter().find(|(s, _)| *s != first) {
                let message = if kind {
                    "a transaction has one reconciliation status; this never matches"
                } else {
                    "a transaction is either balanced or not; this never matches"
                };
                self.push(Severity::Warning, message, first_span.to(other_span));
            }
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::fmt::Write as _;

    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::catalog::MetaKey;
    use crate::catalog::MetaType;
    use crate::currency::Commodity;
    use crate::parser::parse;

    /// An invented ledger.
    pub(super) struct Fixture {
        /// Accounts.
        accounts: Vec<PathEntry>,
        /// Tags.
        tags: Vec<PathEntry>,
        /// Commodities: AUD (A$), USD ($), CAD (C$, alias $).
        commodities: Vec<Commodity>,
        /// Metadata keys.
        keys: Vec<MetaKey>,
    }

    impl Catalog for Fixture {
        type Commodity = Commodity;

        fn accounts(&self) -> &[PathEntry] {
            &self.accounts
        }

        fn tags(&self) -> &[PathEntry] {
            &self.tags
        }

        fn commodities(&self) -> &[Commodity] {
            &self.commodities
        }

        fn meta_keys(&self) -> &[MetaKey] {
            &self.keys
        }
    }

    /// The shared fixture.
    pub(super) fn fixture() -> Fixture {
        Fixture {
            accounts: vec![
                PathEntry::new("a1", ["Expenses", "Food"]),
                PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
                PathEntry::new("a3", ["Income", "Food"]),
                PathEntry::new("a4", ["Assets", "Bank"]),
                PathEntry::new("a5", ["Liabilities", "Credit Card"]),
            ],
            tags: vec![
                PathEntry::new("t1", ["me"]),
                PathEntry::new("t2", ["partner"]),
                PathEntry::new("t3", ["institution"]),
                PathEntry::new("t4", ["institution", "bank-a"]),
            ],
            commodities: vec![
                Commodity::new("AUD", Some("A$"), &[]),
                Commodity::new("USD", Some("$"), &[]),
                Commodity::new("CAD", Some("C$"), &["$"]),
            ],
            keys: vec![
                MetaKey::new("payee", MetaType::Text, 0),
                MetaKey::new("km", MetaType::Number, 2),
                MetaKey::new("visits", MetaType::Number, 1),
                MetaKey::new("deposit", MetaType::Amount, 0),
                MetaKey::new("reimbursed", MetaType::Boolean, 0),
                MetaKey::new("due", MetaType::Date, 0),
                MetaKey::new("synced", MetaType::Timestamp, 0),
                MetaKey::new("owner", MetaType::Account, 0),
            ],
        }
    }

    /// Resolves `text`, which must parse.
    pub(super) fn run(text: &str) -> Resolved {
        resolve(&parse(text).expect("parses"), &fixture())
    }

    /// Resolves `text`, which must produce no error.
    pub(super) fn ok(text: &str) -> ResolvedExpr {
        let resolved = run(text);
        assert!(!resolved.has_errors(), "{text}: {:?}", resolved.diagnostics);
        resolved.expr.expect("no errors means an expression")
    }

    /// The `(severity, message)` pairs for `text`.
    pub(super) fn messages(text: &str) -> Vec<(Severity, String)> {
        run(text)
            .diagnostics
            .into_iter()
            .map(|d| (d.severity, d.message))
            .collect()
    }

    /// The severity as the snapshot spells it.
    const fn severity_name(severity: Severity) -> &'static str {
        match severity {
            Severity::Error => "Error",
            Severity::Warning => "Warning",
            Severity::Hint => "Hint",
        }
    }

    /// Wraps a predicate.
    pub(super) fn p(pred: Pred) -> ResolvedExpr {
        ResolvedExpr::Pred(pred)
    }

    /// An amount predicate over `[lo, hi]`.
    fn amount(lo: Option<Bound>, hi: Option<Bound>, commodity: Option<&str>) -> ResolvedExpr {
        p(Pred::Amount(AmountPred {
            range: NumRange::new(lo, hi),
            commodity: commodity.map(str::to_owned),
        }))
    }

    /// A date predicate over `[from, until)`.
    fn dates(from: Option<Date>, until: Option<Date>) -> ResolvedExpr {
        p(Pred::Date(DateRange { from, until }))
    }

    #[rstest]
    #[case("coffee", p(Pred::Description(TextMatch::Contains("coffee".to_owned()))))]
    #[case("Coffee", p(Pred::Description(TextMatch::Contains("coffee".to_owned()))))]
    #[case("description:=\"Blue Bottle\"", p(Pred::Description(TextMatch::Equals("blue bottle".to_owned()))))]
    #[case("description:a..b", p(Pred::Description(TextMatch::Contains("a..b".to_owned()))))]
    #[case("account:Food:Groceries", p(Pred::Account { id: "a2".to_owned(), subtree: true }))]
    #[case("account:=Expenses:Food", p(Pred::Account { id: "a1".to_owned(), subtree: false }))]
    #[case("ACCOUNT:assets:bank", p(Pred::Account { id: "a4".to_owned(), subtree: true }))]
    #[case("account:\"Credit Card\"", p(Pred::Account { id: "a5".to_owned(), subtree: true }))]
    #[case("tag:institution", p(Pred::Tag(TagPred::Tag { id: "t3".to_owned(), subtree: true })))]
    #[case("tag:=institution", p(Pred::Tag(TagPred::Tag { id: "t3".to_owned(), subtree: false })))]
    #[case("tag:bank-a", p(Pred::Tag(TagPred::Tag { id: "t4".to_owned(), subtree: true })))]
    #[case("tag:*", p(Pred::Tag(TagPred::Any)))]
    #[case("status:Reconciled", p(Pred::Status(Status::Reconciled)))]
    #[case("status:unbalanced", p(Pred::Status(Status::Unbalanced)))]
    #[case("date:2026-03", dates(Some(date(2026, 3, 1)), Some(date(2026, 4, 1))))]
    #[case("date:=2026-03", dates(Some(date(2026, 3, 1)), Some(date(2026, 4, 1))))]
    #[case("date:>2026-03", dates(Some(date(2026, 4, 1)), None))]
    #[case("date:>=2026", dates(Some(date(2026, 1, 1)), None))]
    #[case("date:<2026-03-15", dates(None, Some(date(2026, 3, 15))))]
    #[case("date:<=2026-03", dates(None, Some(date(2026, 4, 1))))]
    #[case(
        "date:2026-01..2026-03",
        dates(Some(date(2026, 1, 1)), Some(date(2026, 4, 1)))
    )]
    #[case("date:2026-01..", dates(Some(date(2026, 1, 1)), None))]
    #[case("date:..2026", dates(None, Some(date(2027, 1, 1))))]
    #[case("date:..\"2026-03\"", dates(None, Some(date(2026, 4, 1))))]
    #[case("date:2026..9999", dates(Some(date(2026, 1, 1)), None))]
    #[case("date:9999", dates(Some(date(9999, 1, 1)), None))]
    #[case("amount:150", amount(Some(Bound::new(dec!(150), true)), Some(Bound::new(dec!(150), true)), None))]
    #[case("amount:>A$150", amount(Some(Bound::new(dec!(150), false)), None, Some("AUD")))]
    #[case("amount:\"150 AUD\"", amount(Some(Bound::new(dec!(150), true)), Some(Bound::new(dec!(150), true)), Some("AUD")))]
    #[case("amount:\"150 AUD\"..200", amount(Some(Bound::new(dec!(150), true)), Some(Bound::new(dec!(200), true)), Some("AUD")))]
    #[case("amount:A$100..200", amount(Some(Bound::new(dec!(100), true)), Some(Bound::new(dec!(200), true)), Some("AUD")))]
    #[case("amount:>1,000", amount(Some(Bound::new(dec!(1000), false)), None, None))]
    #[case("amount:<=20", amount(None, Some(Bound::new(dec!(20), true)), None))]
    #[case("commodity:aud", p(Pred::Commodity("AUD".to_owned())))]
    #[case("any:(account:Bank)", ResolvedExpr::Any(Box::new(p(Pred::Account { id: "a4".to_owned(), subtree: true }))))]
    #[case("-tag:me", ResolvedExpr::Not(Box::new(p(Pred::Tag(TagPred::Tag { id: "t1".to_owned(), subtree: true })))))]
    #[case("a or b", ResolvedExpr::Or(vec![
        p(Pred::Description(TextMatch::Contains("a".to_owned()))),
        p(Pred::Description(TextMatch::Contains("b".to_owned()))),
    ]))]
    fn resolves_builtins(#[case] text: &str, #[case] expected: ResolvedExpr) {
        assert_eq!(ok(text), expected);
    }

    #[rstest]
    #[case("acount:Food", "unknown field 'acount' (did you mean 'account'?)")]
    #[case("colour:red", "unknown field 'colour'")]
    #[case("account:Food", "'Food' is ambiguous: Expenses:Food, Income:Food")]
    #[case("account:Nowhere", "no account matches 'Nowhere'")]
    #[case("account:Expenses::Food", "'Expenses::Food' has an empty segment")]
    #[case("tag:nobody", "no tag matches 'nobody'")]
    #[case("account:>5", "'account:' has no '>'")]
    #[case("account:*", "'account:' has no '*'")]
    #[case("account:a..b", "'account:' has no range")]
    #[case("account:(x)", "'account:' takes no sub-expression")]
    #[case("description:*", "'description:' has no '*'")]
    #[case("description:>a", "'description:' has no '>'")]
    #[case(
        "status:rec",
        "'status:' takes unreconciled, flagged, reconciled, balanced or unbalanced"
    )]
    #[case("status:=reconciled", "'status:' has no '='")]
    #[case(
        "date:2026-13",
        "'2026-13' is not a date; use YYYY, YYYY-MM or YYYY-MM-DD"
    )]
    #[case("date:..", "a range needs at least one end")]
    #[case("date:*", "'date:' has no '*'")]
    #[case("date:>9999", "nothing comes after '9999'")]
    #[case("amount:abc", "'abc' is not a number")]
    #[case(
        "amount:>99999999999999999999999999999",
        "'99999999999999999999999999999' is not a number"
    )]
    #[case("amount:-5", "'amount:' compares magnitudes; drop the sign")]
    #[case("amount:\"XYZ 5\"", "unknown currency 'XYZ'")]
    #[case("amount:$5", "'$' matches more than one currency; use its code")]
    #[case("amount:A$1..C$5", "a range's ends name different currencies")]
    #[case("amount:*", "'amount:' has no '*'")]
    #[case("commodity:XYZ", "unknown currency 'XYZ'")]
    #[case("commodity:>AUD", "'commodity:' has no '>'")]
    #[case(
        "any:tag",
        "'any:' takes a parenthesised expression, e.g. any:(account:Bank)"
    )]
    fn rejects_builtins(#[case] text: &str, #[case] message: &str) {
        assert_eq!(messages(text), vec![(Severity::Error, message.to_owned())]);
        assert_eq!(run(text).expr, None);
    }

    #[test]
    fn collects_every_error() {
        assert_eq!(
            messages("acount:x colour:y"),
            vec![
                (
                    Severity::Error,
                    "unknown field 'acount' (did you mean 'account'?)".to_owned()
                ),
                (Severity::Error, "unknown field 'colour'".to_owned()),
            ]
        );
    }

    #[test]
    fn reports_both_ends_of_a_range() {
        assert_eq!(
            messages("amount:abc..xyz"),
            vec![
                (Severity::Error, "'abc' is not a number".to_owned()),
                (Severity::Error, "'xyz' is not a number".to_owned()),
            ]
        );
    }

    #[rstest]
    #[case("date:2026-03..2026-01", Severity::Warning, "this range is empty")]
    #[case("amount:200..100", Severity::Warning, "this range is empty")]
    #[case(
        "status:reconciled status:flagged",
        Severity::Warning,
        "a transaction has one reconciliation status; this never matches"
    )]
    #[case(
        "status:balanced status:unbalanced",
        Severity::Warning,
        "a transaction is either balanced or not; this never matches"
    )]
    #[case(
        "-account:Bank",
        Severity::Hint,
        "legs not on Bank; to exclude transactions, use -any:(account:Bank)"
    )]
    fn warns_without_blocking(
        #[case] text: &str,
        #[case] severity: Severity,
        #[case] message: &str,
    ) {
        let resolved = run(text);
        assert_eq!(messages(text), vec![(severity, message.to_owned())]);
        assert!(resolved.expr.is_some(), "a warning must not block: {text}");
    }

    #[test]
    fn negated_account_inside_any_draws_no_hint() {
        assert_eq!(messages("-any:(account:Bank)"), vec![]);
    }

    #[test]
    fn diagnostics_corpus() {
        let corpus = [
            "acount:Food",
            "account:Food",
            "date:2026-13",
            "amount:A$1..C$5",
            "date:2026-03..2026-01",
            "-account:Bank",
        ];
        let mut out = String::new();
        for text in corpus {
            for d in run(text).diagnostics {
                writeln!(
                    out,
                    "{text} => {} {}..{}: {}",
                    severity_name(d.severity),
                    d.span.start,
                    d.span.end,
                    d.message
                )
                .expect("writing to a String cannot fail");
            }
        }
        insta::assert_snapshot!(out);
    }
}
