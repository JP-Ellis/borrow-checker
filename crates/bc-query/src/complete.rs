//! What the cursor sits in, for autocomplete, from text that need not parse.

use crate::ast::Field;
use crate::ast::Op;
use crate::parser::Keyword;
use crate::parser::OPERATORS;
use crate::parser::is_field_name;
use crate::parser::keyword;
use crate::span::Span;

/// What to offer at the cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the palette matches every kind; a new kind needs a new suggestion list"
)]
pub enum CompletionKind {
    /// A new term: fields, keys, `any:(`, `(`.
    Start,
    /// After a complete term: `or`, `)`, or a new term.
    AfterTerm,
    /// Inside free text: nothing to suggest.
    Text,
    /// A bare word that may become a field name.
    Field {
        /// The text typed so far.
        partial: String,
    },
    /// A metadata key after `@`.
    Key {
        /// The key typed so far, without `@`.
        partial: String,
    },
    /// Just after `field:`: operators and values.
    Operator {
        /// The field.
        field: Field,
    },
    /// A value for `field`.
    Value {
        /// The field.
        field: Field,
        /// The operator typed, [`Op::Match`] when none.
        op: Op,
        /// The value typed so far. After an opening quote this is the raw text
        /// following it, with escapes not applied.
        partial: String,
    },
}

/// What the cursor sits in.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CompletionContext {
    /// What to offer.
    pub kind: CompletionKind,
    /// The text a chosen suggestion replaces.
    pub replace: Span,
    /// How many `(` are open before the cursor.
    pub open_groups: usize,
}

impl CompletionContext {
    /// Creates a completion context.
    ///
    /// # Arguments
    ///
    /// * `kind` - What to offer.
    /// * `replace` - The text a chosen suggestion replaces.
    /// * `open_groups` - How many `(` are open before the cursor.
    #[must_use]
    pub const fn new(kind: CompletionKind, replace: Span, open_groups: usize) -> Self {
        Self {
            kind,
            replace,
            open_groups,
        }
    }
}

/// Reports what the cursor sits in, reading only the text before it.
///
/// The text need not parse. A cursor past the end, or inside a multi-byte
/// character, moves back to the nearest character boundary.
///
/// # Arguments
///
/// * `text` - The query text.
/// * `cursor` - The cursor's byte offset.
#[must_use]
pub fn parse_partial(text: &str, cursor: usize) -> CompletionContext {
    let mut end = cursor.min(text.len());
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let prefix = text.get(..end).unwrap_or_default();
    let scanned = scan(prefix);
    let (kind, replace) = classify(prefix, &scanned, end);
    CompletionContext::new(kind, replace, scanned.depth)
}

/// What a left-to-right scan of the prefix found.
struct Scan {
    /// Open parentheses.
    depth: usize,
    /// Where the token under the cursor starts.
    token_start: usize,
    /// The opening quote's offset, when the cursor is inside a string.
    open_quote: Option<usize>,
}

/// Tracks parentheses, quotes and token boundaries up to the cursor.
fn scan(prefix: &str) -> Scan {
    let mut out = Scan {
        depth: 0,
        token_start: 0,
        open_quote: None,
    };
    let mut escaped = false;
    for (i, c) in prefix.char_indices() {
        if out.open_quote.is_some() {
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                out.open_quote = None;
            }
            continue;
        }
        let after = i.saturating_add(c.len_utf8());
        match c {
            '"' => out.open_quote = Some(i),
            '(' => {
                out.depth = out.depth.saturating_add(1);
                out.token_start = after;
            }
            ')' => {
                out.depth = out.depth.saturating_sub(1);
                out.token_start = after;
            }
            _ if c.is_whitespace() => out.token_start = after,
            _ => {}
        }
    }
    out
}

