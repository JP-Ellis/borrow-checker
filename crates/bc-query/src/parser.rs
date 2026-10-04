//! Query text to [`Expr`]: a recursive-descent parser recording byte spans.

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::ast::Field;
use crate::ast::Op;
use crate::ast::Term;
use crate::ast::Value;
use crate::span::Span;

/// The deepest nesting the parser accepts; groups and prefix negations both count.
const MAX_DEPTH: usize = 64;

/// Why query text did not parse.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseError {
    /// What went wrong, for the palette's hint line.
    pub message: String,
    /// Where it went wrong.
    pub span: Span,
}

impl ParseError {
    /// Creates a parse error.
    ///
    /// # Arguments
    ///
    /// * `message` - What went wrong.
    /// * `span` - Where it went wrong.
    #[must_use]
    pub fn new(message: impl Into<String>, span: Span) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl core::fmt::Display for ParseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ParseError {}

/// Parses query text into an [`Expr`].
///
/// Field names, operators and values come back as written; [`crate::resolve`]
/// gives them meaning. Callers treat blank text as "no query" before calling.
///
/// # Arguments
///
/// * `text` - The query text.
///
/// # Errors
///
/// Returns [`ParseError`] for empty input, unbalanced parentheses or quotes,
/// a dangling operator, a malformed term, a free word written as `/…/`, or
/// groups and negations nested deeper than 64 levels.
pub fn parse(text: &str) -> Result<Expr, ParseError> {
    let mut parser = Parser {
        text,
        pos: 0,
        depth: 0,
    };
    parser.skip_space();
    if parser.at_end() {
        return Err(ParseError::new("the query is empty", parser.point()));
    }
    let expr = parser.or_expr()?;
    parser.skip_space();
    if parser.peek() == Some(')') {
        return Err(ParseError::new("unmatched ')'", parser.char_span(')')));
    }
    Ok(expr)
}

/// A keyword that structures the expression.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Keyword {
    /// `or`.
    Or,
    /// `and`.
    And,
    /// `not`.
    Not,
}

/// The keyword a bare run spells, in any case.
pub(crate) fn keyword(run: &str) -> Option<Keyword> {
    if run.eq_ignore_ascii_case("or") {
        Some(Keyword::Or)
    } else if run.eq_ignore_ascii_case("and") {
        Some(Keyword::And)
    } else if run.eq_ignore_ascii_case("not") {
        Some(Keyword::Not)
    } else {
        None
    }
}

/// The comparison operators, longest literal first so `>=` wins over `>`.
pub(crate) const OPERATORS: [(&str, Op); 5] = [
    (">=", Op::Ge),
    ("<=", Op::Le),
    (">", Op::Gt),
    ("<", Op::Lt),
    ("=", Op::Equal),
];

/// Whether `name` (without any `@`) is a well-formed field name.
pub(crate) fn is_field_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Byte length of the bare run at the start of `s`: everything up to
/// whitespace, a parenthesis or a quote.
fn bare_len(s: &str) -> usize {
    s.find(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '"'))
        .unwrap_or(s.len())
}

/// The parser's position in the query text.
struct Parser<'a> {
    /// The full query text.
    text: &'a str,
    /// Byte offset of the next unread character.
    pos: usize,
    /// How many parenthesised groups and prefix negations enclose the cursor.
    depth: usize,
}

impl<'a> Parser<'a> {
    /// The unread text.
    fn rest(&self) -> &'a str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    /// The next unread character.
    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    /// Whether all text is consumed.
    fn at_end(&self) -> bool {
        self.rest().is_empty()
    }

    /// Consumes `c`, which the caller has peeked.
    fn bump(&mut self, c: char) {
        self.pos = self.pos.saturating_add(c.len_utf8());
    }

    /// Skips whitespace.
    fn skip_space(&mut self) {
        let trimmed = self.rest().trim_start();
        self.pos = self.text.len().saturating_sub(trimmed.len());
    }

    /// An empty span at the cursor.
    const fn point(&self) -> Span {
        Span::new(self.pos, self.pos)
    }

    /// The span of `c` at the cursor.
    fn char_span(&self, c: char) -> Span {
        Span::new(self.pos, self.pos.saturating_add(c.len_utf8()))
    }

    /// The keyword at the cursor, if the bare run there is exactly one.
    fn peek_keyword(&self) -> Option<Keyword> {
        keyword(self.rest().get(..bare_len(self.rest())).unwrap_or_default())
    }

    /// Consumes the bare run at the cursor, returning its span.
    fn take_run(&mut self) -> Span {
        let start = self.pos;
        self.pos = start.saturating_add(bare_len(self.rest()));
        Span::new(start, self.pos)
    }

