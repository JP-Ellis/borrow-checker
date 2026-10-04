//! Token kinds for syntax highlighting, from query text that need not parse.

use crate::parser::OPERATORS;
use crate::parser::bare_len;
use crate::parser::is_field_name;
use crate::parser::keyword;
use crate::span::Span;

/// What a run of query text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the palette styles every kind; a new kind needs a new style"
)]
pub enum TokenKind {
    /// A built-in field name and its colon: `account:`.
    Field,
    /// A metadata key with its `@` and colon: `@payee:`.
    Key,
    /// A comparison operator, a bare `*`, or a range's `..`.
    Operator,
    /// A criterion value, quoted or bare.
    Value,
    /// Free text, searched in the description.
    Text,
    /// `or`, `and`, `not`, or a `-` negation.
    Keyword,
    /// `(` or `)`.
    Paren,
}

/// A classified run of query text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Token {
    /// What the run is.
    pub kind: TokenKind,
    /// Where it is.
    pub span: Span,
}

impl Token {
    /// Creates a token.
    ///
    /// # Arguments
    ///
    /// * `kind` - What the run is.
    /// * `span` - Where it is.
    #[must_use]
    pub const fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Classifies every run of `text`, in order. Whitespace belongs to no token.
///
/// The text need not parse. An unterminated quote runs to the end, and a field
/// name the catalog lacks still highlights as a field.
///
/// # Arguments
///
/// * `text` - The query text.
#[must_use]
pub fn tokens(text: &str) -> Vec<Token> {
    let mut lexer = Lexer {
        text,
        pos: 0,
        out: Vec::new(),
    };
    lexer.run();
    lexer.out
}

/// The scan position and the tokens so far.
struct Lexer<'a> {
    /// The whole text.
    text: &'a str,
    /// The byte offset reached.
    pos: usize,
    /// The tokens found.
    out: Vec<Token>,
}

impl Lexer<'_> {
    /// The text from the position on.
    fn rest(&self) -> &str {
        self.text.get(self.pos..).unwrap_or_default()
    }

    /// Records `len` bytes from the position as `kind` and moves past them.
    fn push(&mut self, kind: TokenKind, len: usize) {
        let end = self.pos.saturating_add(len).min(self.text.len());
        if end > self.pos {
            self.out.push(Token::new(kind, Span::new(self.pos, end)));
        }
        self.pos = end;
    }

    /// Classifies runs until the text ends.
    fn run(&mut self) {
        while let Some(c) = self.rest().chars().next() {
            if c.is_whitespace() {
                self.pos = self.pos.saturating_add(c.len_utf8());
                continue;
            }
            match c {
                '(' | ')' => self.push(TokenKind::Paren, 1),
                '"' => {
                    let len = quoted_len(self.rest());
                    self.push(TokenKind::Text, len);
                }
                '-' => self.push(TokenKind::Keyword, 1),
                _ => self.word(),
            }
        }
    }

    /// A bare run: a keyword, a `field:criterion` term, or free text.
    fn word(&mut self) {
        let len = bare_len(self.rest());
        let run = self.rest().get(..len).unwrap_or_default();
        if keyword(run).is_some() {
            self.push(TokenKind::Keyword, len);
            return;
        }
        let head = run.find(':').and_then(|colon| {
            let name = run.get(..colon)?;
            let (meta, bare) = name
                .strip_prefix('@')
                .map_or((false, name), |rest| (true, rest));
            is_field_name(bare).then_some((colon, meta))
        });
        let Some((colon, meta)) = head else {
            self.push(TokenKind::Text, len);
            return;
        };
        let kind = if meta {
            TokenKind::Key
        } else {
            TokenKind::Field
        };
        self.push(kind, colon.saturating_add(1));
        self.criterion();
    }

    /// Everything after a term's colon: an operator, `*`, and values split by `..`.
    fn criterion(&mut self) {
        if let Some((literal, _)) = OPERATORS
            .iter()
            .find(|(literal, _)| self.rest().starts_with(literal))
        {
            self.push(TokenKind::Operator, literal.len());
        }
        let rest = self.rest();
        if rest.starts_with('*')
            && rest.get(1..).is_none_or(|after| {
                after.is_empty() || after.starts_with(|c: char| c.is_whitespace() || c == ')')
            })
        {
            self.push(TokenKind::Operator, 1);
            return;
        }
        loop {
            if self.rest().starts_with('"') {
                let len = quoted_len(self.rest());
                self.push(TokenKind::Value, len);
            } else {
                let len = bare_len(self.rest());
                let run = self.rest().get(..len).unwrap_or_default();
                if let Some(at) = run.find("..") {
                    self.push(TokenKind::Value, at);
                    self.push(TokenKind::Operator, 2);
                    continue;
                }
                self.push(TokenKind::Value, len);
            }
            if self.rest().starts_with("..") {
                self.push(TokenKind::Operator, 2);
                continue;
            }
            break;
        }
    }
}

