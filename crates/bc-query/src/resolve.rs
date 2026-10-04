//! Types each term of a parsed [`Expr`] against a [`Catalog`].

use bc_expr::evaluate;
use bc_expr::is_literal;
use jiff::SignedDuration;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::ast::Op;
use crate::ast::Term;
use crate::ast::Value;
use crate::catalog::Catalog;
use crate::catalog::MetaType;
use crate::catalog::PathEntry;
use crate::currency::MarkerError;
use crate::currency::resolve_marker;
use crate::currency::split_marked_amount;
use crate::filter::AmountPred;
use crate::filter::Bound;
use crate::filter::DateRange;
use crate::filter::MetaPred;
use crate::filter::NumRange;
use crate::filter::Pred;
use crate::filter::ResolvedExpr;
use crate::filter::Status;
use crate::filter::TagPred;
use crate::filter::TextMatch;
use crate::filter::TimeRange;
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
        warned_keys: Vec::new(),
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
    Signed,
}

/// The outcome of looking a path up that did not fail.
enum Lookup<'e> {
    /// Exactly one entry.
    Found(&'e PathEntry),
    /// No entry.
    Missing,
}

/// Resolution state.
struct Resolver<'c, C> {
    /// The ledger facts.
    catalog: &'c C,
    /// Diagnostics so far.
    diagnostics: Vec<Diagnostic>,
    /// Whether the cursor is inside `any:(…)`.
    in_any: bool,
    /// Keys already warned about for mismatched entries.
    warned_keys: Vec<String>,
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