    /// Whether no operand can start here: end of text, `)`, `or` or `and`.
    fn at_operand_end(&self) -> bool {
        self.at_end()
            || self.peek() == Some(')')
            || matches!(self.peek_keyword(), Some(Keyword::Or | Keyword::And))
    }

    /// Fails unless the cursor is at whitespace, `)` or the end.
    fn expect_boundary(&self) -> Result<(), ParseError> {
        match self.peek() {
            None | Some(')') => Ok(()),
            Some(c) if c.is_whitespace() => Ok(()),
            Some(c) => Err(ParseError::new(
                format!("expected a space before '{c}'"),
                self.char_span(c),
            )),
        }
    }

    /// `and_expr { "or" and_expr }`.
    fn or_expr(&mut self) -> Result<Expr, ParseError> {
        let mut items = vec![self.and_expr()?];
        loop {
            self.skip_space();
            if self.peek_keyword() != Some(Keyword::Or) {
                break;
            }
            let keyword = self.take_run();
            self.skip_space();
            if self.at_operand_end() {
                return Err(ParseError::new(
                    "expected an expression after 'or'",
                    keyword,
                ));
            }
            items.push(self.and_expr()?);
        }
        Ok(join(items, Expr::Or))
    }

    /// `unary { ["and"] unary }`.
    fn and_expr(&mut self) -> Result<Expr, ParseError> {
        let mut items = vec![self.unary()?];
        loop {
            self.skip_space();
            if self.at_end() || self.peek() == Some(')') {
                break;
            }
            match self.peek_keyword() {
                Some(Keyword::Or) => break,
                Some(Keyword::And) => {
                    let keyword = self.take_run();
                    self.skip_space();
                    if self.at_operand_end() {
                        return Err(ParseError::new(
                            "expected an expression after 'and'",
                            keyword,
                        ));
                    }
                }
                Some(Keyword::Not) | None => {}
            }
            items.push(self.unary()?);
        }
        Ok(join(items, Expr::And))
    }

    /// `{ "-" | "not" } primary`, iterating so a long run of negations
    /// cannot exhaust the stack. Each negation counts toward [`MAX_DEPTH`].
    fn unary(&mut self) -> Result<Expr, ParseError> {
        let mut negations: Vec<usize> = Vec::new();
        loop {
            self.skip_space();
            let start = self.pos;
            let token = if self.peek() == Some('-') {
                self.bump('-');
                "-"
            } else if self.peek_keyword() == Some(Keyword::Not) {
                self.take_run();
                "not"
            } else {
                break;
            };
            let end = self.pos;
            if self.depth >= MAX_DEPTH {
                return Err(ParseError::new(
                    "the query nests more than 64 negations and groups",
                    Span::new(start, end),
                ));
            }
            self.depth = self.depth.saturating_add(1);
            negations.push(start);
            self.skip_space();
            if self.at_operand_end() {
                return Err(ParseError::new(
                    format!("expected an expression after '{token}'"),
                    Span::new(start, end),
                ));
            }
        }
        let mut expr = self.primary()?;
        self.depth = self.depth.saturating_sub(negations.len());
        while let Some(start) = negations.pop() {
            let span = Span::new(start, expr.span().end);
            expr = Expr::Not(Box::new(expr), span);
        }
        Ok(expr)
    }

    /// A group, a quoted word, a term or a bare word.
    fn primary(&mut self) -> Result<Expr, ParseError> {
        match self.peek() {
            None => Err(ParseError::new("expected an expression", self.point())),
            Some('(') => self.group(),
            Some(')') => Err(ParseError::new("unmatched ')'", self.char_span(')'))),
            Some('"') => {
                let value = self.quoted()?;
                self.expect_boundary()?;
                Ok(Expr::Word(value))
            }
            Some(_) => match self.peek_keyword() {
                Some(keyword @ (Keyword::Or | Keyword::And)) => {
                    let name = if keyword == Keyword::Or { "or" } else { "and" };
                    let span = self.take_run();
                    Err(ParseError::new(
                        format!("expected an expression before '{name}'"),
                        span,
                    ))
                }
                Some(Keyword::Not) | None => self.term_or_word(),
            },
        }
    }