/// The byte length of the quoted string at the start of `s`, quotes included;
/// all of `s` when the string is unterminated.
fn quoted_len(s: &str) -> usize {
    let mut escaped = false;
    for (i, c) in s.char_indices().skip(1) {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return i.saturating_add(1);
        }
    }
    s.len()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::TokenKind as K;
    use super::tokens;

    /// Each token's kind with the text it covers.
    fn runs(text: &str) -> Vec<(K, &str)> {
        tokens(text)
            .into_iter()
            .map(|t| {
                (
                    t.kind,
                    text.get(t.span.start..t.span.end).unwrap_or_default(),
                )
            })
            .collect()
    }

    #[rstest]
    #[case("account:Food", vec![(K::Field, "account:"), (K::Value, "Food")])]
    #[case("@payee:\"Blue Bottle\"", vec![(K::Key, "@payee:"), (K::Value, "\"Blue Bottle\"")])]
    #[case(
        "amount:>=A$100..200",
        vec![(K::Field, "amount:"), (K::Operator, ">="), (K::Value, "A$100"), (K::Operator, ".."), (K::Value, "200")]
    )]
    #[case(
        "-tag:me or coffee",
        vec![(K::Keyword, "-"), (K::Field, "tag:"), (K::Value, "me"), (K::Keyword, "or"), (K::Text, "coffee")]
    )]
    #[case(
        "not (a OR b)",
        vec![(K::Keyword, "not"), (K::Paren, "("), (K::Text, "a"), (K::Keyword, "OR"), (K::Text, "b"), (K::Paren, ")")]
    )]
    #[case(
        "any:(tag:*)",
        vec![(K::Field, "any:"), (K::Paren, "("), (K::Field, "tag:"), (K::Operator, "*"), (K::Paren, ")")]
    )]
    #[case("\"re: invoice\" café", vec![(K::Text, "\"re: invoice\""), (K::Text, "café")])]
    #[case("date:..\"2026-03\"", vec![(K::Field, "date:"), (K::Operator, ".."), (K::Value, "\"2026-03\"")])]
    #[case("12:30", vec![(K::Text, "12:30")])]
    #[case("@payee:\"unterminated", vec![(K::Key, "@payee:"), (K::Value, "\"unterminated")])]
    #[case("account:", vec![(K::Field, "account:")])]
    #[case("re-x", vec![(K::Text, "re-x")])]
    #[case("tag:*x", vec![(K::Field, "tag:"), (K::Value, "*x")])]
    #[case("", vec![])]
    fn classifies_runs(#[case] text: &str, #[case] expected: Vec<(K, &str)>) {
        assert_eq!(runs(text), expected);
    }

    #[test]
    fn spans_land_on_char_boundaries() {
        let text = "@payee:\"Zoë\" café -(x..ÿ";
        for token in tokens(text) {
            assert!(text.is_char_boundary(token.span.start), "{token:?}");
            assert!(text.is_char_boundary(token.span.end), "{token:?}");
        }
    }

    #[test]
    fn a_long_input_finishes() {
        let text = "a..".repeat(5_000);
        let found = tokens(&format!("date:{text}"));
        // `date:`, then a value and a `..` for each of the 5,000 repeats.
        assert_eq!(found.len(), 10_001);
        assert_eq!(
            found
                .iter()
                .rev()
                .take(2)
                .map(|t| t.kind)
                .collect::<Vec<_>>(),
            vec![K::Operator, K::Value]
        );
        assert_eq!(tokens(&"(".repeat(20_000)).len(), 20_000);
    }
}