/// The message for a path nothing matches, suggesting the closest segment
/// name for its last segment. `noun` names the kind of path (`account`, `tag`).
fn unknown_path(entries: &[PathEntry], text: &str, noun: &str) -> String {
    let last = text.rsplit(':').next().unwrap_or(text).to_ascii_lowercase();
    let folded: Vec<(String, &str)> = entries
        .iter()
        .flat_map(|a| a.path.iter())
        .map(|s| (fold(s), s.as_str()))
        .collect();
    let hint = suggest(&last, folded.iter().map(|(f, _)| f.as_str()))
        .and_then(|near| folded.iter().find(|(f, _)| f == near))
        .map(|(_, original)| *original);
    match hint {
        Some(s) => format!("no {noun} matches '{text}' (did you mean '{s}'?)"),
        None => format!("no {noun} matches '{text}'"),
    }
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
            Expr::Word(value) => {
                if value.text.is_empty() {
                    self.push(
                        Severity::Warning,
                        "an empty value matches every transaction",
                        value.span,
                    );
                }
                Some(pred(Pred::Description(TextMatch::Contains(fold(
                    &value.text,
                )))))
            }
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
            return self.meta_term(term);
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
            } => {
                if value.text.is_empty() {
                    let message = if term.field.meta {
                        format!(
                            "an empty value matches every transaction with '@{}'",
                            term.field.name.to_ascii_lowercase()
                        )
                    } else {
                        "an empty value matches every transaction".to_owned()
                    };
                    self.push(Severity::Warning, message, value.span);
                }
                Some(TextMatch::Contains(fold(&value.text)))
            }
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

    /// Looks a path up in `entries`. An empty segment or an ambiguous path is
    /// reported; a path nothing matches is returned as [`Lookup::Missing`] for
    /// the caller to report.
    fn find<'e>(&mut self, entries: &'e [PathEntry], value: &Value) -> Option<Lookup<'e>> {
        let Some(segments) = path::split(&value.text) else {
            return self.fail(empty_segment(&value.text), value.span);
        };
        match path::resolve(entries, &segments) {
            path::Match::One(entry) => Some(Lookup::Found(entry)),
            path::Match::Missing => Some(Lookup::Missing),
            path::Match::Many(found) => self.fail(ambiguous_path(&value.text, &found), value.span),
        }
    }

    /// Looks a path up in `entries`, reporting a missing or ambiguous one.
    fn lookup<'e>(
        &mut self,
        entries: &'e [PathEntry],
        value: &Value,
        noun: &str,
    ) -> Option<&'e PathEntry> {
        match self.find(entries, value)? {
            Lookup::Found(entry) => Some(entry),
            Lookup::Missing => self.fail(unknown_path(entries, &value.text, noun), value.span),
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
        let (range, commodity) = self.ranged(term, |r, v| r.money(v, sign))?;
        // A negative bound is already an error in `money`, so only the
        // exclusive zero bound (`amount:<0`) reaches here.
        if sign == Sign::Magnitude && range.hi.is_some_and(|b| b.value.is_zero() && !b.inclusive) {
            self.push(
                Severity::Warning,
                "'amount:' compares magnitudes, so nothing is below 0; this never matches",
                term.span,
            );
        }
        Some(AmountPred::new(range, commodity))
    }

    /// An `@key:` term, read by the key's registered type.
    fn meta_term(&mut self, term: &Term) -> Option<ResolvedExpr> {
        let catalog = self.catalog;
        let key = term.field.name.to_ascii_lowercase();
        let Some(def) = catalog.meta_keys().iter().find(|k| k.key == key) else {
            let message = match suggest(&key, catalog.meta_keys().iter().map(|k| k.key.as_str())) {
                Some(s) => format!("unknown key '@{key}' (did you mean '@{s}'?)"),
                None => format!("unknown key '@{key}'"),
            };
            return self.fail(message, term.field.span);
        };
        if let Criterion::Any(_) = term.criterion {
            return Some(pred(Pred::Meta {
                key,
                pred: MetaPred::Exists,
            }));
        }
        let meta_pred = match def.ty {
            MetaType::Text => self.text(term).map(MetaPred::Text),
            MetaType::Number => self
                .ranged(term, |r, v| r.number(&v.text, v.span).map(|n| (n, None)))
                .map(|(range, _)| MetaPred::Number(range)),
            MetaType::Amount => self.amount(term, Sign::Signed).map(MetaPred::Amount),
            MetaType::Boolean => self.boolean(term).map(MetaPred::Boolean),
            MetaType::Date => self.date(term).map(MetaPred::Date),
            MetaType::Timestamp => self
                .interval(term, Self::instant)
                .map(|(from, until)| MetaPred::Timestamp(TimeRange::new(from, until))),
            MetaType::Account => self.meta_account(term),
        }?;
        if def.ty != MetaType::Text && def.mismatched > 0 && !self.warned_keys.contains(&key) {
            self.warned_keys.push(key.clone());
            let message = if def.mismatched == 1 {
                format!(
                    "1 value of '@{key}' is not {} and was not compared",
                    def.ty.singular()
                )
            } else {
                format!(
                    "{} values of '@{key}' are not {} and were not compared",
                    def.mismatched,
                    def.ty.plural()
                )
            };
            self.push(Severity::Warning, message, term.span);
        }
        Some(pred(Pred::Meta {
            key,
            pred: meta_pred,
        }))
    }

    /// `true` or `false`.
    fn boolean(&mut self, term: &Term) -> Option<bool> {
        let value = match &term.criterion {
            Criterion::Compare {
                op: Op::Match | Op::Equal,
                value,
                ..
            } => value,
            Criterion::Compare { .. }
            | Criterion::Any(_)
            | Criterion::Range { .. }
            | Criterion::Group(..) => return self.reject(term),
        };
        match fold(&value.text).as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => self.fail(format!("'{}' takes true or false", label(term)), value.span),
        }
    }

    /// An RFC 3339 instant as `[t, t + 1ns)`, or a date period bounded at UTC
    /// midnight.
    fn instant(&mut self, value: &Value) -> Option<(Timestamp, Option<Timestamp>)> {
        if let Ok(at) = value.text.parse::<Timestamp>() {
            return Some((at, at.checked_add(SignedDuration::from_nanos(1)).ok()));
        }
        let Some(found) = period::parse(&value.text) else {
            return self.fail(
                format!("'{}' is not a date or RFC 3339 timestamp", value.text),
                value.span,
            );
        };
        let utc = |d: Date| {
            d.to_zoned(jiff::tz::TimeZone::UTC)
                .ok()
                .map(|z| z.timestamp())
        };
        let Some(start) = utc(found.start) else {
            return self.fail(format!("'{}' is out of range", value.text), value.span);
        };
        Some((start, found.end.and_then(utc)))
    }

    /// An account-typed key, matched on its stored path text.
    ///
    /// A path no live account matches is kept verbatim so tombstoned values
    /// still match. It warns when it looks like a full path (its first segment
    /// starts some live account) and is an error otherwise, since a mistyped
    /// trailing run would silently match nothing.
    fn meta_account(&mut self, term: &Term) -> Option<MetaPred> {
        let catalog = self.catalog;
        let (value, subtree) = self.path_value(term)?;
        let path = match self.find(catalog.accounts(), value)? {
            Lookup::Found(entry) => entry.path.clone(),
            Lookup::Missing => {
                let segments: Vec<String> = value.text.split(':').map(str::to_owned).collect();
                let first = segments.first().map_or("", String::as_str);
                if catalog.accounts().iter().any(|a| {
                    a.path
                        .first()
                        .is_some_and(|f| f.eq_ignore_ascii_case(first))
                }) {
                    self.push(
                        Severity::Warning,
                        format!(
                            "no live account matches '{}'; matching stored paths only",
                            value.text
                        ),
                        value.span,
                    );
                    segments
                } else {
                    return self.fail(
                        unknown_path(catalog.accounts(), &value.text, "account"),
                        value.span,
                    );
                }
            }
        };
        Some(MetaPred::Account { path, subtree })
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

    use jiff::Timestamp;
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
    #[case("amount:=150", amount(Some(Bound::new(dec!(150), true)), Some(Bound::new(dec!(150), true)), None))]
    #[case("amount:150.00", amount(Some(Bound::new(dec!(150), true)), Some(Bound::new(dec!(150), true)), None))]
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
    #[case("acount:Food", "unknown field 'acount' (did you mean 'account'?)")] // spellchecker:disable-line
    #[case("colour:red", "unknown field 'colour'")]
    #[case("account:Food", "'Food' is ambiguous: Expenses:Food, Income:Food")]
    #[case("account:Nowhere", "no account matches 'Nowhere'")]
    #[case(
        "account:Grocries",
        "no account matches 'Grocries' (did you mean 'Groceries'?)"
    )]
    #[case("account:Expenses::Food", "'Expenses::Food' has an empty segment")]
    #[case("tag:nobody", "no tag matches 'nobody'")]
    #[case(
        "tag:institutoin",
        "no tag matches 'institutoin' (did you mean 'institution'?)"
    )]
    #[case("account:>5", "'account:' has no '>'")]
    #[case("account:*", "'account:' has no '*'")]
    #[case("account:a..b", "'account:' has no range")]
    #[case("account:(x)", "'account:' takes no sub-expression")]
    #[case("description:*", "'description:' has no '*'")]
    #[case("description:>a", "'description:' has no '>'")]
    #[case("description:(x)", "'description:' takes no sub-expression")]
    #[case("tag:>x", "'tag:' has no '>'")]
    #[case("tag:a..b", "'tag:' has no range")]
    #[case("commodity:*", "'commodity:' has no '*'")]
    #[case("commodity:a..b", "'commodity:' has no range")]
    #[case("status:*", "'status:' has no '*'")]
    #[case("status:>x", "'status:' has no '>'")]
    #[case("status:a..b", "'status:' has no range")]
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
            messages("acount:x colour:y"), // spellchecker:disable-line
            vec![
                (
                    Severity::Error,
                    "unknown field 'acount' (did you mean 'account'?)".to_owned() // spellchecker:disable-line
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
        "amount:<0",
        Severity::Warning,
        "'amount:' compares magnitudes, so nothing is below 0; this never matches"
    )]
    #[case("\"\"", Severity::Warning, "an empty value matches every transaction")]
    #[case(
        "description:\"\"",
        Severity::Warning,
        "an empty value matches every transaction"
    )]
    #[case(
        "@payee:\"\"",
        Severity::Warning,
        "an empty value matches every transaction with '@payee'"
    )]
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

    #[rstest]
    #[case("amount:<=0")]
    #[case("amount:..0")]
    #[case("amount:<1")]
    fn amount_upper_bound_at_or_above_zero_draws_no_warning(#[case] text: &str) {
        assert_eq!(messages(text), vec![]);
    }

    #[rstest]
    #[case("amount:<=-1")]
    #[case("amount:..-1")]
    fn negative_magnitude_bound_is_an_error(#[case] text: &str) {
        let found = messages(text);
        assert!(
            found
                .iter()
                .any(|(s, m)| *s == Severity::Error && m.contains("drop the sign")),
            "{found:?}"
        );
    }

    #[rstest]
    #[case("description:=\"\"")]
    #[case("@payee:=\"\"")]
    fn empty_exact_text_draws_no_warning(#[case] text: &str) {
        assert_eq!(messages(text), vec![]);
    }

    #[rstest]
    #[case("status:reconciled status:reconciled")]
    #[case("status:reconciled status:balanced")]
    #[case("status:reconciled or status:flagged")]
    #[case("-status:flagged status:reconciled")]
    fn compatible_statuses_draw_no_warning(#[case] text: &str) {
        assert_eq!(messages(text), vec![]);
        assert!(run(text).expr.is_some());
    }

    #[test]
    fn negated_account_inside_any_draws_no_hint() {
        assert_eq!(messages("-any:(account:Bank)"), vec![]);
    }

    /// A metadata predicate.
    fn meta(key: &str, pred: MetaPred) -> ResolvedExpr {
        p(Pred::Meta {
            key: key.to_owned(),
            pred,
        })
    }

    /// An instant from RFC 3339 text.
    fn ts(text: &str) -> Timestamp {
        text.parse().expect("valid timestamp")
    }

    #[rstest]
    #[case("@payee:coffee", meta("payee", MetaPred::Text(TextMatch::Contains("coffee".to_owned()))))]
    #[case("@Payee:=\"Example Cafe\"", meta("payee", MetaPred::Text(TextMatch::Equals("example cafe".to_owned()))))]
    #[case("@payee:a..b", meta("payee", MetaPred::Text(TextMatch::Contains("a..b".to_owned()))))]
    #[case("@payee:*", meta("payee", MetaPred::Exists))]
    #[case("@km:*", meta("km", MetaPred::Exists))]
    #[case("@km:=5", meta("km", MetaPred::Number(NumRange::new(Some(Bound::new(dec!(5), true)), Some(Bound::new(dec!(5), true))))))]
    #[case("@km:>5", meta("km", MetaPred::Number(NumRange::new(Some(Bound::new(dec!(5), false)), None))))]
    #[case("@km:..\"5\"", meta("km", MetaPred::Number(NumRange::new(None, Some(Bound::new(dec!(5), true))))))]
    #[case("@deposit:\"A$500\"..900", meta("deposit", MetaPred::Amount(AmountPred {
        range: NumRange::new(Some(Bound::new(dec!(500), true)), Some(Bound::new(dec!(900), true))),
        commodity: Some("AUD".to_owned()),
    })))]
    #[case("@deposit:>=A$500", meta("deposit", MetaPred::Amount(AmountPred {
        range: NumRange::new(Some(Bound::new(dec!(500), true)), None),
        commodity: Some("AUD".to_owned()),
    })))]
    #[case("@deposit:-50", meta("deposit", MetaPred::Amount(AmountPred {
        range: NumRange::new(Some(Bound::new(dec!(-50), true)), Some(Bound::new(dec!(-50), true))),
        commodity: None,
    })))]
    #[case("@reimbursed:TRUE", meta("reimbursed", MetaPred::Boolean(true)))]
    #[case("@reimbursed:=false", meta("reimbursed", MetaPred::Boolean(false)))]
    #[case("@due:2026-03", meta("due", MetaPred::Date(DateRange {
        from: Some(date(2026, 3, 1)),
        until: Some(date(2026, 4, 1)),
    })))]
    #[case("@due:>=2026-03", meta("due", MetaPred::Date(DateRange {
        from: Some(date(2026, 3, 1)),
        until: None,
    })))]
    #[case("@due:2026-01..2026-03", meta("due", MetaPred::Date(DateRange {
        from: Some(date(2026, 1, 1)),
        until: Some(date(2026, 4, 1)),
    })))]
    #[case("@synced:2026-03..", meta("synced", MetaPred::Timestamp(TimeRange {
        from: Some(ts("2026-03-01T00:00:00Z")),
        until: None,
    })))]
    #[case("@synced:<2026-03-15T10:00:00Z", meta("synced", MetaPred::Timestamp(TimeRange {
        from: None,
        until: Some(ts("2026-03-15T10:00:00Z")),
    })))]
    #[case("@synced:<=2026-03-15T10:00:00Z", meta("synced", MetaPred::Timestamp(TimeRange {
        from: None,
        until: Some(ts("2026-03-15T10:00:00.000000001Z")),
    })))]
    #[case("@owner:=Expenses:Food", meta("owner", MetaPred::Account {
        path: vec!["Expenses".to_owned(), "Food".to_owned()],
        subtree: false,
    }))]
    #[case("@synced:2026-03", meta("synced", MetaPred::Timestamp(TimeRange {
        from: Some(ts("2026-03-01T00:00:00Z")),
        until: Some(ts("2026-04-01T00:00:00Z")),
    })))]
    #[case("@synced:2026-03-15T10:00:00Z", meta("synced", MetaPred::Timestamp(TimeRange {
        from: Some(ts("2026-03-15T10:00:00Z")),
        until: Some(ts("2026-03-15T10:00:00.000000001Z")),
    })))]
    #[case("@synced:>2026-03-15T10:00:00Z", meta("synced", MetaPred::Timestamp(TimeRange {
        from: Some(ts("2026-03-15T10:00:00.000000001Z")),
        until: None,
    })))]
    #[case("@owner:Groceries", meta("owner", MetaPred::Account {
        path: vec!["Expenses".to_owned(), "Food".to_owned(), "Groceries".to_owned()],
        subtree: true,
    }))]
    fn resolves_meta(#[case] text: &str, #[case] expected: ResolvedExpr) {
        assert_eq!(ok(text), expected);
    }

    #[test]
    fn number_key_compares_signed_and_warns_about_mismatches() {
        let resolved = run("@km:-5..5");
        assert_eq!(
            resolved.expr,
            Some(meta(
                "km",
                MetaPred::Number(NumRange::new(
                    Some(Bound::new(dec!(-5), true)),
                    Some(Bound::new(dec!(5), true)),
                ))
            ))
        );
        assert_eq!(
            messages("@km:-5..5"),
            vec![(
                Severity::Warning,
                "2 values of '@km' are not numbers and were not compared".to_owned()
            )]
        );
        assert_eq!(
            messages("@visits:<0"),
            vec![(
                Severity::Warning,
                "1 value of '@visits' is not a number and was not compared".to_owned()
            )]
        );
        assert_eq!(messages("@km:*"), vec![]);
    }

    #[test]
    fn mismatch_warning_appears_once_per_key() {
        let found = messages("@km:1 or @km:5..9");
        assert_eq!(
            found,
            vec![(
                Severity::Warning,
                "2 values of '@km' are not numbers and were not compared".to_owned()
            )]
        );
    }

    #[test]
    fn tombstoned_account_path_warns_and_matches_by_text() {
        let resolved = run("@owner:=Assets:Gone");
        assert_eq!(
            resolved.expr,
            Some(meta(
                "owner",
                MetaPred::Account {
                    path: vec!["Assets".to_owned(), "Gone".to_owned()],
                    subtree: false,
                }
            ))
        );
        assert_eq!(
            messages("@owner:=Assets:Gone"),
            vec![(
                Severity::Warning,
                "no live account matches 'Assets:Gone'; matching stored paths only".to_owned()
            )]
        );
    }

    #[rstest]
    #[case("@pyee:x", "unknown key '@pyee' (did you mean '@payee'?)")]
    #[case("@colour:x", "unknown key '@colour'")]
    #[case("@payee:>x", "'@payee:' has no '>'")]
    #[case("@km:x", "'x' is not a number")]
    #[case("@km:A$5", "'A$5' is not a number")]
    #[case("@owner:Nowhere:Gone", "no account matches 'Nowhere:Gone'")]
    #[case("@reimbursed:maybe", "'@reimbursed:' takes true or false")]
    #[case("@reimbursed:>true", "'@reimbursed:' has no '>'")]
    #[case("@synced:nope", "'nope' is not a date or RFC 3339 timestamp")]
    #[case("@owner:Food", "'Food' is ambiguous: Expenses:Food, Income:Food")]
    #[case("@owner:>x", "'@owner:' has no '>'")]
    #[case("@owner:\"Assets::Bank\"", "'Assets::Bank' has an empty segment")]
    #[case(
        "@owner:Grocries",
        "no account matches 'Grocries' (did you mean 'Groceries'?)"
    )]
    #[case("@owner:Zzzzzzzz", "no account matches 'Zzzzzzzz'")]
    #[case("@due:x", "'x' is not a date; use YYYY, YYYY-MM or YYYY-MM-DD")]
    fn rejects_meta(#[case] text: &str, #[case] message: &str) {
        assert_eq!(messages(text), vec![(Severity::Error, message.to_owned())]);
    }

    #[test]
    fn diagnostics_corpus() {
        let corpus = [
            "acount:Food", // spellchecker:disable-line
            "account:Food",
            "date:2026-13",
            "amount:A$1..C$5",
            "date:2026-03..2026-01",
            "-account:Bank",
            "@pyee:x",
            "@km:>5",
            "@owner:=Assets:Gone",
            "amount:-5",
            "status:reconciled status:flagged",
            "@payee:\"\"",
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