/// Classifies the token under the cursor.
fn classify(prefix: &str, scan: &Scan, end: usize) -> (CompletionKind, Span) {
    let raw = prefix.get(scan.token_start..).unwrap_or_default();
    let token = raw.trim_start_matches('-');
    let start = end.saturating_sub(token.len());
    let here = Span::new(end, end);

    if token.is_empty() {
        let kind = if starts_new_term(prefix.get(..start).unwrap_or_default()) {
            CompletionKind::Start
        } else {
            CompletionKind::AfterTerm
        };
        return (kind, here);
    }

    let field_head = token.find(':').and_then(|colon| {
        let head = token.get(..colon).unwrap_or_default();
        let (meta, name) = head.strip_prefix('@').map_or((false, head), |n| (true, n));
        is_field_name(name).then(|| {
            let field = Field::new(
                &name.to_ascii_lowercase(),
                meta,
                Span::new(start, start.saturating_add(head.len())),
            );
            (field, colon.saturating_add(1))
        })
    });
    let Some((field, value_at)) = field_head else {
        if let Some(quote) = scan.open_quote {
            return (CompletionKind::Text, Span::new(quote, end));
        }
        if token.contains('"') {
            return (CompletionKind::AfterTerm, here);
        }
        if token.contains(':') {
            return (CompletionKind::Text, Span::new(start, end));
        }
        return match token.strip_prefix('@') {
            Some(key) => (
                CompletionKind::Key {
                    partial: key.to_owned(),
                },
                Span::new(start, end),
            ),
            None => (
                CompletionKind::Field {
                    partial: token.to_owned(),
                },
                Span::new(start, end),
            ),
        };
    };

    let after_colon = token.get(value_at..).unwrap_or_default();
    let (op, op_len) = OPERATORS
        .iter()
        .find(|(literal, _)| after_colon.starts_with(literal))
        .map_or((Op::Match, 0), |(literal, op)| (*op, literal.len()));
    let value = after_colon.get(op_len..).unwrap_or_default();

    if let Some(quote) = scan.open_quote {
        let partial = prefix.get(quote.saturating_add(1)..).unwrap_or_default();
        return (
            CompletionKind::Value {
                field,
                op,
                partial: partial.to_owned(),
            },
            Span::new(quote, end),
        );
    }
    if value.is_empty() && op == Op::Match {
        return (CompletionKind::Operator { field }, here);
    }
    if value.ends_with('"') {
        return (CompletionKind::AfterTerm, here);
    }
    let partial = value.rsplit_once("..").map_or(value, |(_, hi)| hi);
    (
        CompletionKind::Value {
            field,
            op,
            partial: partial.to_owned(),
        },
        Span::new(end.saturating_sub(partial.len()), end),
    )
}

