//! [`Expr`] to canonical query text.

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::ast::Term;
use crate::parser::OPERATORS;
use crate::parser::keyword;

/// Writes `expr` as canonical query text.
///
/// Single spaces separate conjuncts, `or` separates alternatives, negation
/// prints as `-`, and every nested `and`/`or` is parenthesised. A value is
/// quoted when its bare text would parse differently in any position a value
/// can take, so `*`, `=5` and `a..b` are quoted even where they would not
/// need it, such as a free word or after `:=`.
///
/// # Arguments
///
/// * `expr` - The expression to print.
#[must_use]
pub fn print(expr: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, expr);
    out
}

/// Appends `expr` to `out`.
fn write_expr(out: &mut String, expr: &Expr) {
    match expr {
        Expr::Or(items, _) => write_joined(out, items, " or "),
        Expr::And(items, _) => write_joined(out, items, " "),
        Expr::Not(inner, _) => {
            out.push('-');
            write_operand(out, inner);
        }
        Expr::Term(term) => write_term(out, term),
        Expr::Word(value) => write_text(out, &value.text, word_needs_quotes(&value.text)),
    }
}

/// Appends `items` separated by `separator`.
fn write_joined(out: &mut String, items: &[Expr], separator: &str) {
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            out.push_str(separator);
        }
        write_operand(out, item);
    }
}

/// Appends `expr`, parenthesised when it is an `and` or `or`.
fn write_operand(out: &mut String, expr: &Expr) {
    if matches!(expr, Expr::Or(..) | Expr::And(..)) {
        out.push('(');
        write_expr(out, expr);
        out.push(')');
    } else {
        write_expr(out, expr);
    }
}

/// Appends `field:criterion`.
fn write_term(out: &mut String, term: &Term) {
    if term.field.meta {
        out.push('@');
    }
    out.push_str(&term.field.name);
    out.push(':');
    match &term.criterion {
        Criterion::Any(_) => out.push('*'),
        Criterion::Compare { op, value, .. } => {
            out.push_str(op.as_str());
            write_text(out, &value.text, value_needs_quotes(&value.text));
        }
        Criterion::Range { lo, hi, .. } => {
            if let Some(low) = lo {
                write_text(out, &low.text, range_end_needs_quotes(&low.text));
            }
            out.push_str("..");
            if let Some(high) = hi {
                write_text(out, &high.text, range_end_needs_quotes(&high.text));
            }
        }
        Criterion::Group(inner, _) => {
            out.push('(');
            write_expr(out, inner);
            out.push(')');
        }
    }
}