    /// `"(" or_expr ")"`, the cursor on the `(`.
    fn group(&mut self) -> Result<Expr, ParseError> {
        let open = self.char_span('(');
        if self.depth >= MAX_DEPTH {
            return Err(ParseError::new("the query nests more than 64 groups", open));
        }
        self.bump('(');
        self.skip_space();
        if self.peek() == Some(')') {
            self.bump(')');
            return Err(ParseError::new(
                "empty parentheses",
                Span::new(open.start, self.pos),
            ));
        }
        self.depth = self.depth.saturating_add(1);
        let inner = self.or_expr()?;
        self.depth = self.depth.saturating_sub(1);
        self.skip_space();
        if self.peek() != Some(')') {
            return Err(ParseError::new("unclosed '('", open));
        }
        self.bump(')');
        Ok(inner)
    }

    /// A `field:criterion` term, or a bare word when the run has no colon.
    fn term_or_word(&mut self) -> Result<Expr, ParseError> {
        let start = self.pos;
        let run_len = bare_len(self.rest());
        let run = self.rest().get(..run_len).unwrap_or_default();
        let run_span = Span::new(start, start.saturating_add(run_len));
        let Some(colon) = run.find(':') else {
            if run.starts_with('@') {
                return Err(ParseError::new(
                    format!("expected ':' after '{run}'"),
                    run_span,
                ));
            }
            if run.len() >= 2 && run.starts_with('/') && run.ends_with('/') {
                return Err(ParseError::new(
                    "regular expressions are reserved; quote it to search for the text",
                    run_span,
                ));
            }
            self.pos = run_span.end;
            self.expect_boundary()?;
            return Ok(Expr::Word(Value::new(run, run_span)));
        };
        let head = run.get(..colon).unwrap_or_default();
        let field = field(head, start)?;
        self.pos = start.saturating_add(colon).saturating_add(1);
        let criterion = self.criterion(&field)?;
        let span = Span::new(start, criterion.span().end);
        Ok(Expr::Term(Term::new(field, criterion, span)))
    }

    /// Consumes a comparison operator, or none.
    fn operator(&mut self) -> Op {
        for (literal, op) in OPERATORS {
            if self.rest().starts_with(literal) {
                self.pos = self.pos.saturating_add(literal.len());
                return op;
            }
        }
        Op::Match
    }

    /// Everything after a term's colon.
    fn criterion(&mut self, field: &Field) -> Result<Criterion, ParseError> {
        let start = self.pos;
        let op = self.operator();
        match self.peek() {
            Some('(') if op == Op::Match => {
                let inner = self.group()?;
                self.expect_boundary()?;
                return Ok(Criterion::Group(
                    Box::new(inner),
                    Span::new(start, self.pos),
                ));
            }
            Some('"') => {
                let value = self.quoted()?;
                self.expect_boundary()?;
                return Ok(Criterion::Compare {
                    op,
                    value,
                    span: Span::new(start, self.pos),
                });
            }
            _ => {}
        }
        let value_start = self.pos;
        let text = self.rest().get(..bare_len(self.rest())).unwrap_or_default();
        self.pos = value_start.saturating_add(text.len());
        let span = Span::new(start, self.pos);
        if text.is_empty() {
            let sigil = if field.meta { "@" } else { "" };
            return Err(ParseError::new(
                format!(
                    "expected a value after '{sigil}{}:{}'",
                    field.name,
                    op.as_str()
                ),
                span,
            ));
        }
        self.expect_boundary()?;
        if op == Op::Match && text == "*" {
            return Ok(Criterion::Any(span));
        }
        if let Some((lo, hi)) = text.split_once("..") {
            if op != Op::Match {
                return Err(ParseError::new(
                    "a comparison takes one value, not a range",
                    span,
                ));
            }
            let lo_end = value_start.saturating_add(lo.len());
            let lo_value = (!lo.is_empty()).then(|| Value::new(lo, Span::new(value_start, lo_end)));
            let hi_start = lo_end.saturating_add(2);
            let hi_value = (!hi.is_empty()).then(|| Value::new(hi, Span::new(hi_start, self.pos)));
            return Ok(Criterion::Range {
                lo: lo_value,
                hi: hi_value,
                span,
            });
        }
        Ok(Criterion::Compare {
            op,
            value: Value::new(text, Span::new(value_start, self.pos)),
            span,
        })
    }