/// Whether a new term starts after `before`: nothing yet, an opening
/// parenthesis, a negation, or a keyword.
///
/// Like the parser, a `-` negates only at the start of a token, and a keyword
/// is a bare run that whitespace or a parenthesis delimits.
fn starts_new_term(before: &str) -> bool {
    let trimmed = before.trim_end();
    let without_dashes = trimmed.trim_end_matches('-');
    if without_dashes.is_empty() || without_dashes.ends_with('(') {
        return true;
    }
    if without_dashes.len() < trimmed.len() {
        return without_dashes.ends_with(char::is_whitespace);
    }
    let last = trimmed
        .rsplit(|c: char| c.is_whitespace() || matches!(c, '(' | ')'))
        .next()
        .unwrap_or_default()
        .trim_start_matches('-');
    matches!(
        keyword(last),
        Some(Keyword::Or | Keyword::And | Keyword::Not)
    )
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    /// A context.
    fn ctx(
        kind: CompletionKind,
        start: usize,
        end: usize,
        open_groups: usize,
    ) -> CompletionContext {
        CompletionContext {
            kind,
            replace: Span::new(start, end),
            open_groups,
        }
    }

    /// A field reference.
    fn field(name: &str, meta: bool, start: usize, end: usize) -> Field {
        Field::new(name, meta, Span::new(start, end))
    }

    /// Partial text.
    fn s(text: &str) -> String {
        text.to_owned()
    }

    #[rstest]
    #[case("", ctx(CompletionKind::Start, 0, 0, 0))]
    #[case("acc", ctx(CompletionKind::Field { partial: s("acc") }, 0, 3, 0))]
    #[case("a ", ctx(CompletionKind::AfterTerm, 2, 2, 0))]
    #[case("a or ", ctx(CompletionKind::Start, 5, 5, 0))]
    #[case("-", ctx(CompletionKind::Start, 1, 1, 0))]
    #[case("-ta", ctx(CompletionKind::Field { partial: s("ta") }, 1, 3, 0))]
    #[case("@pa", ctx(CompletionKind::Key { partial: s("pa") }, 0, 3, 0))]
    #[case("account:", ctx(CompletionKind::Operator { field: field("account", false, 0, 7) }, 8, 8, 0))]
    #[case("account:Foo", ctx(CompletionKind::Value {
        field: field("account", false, 0, 7), op: Op::Match, partial: s("Foo") }, 8, 11, 0))]
    #[case("amount:>=1", ctx(CompletionKind::Value {
        field: field("amount", false, 0, 6), op: Op::Ge, partial: s("1") }, 9, 10, 0))]
    #[case("amount:>", ctx(CompletionKind::Value {
        field: field("amount", false, 0, 6), op: Op::Gt, partial: s("") }, 8, 8, 0))]
    #[case("date:2026-01..20", ctx(CompletionKind::Value {
        field: field("date", false, 0, 4), op: Op::Match, partial: s("20") }, 14, 16, 0))]
    #[case("@payee:\"Blue Bo", ctx(CompletionKind::Value {
        field: field("payee", true, 0, 6), op: Op::Match, partial: s("Blue Bo") }, 7, 15, 0))]
    #[case("@PAYEE:x", ctx(CompletionKind::Value {
        field: field("payee", true, 0, 6), op: Op::Match, partial: s("x") }, 7, 8, 0))]
    #[case("@payee:\"Blue\" ", ctx(CompletionKind::AfterTerm, 14, 14, 0))]
    #[case("@payee:\"Blue\"", ctx(CompletionKind::AfterTerm, 13, 13, 0))]
    #[case("(a or (b", ctx(CompletionKind::Field { partial: s("b") }, 7, 8, 2))]
    #[case("any:(", ctx(CompletionKind::Start, 5, 5, 1))]
    #[case("a)", ctx(CompletionKind::AfterTerm, 2, 2, 0))]
    #[case("- ", ctx(CompletionKind::Start, 2, 2, 0))]
    #[case("a - ", ctx(CompletionKind::Start, 4, 4, 0))]
    #[case("re- ", ctx(CompletionKind::AfterTerm, 4, 4, 0))]
    #[case("date:2026- ", ctx(CompletionKind::AfterTerm, 11, 11, 0))]
    #[case("(not ", ctx(CompletionKind::Start, 5, 5, 1))]
    #[case("any:(not ", ctx(CompletionKind::Start, 9, 9, 1))]
    #[case("(a)or ", ctx(CompletionKind::Start, 6, 6, 0))]
    #[case("\"some te", ctx(CompletionKind::Text, 0, 8, 0))]
    #[case("12:3", ctx(CompletionKind::Text, 0, 4, 0))]
    fn at_end(#[case] text: &str, #[case] expected: CompletionContext) {
        assert_eq!(parse_partial(text, text.len()), expected);
    }

    #[test]
    fn cursor_mid_text_reads_only_the_prefix() {
        assert_eq!(
            parse_partial("account:Food tag:me", 4),
            ctx(CompletionKind::Field { partial: s("acco") }, 0, 4, 0)
        );
    }

    #[test]
    fn cursor_inside_a_multibyte_char_clamps_back() {
        // 'ï' occupies bytes 2..4; 3 is inside it.
        assert_eq!(
            parse_partial("naïve", 3),
            ctx(CompletionKind::Field { partial: s("na") }, 0, 2, 0)
        );
    }

    #[test]
    fn cursor_past_the_end_clamps_to_the_end() {
        assert_eq!(
            parse_partial("acc", 99),
            ctx(CompletionKind::Field { partial: s("acc") }, 0, 3, 0)
        );
    }

    #[rstest]
    #[case("amount:\"150 AUD\"..", ctx(CompletionKind::Value {
        field: field("amount", false, 0, 6), op: Op::Match, partial: s("") }, 18, 18, 0))]
    #[case("date:..\"2026", ctx(CompletionKind::Value {
        field: field("date", false, 0, 4), op: Op::Match, partial: s("2026") }, 7, 12, 0))]
    #[case("@payee:\"Zoë", ctx(CompletionKind::Value {
        field: field("payee", true, 0, 6), op: Op::Match, partial: s("Zoë") }, 7, 12, 0))]
    #[case("@payee:\"Zoë\" ", ctx(CompletionKind::AfterTerm, 14, 14, 0))]
    #[case("café", ctx(CompletionKind::Field { partial: s("café") }, 0, 5, 0))]
    #[case("/ab/", ctx(CompletionKind::Field { partial: s("/ab/") }, 0, 4, 0))]
    #[case("\"unterminated", ctx(CompletionKind::Text, 0, 13, 0))]
    fn at_end_edge_cases(#[case] text: &str, #[case] expected: CompletionContext) {
        assert_eq!(parse_partial(text, text.len()), expected);
    }

    #[test]
    fn deep_nesting_counts_groups_without_recursing() {
        let text = "(".repeat(20_000);
        let got = parse_partial(&text, text.len());
        assert_eq!(got, ctx(CompletionKind::Start, 20_000, 20_000, 20_000));
    }

    #[test]
    fn every_cursor_in_non_ascii_text_is_safe() {
        let text = "@payee:\"Zoë\" café -(x";
        for cursor in 0..=text.len() + 2 {
            let got = parse_partial(text, cursor);
            assert!(text.is_char_boundary(got.replace.start), "{cursor}");
            assert!(text.is_char_boundary(got.replace.end), "{cursor}");
        }
    }
}