/// Appends `text`, quoted and escaped when `quote` is set.
fn write_text(out: &mut String, text: &str, quote: bool) {
    if !quote {
        out.push_str(text);
        return;
    }
    out.push('"');
    for c in text.chars() {
        if matches!(c, '"' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
}

/// Whether a criterion value must be quoted to parse back as itself.
fn value_needs_quotes(text: &str) -> bool {
    text.is_empty()
        || text == "*"
        || OPERATORS
            .iter()
            .any(|(literal, _)| text.starts_with(literal))
        || text.contains("..")
        || text
            .chars()
            .any(|c| c.is_whitespace() || matches!(c, '(' | ')' | '"' | '\\'))
}

/// Whether a range end must be quoted. A dot at the inner edge would merge
/// with the `..` separator.
fn range_end_needs_quotes(text: &str) -> bool {
    value_needs_quotes(text) || text.starts_with('.') || text.ends_with('.')
}

/// Whether free text must be quoted to parse back as a word.
fn word_needs_quotes(text: &str) -> bool {
    value_needs_quotes(text)
        || text.contains(':')
        || text.starts_with(['-', '@'])
        || keyword(text).is_some()
        || (text.len() >= 2 && text.starts_with('/') && text.ends_with('/'))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use proptest::prelude::*;
    use rstest::rstest;

    use super::*;
    use crate::ast::Criterion;
    use crate::ast::Field;
    use crate::ast::Op;
    use crate::ast::Term;
    use crate::ast::Value;
    use crate::parser::parse;
    use crate::span::Span;

    #[rstest]
    #[case("blue   bottle", "blue bottle")]
    #[case("a and b", "a b")]
    #[case("NOT a", "-a")]
    #[case("a b or c", "(a b) or c")]
    #[case("(a or b) c", "(a or b) c")]
    #[case("-(a or b)", "-(a or b)")]
    #[case("@payee:\"Blue Bottle\"", "@payee:\"Blue Bottle\"")]
    #[case("@payee:\"coffee\"", "@payee:coffee")]
    #[case("\"or\"", "\"or\"")]
    #[case("\"a:b\"", "\"a:b\"")]
    #[case("\"-x\"", "\"-x\"")]
    #[case("@payee:\"*\"", "@payee:\"*\"")]
    #[case("@payee:\"=5\"", "@payee:\"=5\"")]
    #[case("@payee:\"a..b\"", "@payee:\"a..b\"")]
    #[case("amount:A$100..200", "amount:A$100..200")]
    #[case("date:..2026", "date:..2026")]
    #[case("amount:>=100", "amount:>=100")]
    #[case("date:\"a.\"..b", "date:\"a.\"..b")]
    #[case("date:\".x\"..", "date:\".x\"..")]
    #[case("\"/x/\"", "\"/x/\"")]
    #[case("-\"-x\"", "-\"-x\"")]
    #[case("any:( tag:me )", "any:(tag:me)")]
    #[case(r#"description:"say \"hi\"""#, r#"description:"say \"hi\"""#)]
    fn prints_canonically(#[case] text: &str, #[case] expected: &str) {
        assert_eq!(print(&parse(text).expect("parses")), expected);
    }

    /// Value text, quoted by the printer when it must be. Mixes arbitrary
    /// Unicode with strings built from the characters and words that decide
    /// quoting.
    fn text() -> impl Strategy<Value = String> {
        prop_oneof![
            ".{0,8}",
            "[ab\\-@=<>:*./\"\\\\() é]{0,6}",
            prop::sample::select(vec![
                "or", "and", "not", "OR", "*", "..", "/x/", "a.", ".x", "",
            ])
            .prop_map(str::to_owned),
        ]
    }

    /// Text a range end holds: often bare, sometimes needing quotes.
    fn range_end() -> impl Strategy<Value = String> {
        prop_oneof!["[a-zA-Z0-9]{1,6}", text()]
    }

    /// A syntactically valid field name.
    fn name() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9_-]{0,6}"
    }

    /// Any operator.
    fn op() -> impl Strategy<Value = Op> {
        prop_oneof![
            Just(Op::Match),
            Just(Op::Equal),
            Just(Op::Gt),
            Just(Op::Ge),
            Just(Op::Lt),
            Just(Op::Le)
        ]
    }

    /// A value with no span.
    fn value(text: &str) -> Value {
        Value::new(text, Span::default())
    }

    /// A criterion other than a group.
    fn leaf_criterion() -> impl Strategy<Value = Criterion> {
        prop_oneof![
            Just(Criterion::Any(Span::default())),
            (op(), text()).prop_map(|(op, t)| Criterion::Compare {
                op,
                value: value(&t),
                span: Span::default()
            }),
            (
                proptest::option::of(range_end()),
                proptest::option::of(range_end())
            )
                .prop_map(|(lo, hi)| {
                    Criterion::Range {
                        lo: lo.as_deref().map(value),
                        hi: hi.as_deref().map(value),
                        span: Span::default(),
                    }
                }),
        ]
    }

    /// A term with no span.
    fn term(name: &str, meta: bool, criterion: Criterion) -> Expr {
        Expr::Term(Term::new(
            Field::new(name, meta, Span::default()),
            criterion,
            Span::default(),
        ))
    }

    /// Any expression the parser can produce.
    fn expr() -> impl Strategy<Value = Expr> {
        let leaf = prop_oneof![
            text().prop_map(|t| Expr::Word(value(&t))),
            (name(), any::<bool>(), leaf_criterion()).prop_map(|(n, m, c)| term(&n, m, c)),
        ];
        leaf.prop_recursive(4, 32, 4, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 2..4)
                    .prop_map(|v| Expr::Or(v, Span::default())),
                proptest::collection::vec(inner.clone(), 2..4)
                    .prop_map(|v| Expr::And(v, Span::default())),
                inner
                    .clone()
                    .prop_map(|e| Expr::Not(Box::new(e), Span::default())),
                (name(), any::<bool>(), inner).prop_map(|(n, m, e)| {
                    term(&n, m, Criterion::Group(Box::new(e), Span::default()))
                }),
            ]
        })
    }

    proptest! {
        #[test]
        fn print_round_trips(e in expr()) {
            let printed = print(&e);
            let reparsed = parse(&printed).map(|r| r.without_spans());
            prop_assert_eq!(reparsed, Ok(e), "printed as {:?}", printed);
        }
    }
}