    /// A quoted string, the cursor on the opening quote. `\"` and `\\` are
    /// the only escapes.
    fn quoted(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        self.bump('"');
        let mut text = String::new();
        loop {
            match self.peek() {
                None => return Err(ParseError::new("unclosed '\"'", Span::new(start, self.pos))),
                Some('"') => {
                    self.bump('"');
                    return Ok(Value::new(&text, Span::new(start, self.pos)));
                }
                Some('\\') => {
                    let escape = self.pos;
                    self.bump('\\');
                    match self.peek() {
                        Some(c @ ('"' | '\\')) => {
                            text.push(c);
                            self.bump(c);
                        }
                        Some(c) => {
                            return Err(ParseError::new(
                                format!(r#"unknown escape '\{c}'; only \" and \\ are allowed"#),
                                Span::new(escape, self.pos.saturating_add(c.len_utf8())),
                            ));
                        }
                        None => {
                            return Err(ParseError::new(
                                "unclosed '\"'",
                                Span::new(start, self.pos),
                            ));
                        }
                    }
                }
                Some(c) => {
                    text.push(c);
                    self.bump(c);
                }
            }
        }
    }
}

/// Checks a field head (`account`, `@payee`) and builds its [`Field`].
fn field(head: &str, start: usize) -> Result<Field, ParseError> {
    let (meta, name) = head.strip_prefix('@').map_or((false, head), |n| (true, n));
    let span = Span::new(start, start.saturating_add(head.len()));
    if !is_field_name(name) {
        return Err(ParseError::new(
            format!("'{head}' is not a field name; quote text that contains ':'"),
            span,
        ));
    }
    Ok(Field::new(name, meta, span))
}

/// One item stays itself; two or more become `build(items, span)`.
fn join(mut items: Vec<Expr>, build: fn(Vec<Expr>, Span) -> Expr) -> Expr {
    if items.len() == 1
        && let Some(only) = items.pop()
    {
        return only;
    }
    let first = items.first().map(Expr::span).unwrap_or_default();
    let last = items.last().map(Expr::span).unwrap_or_default();
    build(items, first.to(last))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::fmt::Write as _;

    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    /// Renders an expression as an S-expression; values sit in `<…>` so their
    /// exact text is visible.
    fn shape(expr: &Expr) -> String {
        match expr {
            Expr::Or(items, _) => list("or", items),
            Expr::And(items, _) => list("and", items),
            Expr::Not(inner, _) => format!("(not {})", shape(inner)),
            Expr::Term(term) => {
                let sigil = if term.field.meta { "@" } else { "" };
                let criterion = match &term.criterion {
                    Criterion::Any(_) => "*".to_owned(),
                    Criterion::Compare { op, value, .. } => {
                        format!("{}<{}>", op.as_str(), value.text)
                    }
                    Criterion::Range { lo, hi, .. } => format!(
                        "<{}>..<{}>",
                        lo.as_ref().map_or("", |v| v.text.as_str()),
                        hi.as_ref().map_or("", |v| v.text.as_str())
                    ),
                    Criterion::Group(inner, _) => format!("({})", shape(inner)),
                };
                format!("{sigil}{}:{criterion}", term.field.name)
            }
            Expr::Word(value) => format!("<{}>", value.text),
        }
    }

    /// `(head a b …)`.
    fn list(head: &str, items: &[Expr]) -> String {
        let mut out = format!("({head}");
        for item in items {
            write!(out, " {}", shape(item)).expect("writing to a String cannot fail");
        }
        out.push(')');
        out
    }

    #[rstest]
    #[case("coffee", "<coffee>")]
    #[case("café", "<café>")]
    #[case("blue bottle", "(and <blue> <bottle>)")]
    #[case("\"blue bottle\"", "<blue bottle>")]
    #[case("\"or\"", "<or>")]
    #[case(r#""say \"hi\"""#, r#"<say "hi">"#)]
    #[case("@payee:coffee", "@payee:<coffee>")]
    #[case("@payee:\"Zoë\"", "@payee:<Zoë>")]
    #[case("@payee:=\"Blue Bottle\"", "@payee:=<Blue Bottle>")]
    #[case("@payee:\"*\"", "@payee:<*>")]
    #[case("ACCOUNT:Food", "ACCOUNT:<Food>")]
    #[case("account:Expenses:Food", "account:<Expenses:Food>")]
    #[case("amount:>=100", "amount:>=<100>")]
    #[case("amount:A$100..200", "amount:<A$100>..<200>")]
    #[case("date:2026-01..", "date:<2026-01>..<>")]
    #[case("date:..2026-03", "date:<>..<2026-03>")]
    #[case("description:..", "description:<>..<>")]
    #[case("tag:*", "tag:*")]
    #[case("-tag:me", "(not tag:<me>)")]
    #[case("not tag:me", "(not tag:<me>)")]
    #[case("- tag:me", "(not tag:<me>)")]
    #[case("--a", "(not (not <a>))")]
    #[case("a and b", "(and <a> <b>)")]
    #[case("a or b c", "(or <a> (and <b> <c>))")]
    #[case("A OR b", "(or <A> <b>)")]
    #[case("(a or b) c", "(and (or <a> <b>) <c>)")]
    #[case("(a b) c", "(and (and <a> <b>) <c>)")]
    #[case("-(a b)", "(not (and <a> <b>))")]
    #[case(
        "any:(account:Bank amount:>100)",
        "any:((and account:<Bank> amount:><100>))"
    )]
    #[case("-any:( tag:me )", "(not any:(tag:<me>))")]
    #[case("re-x", "<re-x>")]
    fn parses(#[case] text: &str, #[case] expected: &str) {
        assert_eq!(shape(&parse(text).expect("parses")), expected);
    }

    #[rstest]
    #[case("", "the query is empty", 0, 0)]
    #[case("  ", "the query is empty", 2, 2)]
    #[case("(a", "unclosed '('", 0, 1)]
    #[case("a)", "unmatched ')'", 1, 2)]
    #[case("()", "empty parentheses", 0, 2)]
    #[case("or a", "expected an expression before 'or'", 0, 2)]
    #[case("a or", "expected an expression after 'or'", 2, 4)]
    #[case("a and", "expected an expression after 'and'", 2, 5)]
    #[case("-", "expected an expression after '-'", 0, 1)]
    #[case("not", "expected an expression after 'not'", 0, 3)]
    #[case("account:", "expected a value after 'account:'", 8, 8)]
    #[case("amount:>", "expected a value after 'amount:>'", 7, 8)]
    #[case("@payee", "expected ':' after '@payee'", 0, 6)]
    #[case(
        "12:30",
        "'12' is not a field name; quote text that contains ':'",
        0,
        2
    )]
    #[case("\"abc", "unclosed '\"'", 0, 4)]
    #[case(
        r#""a\nb""#,
        r#"unknown escape '\n'; only \" and \\ are allowed"#,
        2,
        4
    )]
    #[case("amount:>1..2", "a comparison takes one value, not a range", 7, 12)]
    #[case("@payee:\"a\"b", "expected a space before 'b'", 10, 11)]
    #[case("any:(a", "unclosed '('", 4, 5)]
    fn rejects(
        #[case] text: &str,
        #[case] message: &str,
        #[case] start: usize,
        #[case] end: usize,
    ) {
        assert_eq!(
            parse(text),
            Err(ParseError::new(message, Span::new(start, end)))
        );
    }

    #[test]
    fn spans_cover_multibyte_text() {
        let expr = parse("café @payee:\"Zoë\"").expect("parses");
        let Expr::And(items, span) = expr else {
            panic!("expected and")
        };
        assert_eq!(span, Span::new(0, 19));
        assert_eq!(items.first().map(Expr::span), Some(Span::new(0, 5)));
        assert_eq!(items.get(1).map(Expr::span), Some(Span::new(6, 19)));
    }

    #[test]
    fn accepts_sixty_four_nested_groups() {
        let text = format!("{}a{}", "(".repeat(64), ")".repeat(64));
        assert_eq!(shape(&parse(&text).expect("parses")), "<a>");
    }

    #[test]
    fn accepts_sixty_four_stacked_negations() {
        let text = format!("{}a", "-".repeat(64));
        assert_eq!(
            shape(&parse(&text).expect("parses"))
                .matches("(not ")
                .count(),
            64
        );
    }

    #[rstest]
    #[case("-", 65)]
    #[case("not ", 65)]
    #[case("-", 20_000)]
    fn rejects_too_many_negations(#[case] prefix: &str, #[case] count: usize) {
        let text = format!("{}a", prefix.repeat(count));
        let error = parse(&text).expect_err("too deep");
        assert_eq!(
            error.message,
            "the query nests more than 64 negations and groups"
        );
    }

    #[test]
    fn negations_and_groups_share_the_depth_budget() {
        let text = format!("{}{}a{}", "-".repeat(33), "(".repeat(32), ")".repeat(32));
        let error = parse(&text).expect_err("too deep");
        assert_eq!(error.message, "the query nests more than 64 groups");
    }

    #[rstest]
    #[case("/x/")]
    #[case("//")]
    fn rejects_a_slash_wrapped_word(#[case] text: &str) {
        let error = parse(text).expect_err("reserved");
        assert_eq!(
            error.message,
            "regular expressions are reserved; quote it to search for the text"
        );
    }

    #[test]
    fn a_quoted_slash_wrapped_word_is_text() {
        assert_eq!(shape(&parse("\"/x/\"").expect("parses")), "</x/>");
    }

    #[test]
    fn rejects_a_sixty_fifth_nested_group() {
        let text = format!("{}a{}", "(".repeat(65), ")".repeat(65));
        assert_eq!(
            parse(&text),
            Err(ParseError::new(
                "the query nests more than 64 groups",
                Span::new(64, 65)
            ))
        );
    }
}
