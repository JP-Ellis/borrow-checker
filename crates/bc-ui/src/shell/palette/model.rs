//! The palette's behaviour, free of Leptos: what the input shows, what the
//! hint line says, what the dropdown offers, and what Tab and Enter do.

use bc_query::Catalog;
use bc_query::Expr;
use bc_query::Severity;
use bc_query::Span;
use bc_query::describe::describe;
use bc_query::highlight::Token;
use bc_query::highlight::TokenKind;
use bc_query::highlight::tokens;
use bc_query::parse;
use bc_query::parse_partial;
use bc_query::resolve;
use bc_query::suggest::Suggestion;
use bc_query::suggest::SuggestionKind;
use bc_query::suggest::suggest;
use jiff::civil::Date;

/// The hint line for an empty palette.
pub const EMPTY_HINT: &str = "Type to search descriptions, or a field: account: tag: @payee: …";

/// The hint line for input starting with `>`, which is reserved for commands.
pub const COMMAND_HINT: &str = "'>' starts a command; there are none yet";

/// A run of the input drawn in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// The text.
    pub text: String,
    /// The token it belongs to; `None` for whitespace.
    pub kind: Option<TokenKind>,
    /// The worst underline over it.
    pub mark: Option<Severity>,
}

/// One line of the hint area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintLine {
    /// The diagnostic's severity; `None` for plain guidance.
    pub severity: Option<Severity>,
    /// The words.
    pub text: String,
}

impl HintLine {
    /// Creates a line.
    fn new(severity: Option<Severity>, text: impl Into<String>) -> Self {
        Self {
            severity,
            text: text.into(),
        }
    }
}

/// What Enter would commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ready {
    /// Nothing: the input is blank.
    Blank,
    /// This query.
    Query(Expr),
    /// Nothing: an error blocks the commit.
    Blocked,
}

/// Everything the palette shows for one text and cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Analysis {
    /// The input, split into styled runs.
    pub segments: Vec<Segment>,
    /// The hint area, errors first.
    pub hints: Vec<HintLine>,
    /// The dropdown, best first.
    pub suggestions: Vec<Suggestion>,
    /// The text a chosen suggestion replaces.
    pub replace: Span,
    /// What Enter would commit.
    pub ready: Ready,
}

/// Analyses `text` with the cursor at byte `cursor`.
///
/// # Arguments
///
/// * `text` - The input.
/// * `cursor` - The cursor's byte offset.
/// * `catalog` - The ledger facts.
/// * `today` - The date that period suggestions start from.
#[must_use]
pub fn analyse<C>(text: &str, cursor: usize, catalog: &C, today: Date) -> Analysis
where
    C: Catalog,
{
    let context = parse_partial(text, cursor);
    let suggestions = if text.trim_start().starts_with('>') {
        Vec::new()
    } else {
        suggest(&context, catalog, today)
    };
    let (ready, hints, marks) = check(text, cursor, catalog);
    Analysis {
        segments: segments(text, &tokens(text), &marks),
        hints,
        suggestions,
        replace: context.replace,
        ready,
    }
}

/// Parses and resolves `text`: what Enter would commit, the hint lines, and
/// the spans to underline.
fn check<C>(text: &str, cursor: usize, catalog: &C) -> (Ready, Vec<HintLine>, Vec<(Span, Severity)>)
where
    C: Catalog,
{
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return (
            Ready::Blank,
            vec![HintLine::new(None, EMPTY_HINT)],
            Vec::new(),
        );
    }
    if trimmed.starts_with('>') {
        return (
            Ready::Blocked,
            vec![HintLine::new(Some(Severity::Error), COMMAND_HINT)],
            Vec::new(),
        );
    }
    let expr = match parse(text) {
        Ok(expr) => expr,
        Err(error) => {
            return (
                Ready::Blocked,
                vec![HintLine::new(Some(Severity::Error), error.message.clone())],
                vec![(widen(text, error.span), Severity::Error)],
            );
        }
    };
    let resolved = resolve(&expr, catalog);
    let mut hints: Vec<HintLine> = Vec::new();
    for severity in [Severity::Error, Severity::Warning, Severity::Hint] {
        for diagnostic in resolved
            .diagnostics
            .iter()
            .filter(|d| d.severity == severity)
        {
            let line = HintLine::new(Some(severity), diagnostic.message.clone());
            if !hints.contains(&line) {
                hints.push(line);
            }
        }
    }
    let marks: Vec<(Span, Severity)> = resolved
        .diagnostics
        .iter()
        .filter(|d| d.severity != Severity::Hint)
        .map(|d| (widen(text, d.span), d.severity))
        .collect();
    if resolved.has_errors() {
        return (Ready::Blocked, hints, marks);
    }
    if let Some(words) = describe(&expr, cursor, catalog) {
        hints.push(HintLine::new(None, words));
    }
    (Ready::Query(expr), hints, marks)
}

/// `span`, or the character before it when it is empty, so an underline shows.
fn widen(text: &str, span: Span) -> Span {
    if span.start < span.end {
        return span;
    }
    let start = text
        .get(..span.start)
        .and_then(|before| before.char_indices().next_back())
        .map_or(span.start, |(at, _)| at);
    Span::new(start, span.end.max(start))
}

/// Orders severities, worst first: the lowest rank is the worst. The chip row
/// picks a conjunct's worst diagnostic with it too.
///
/// # Arguments
///
/// * `severity` - The severity to rank.
#[must_use]
pub const fn severity_rank(severity: Severity) -> u8 {
    match severity {
        Severity::Error => 0,
        Severity::Warning => 1,
        Severity::Hint => 2,
    }
}

/// Splits `text` at every token and underline boundary into styled runs.
///
/// # Arguments
///
/// * `text` - The input.
/// * `tokens` - Its tokens.
/// * `marks` - The spans to underline, with their severity.
#[must_use]
pub fn segments(text: &str, tokens: &[Token], marks: &[(Span, Severity)]) -> Vec<Segment> {
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for token in tokens {
        cuts.extend([token.span.start, token.span.end]);
    }
    for (span, _) in marks {
        cuts.extend([span.start, span.end]);
    }
    cuts.retain(|&cut| cut <= text.len() && text.is_char_boundary(cut));
    cuts.sort_unstable();
    cuts.dedup();
    cuts.windows(2)
        .filter_map(|pair| {
            let start = *pair.first()?;
            let end = *pair.get(1)?;
            let piece = text.get(start..end)?;
            let kind = tokens
                .iter()
                .find(|t| t.span.start <= start && end <= t.span.end)
                .map(|t| t.kind);
            let mark = marks
                .iter()
                .filter(|(span, _)| span.start <= start && end <= span.end)
                .map(|(_, severity)| *severity)
                .min_by_key(|severity| severity_rank(*severity));
            Some(Segment {
                text: piece.to_owned(),
                kind,
                mark,
            })
        })
        .collect()
}

/// `text` with `suggestion` in place of `replace`, and the cursor's byte
/// offset just after the insert.
///
/// A keyword, field or key inserted straight after a term gets a space before
/// it.
///
/// # Arguments
///
/// * `text` - The input.
/// * `replace` - The text the suggestion replaces.
/// * `suggestion` - The suggestion.
#[must_use]
pub fn accept(text: &str, replace: Span, suggestion: &Suggestion) -> (String, usize) {
    let start = replace.start.min(text.len());
    let end = replace.end.clamp(start, text.len());
    let before = text.get(..start).unwrap_or_default();
    let after = text.get(end..).unwrap_or_default();
    let mut out = String::with_capacity(
        text.len()
            .saturating_add(suggestion.insert.len())
            .saturating_add(1),
    );
    out.push_str(before);
    if needs_space(before, suggestion) {
        out.push(' ');
    }
    out.push_str(&suggestion.insert);
    let cursor = out.len();
    out.push_str(after);
    (out, cursor)
}

/// Whether `suggestion` needs a space to stand apart from what precedes it.
fn needs_space(before: &str, suggestion: &Suggestion) -> bool {
    let starts_term = match suggestion.kind {
        SuggestionKind::Field | SuggestionKind::Key => true,
        SuggestionKind::Keyword => suggestion.insert != ")",
        SuggestionKind::Operator
        | SuggestionKind::Account
        | SuggestionKind::Tag
        | SuggestionKind::Value => false,
    };
    starts_term
        && before
            .chars()
            .next_back()
            .is_some_and(|c| !c.is_whitespace() && c != '(' && c != '-')
}

/// What Enter does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Enter {
    /// Commit this query; `None` for blank input.
    Commit(Option<Expr>),
    /// Keep editing this text, with the cursor at this byte offset.
    Edit(String, usize),
}

/// Enter at byte `cursor` with suggestion `selected` highlighted.
///
/// A highlighted suggestion that accepts on Enter is inserted first. The
/// resulting text commits when it has no errors; otherwise editing continues
/// with that text.
///
/// # Arguments
///
/// * `text` - The input.
/// * `cursor` - The cursor's byte offset.
/// * `selected` - The highlighted suggestion's index, when the dropdown shows.
/// * `catalog` - The ledger facts.
/// * `today` - The date that period suggestions start from.
#[must_use]
pub fn enter<C>(
    text: &str,
    cursor: usize,
    selected: Option<usize>,
    catalog: &C,
    today: Date,
) -> Enter
where
    C: Catalog,
{
    let analysis = analyse(text, cursor, catalog, today);
    let chosen = selected
        .and_then(|index| analysis.suggestions.get(index))
        .filter(|s| s.accept_on_enter);
    let (next_text, next_cursor, ready) = match chosen {
        Some(suggestion) => {
            let (inserted, at) = accept(text, analysis.replace, suggestion);
            let ready = analyse(&inserted, at, catalog, today).ready;
            (inserted, at, ready)
        }
        None => (text.to_owned(), cursor, analysis.ready),
    };
    match ready {
        Ready::Blank => Enter::Commit(None),
        Ready::Query(expr) => Enter::Commit(Some(expr)),
        Ready::Blocked => Enter::Edit(next_text, next_cursor),
    }
}

/// The byte offset `units` UTF-16 code units into `text`, as the DOM counts
/// a cursor; the end of `text` when it is shorter.
///
/// # Arguments
///
/// * `text` - The input.
/// * `units` - The DOM's cursor offset.
#[must_use]
pub fn utf16_to_byte(text: &str, units: usize) -> usize {
    let mut seen = 0_usize;
    for (byte, c) in text.char_indices() {
        if seen >= units {
            return byte;
        }
        seen = seen.saturating_add(c.len_utf16());
    }
    text.len()
}

/// The UTF-16 code units before byte `byte` of `text`, as the DOM counts a
/// cursor.
///
/// # Arguments
///
/// * `text` - The input.
/// * `byte` - A byte offset into `text`.
#[must_use]
pub fn byte_to_utf16(text: &str, byte: usize) -> usize {
    text.char_indices()
        .take_while(|(at, _)| *at < byte)
        .fold(0_usize, |units, (_, c)| units.saturating_add(c.len_utf16()))
}

/// A token kind's name, as the highlight's `data-kind` attribute shows it.
#[must_use]
pub const fn kind_name(kind: TokenKind) -> &'static str {
    match kind {
        TokenKind::Field => "field",
        TokenKind::Key => "key",
        TokenKind::Operator => "operator",
        TokenKind::Value => "value",
        TokenKind::Text => "text",
        TokenKind::Keyword => "keyword",
        TokenKind::Paren => "paren",
    }
}

/// A severity's name, as `data-mark` and `data-severity` attributes show it.
#[must_use]
pub const fn severity_name(severity: Severity) -> &'static str {
    match severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
        Severity::Hint => "hint",
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_query::Severity;
    use bc_query::Span;
    use bc_query::catalog::MetaKey;
    use bc_query::catalog::MetaType;
    use bc_query::catalog::PathEntry;
    use bc_query::catalog::Snapshot;
    use bc_query::currency::Commodity;
    use bc_query::highlight::TokenKind;
    use bc_query::highlight::tokens;
    use bc_query::print;
    use bc_query::suggest::Suggestion;
    use bc_query::suggest::SuggestionKind;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    /// The day every test runs on.
    const TODAY: Date = date(2026, 10, 4);

    /// An invented ledger.
    fn catalog() -> Snapshot {
        Snapshot::new(
            vec![
                PathEntry::new("a1", ["Expenses", "Food"]),
                PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
                PathEntry::new("a3", ["Income", "Food"]),
            ],
            vec![PathEntry::new("t1", ["me"])],
            vec![Commodity::new("AUD", Some("A$"), &[])],
            vec![MetaKey::new("km", MetaType::Number, 2)],
        )
    }

    /// The analysis with the cursor at the end of `text`.
    fn at_end(text: &str) -> Analysis {
        analyse(text, text.len(), &catalog(), TODAY)
    }

    /// A hint line.
    fn line(severity: Option<Severity>, text: &str) -> HintLine {
        HintLine {
            severity,
            text: text.to_owned(),
        }
    }

    /// A segment.
    fn seg(text: &str, kind: Option<TokenKind>, mark: Option<Severity>) -> Segment {
        Segment {
            text: text.to_owned(),
            kind,
            mark,
        }
    }

    /// The printed query an Enter commits; panics on an edit.
    fn committed(outcome: Enter) -> Option<String> {
        match outcome {
            Enter::Commit(expr) => expr.map(|e| print(&e)),
            Enter::Edit(text, _) => panic!("expected a commit, got an edit of {text:?}"),
        }
    }

    /// A suggestion inserting `insert`.
    fn sug(insert: &str, kind: SuggestionKind) -> Suggestion {
        Suggestion::new(insert, insert, "", kind, false)
    }

    #[test]
    fn an_empty_palette_offers_fields_and_the_copy() {
        let analysis = at_end("");
        assert_eq!(analysis.ready, Ready::Blank);
        assert_eq!(analysis.hints, vec![line(None, EMPTY_HINT)]);
        assert_eq!(
            analysis.suggestions.first().map(|s| s.label.as_str()),
            Some("description:")
        );
    }

    #[test]
    fn an_error_blocks_and_underlines_its_span() {
        let analysis = at_end("acount:x");
        assert_eq!(analysis.ready, Ready::Blocked);
        assert_eq!(
            analysis.hints,
            vec![line(
                Some(Severity::Error),
                "unknown field 'acount' (did you mean 'account'?)"
            )]
        );
        assert_eq!(
            analysis.segments,
            vec![
                seg("acount", Some(TokenKind::Field), Some(Severity::Error)),
                seg(":", Some(TokenKind::Field), None),
                seg("x", Some(TokenKind::Value), None),
            ]
        );
    }

    #[test]
    fn a_zero_width_parse_error_underlines_the_character_before() {
        let analysis = at_end("account:");
        assert_eq!(analysis.ready, Ready::Blocked);
        assert_eq!(
            analysis.segments,
            vec![
                seg("account", Some(TokenKind::Field), None),
                seg(":", Some(TokenKind::Field), Some(Severity::Error)),
            ]
        );
    }

    #[test]
    fn repeated_warnings_show_once_and_the_term_is_described() {
        let analysis = at_end("@km:1 or @km:2");
        assert!(matches!(analysis.ready, Ready::Query(_)));
        assert_eq!(
            analysis.hints,
            vec![
                line(
                    Some(Severity::Warning),
                    "2 values of '@km' are not numbers and were not compared"
                ),
                line(None, "@km exactly 2."),
            ]
        );
    }

    #[test]
    fn a_leading_angle_bracket_is_reserved() {
        let analysis = at_end(">go");
        assert_eq!(analysis.ready, Ready::Blocked);
        assert_eq!(
            analysis.hints,
            vec![line(Some(Severity::Error), COMMAND_HINT)]
        );
        assert_eq!(analysis.suggestions, Vec::<Suggestion>::new());
    }

    #[test]
    fn a_valid_query_describes_the_term_at_the_cursor() {
        assert_eq!(
            at_end("amount:>100").hints,
            vec![line(None, "Leg amount over 100, in any currency.")]
        );
    }

    #[rstest]
    #[case("status:unrec", Some(0), Some("status:unreconciled"))]
    #[case("account:groc", Some(0), Some("account:Groceries"))]
    #[case("account:food", Some(1), Some("account:Income:Food"))]
    #[case("amount:100", Some(0), Some("amount:100"))]
    #[case("des", Some(0), Some("des"))]
    #[case("", Some(0), None)]
    fn enter_commits(
        #[case] text: &str,
        #[case] selected: Option<usize>,
        #[case] expected: Option<&str>,
    ) {
        let outcome = enter(text, text.len(), selected, &catalog(), TODAY);
        assert_eq!(committed(outcome).as_deref(), expected);
    }

    #[rstest]
    #[case("acount:x", Some(0))]
    #[case("status:", None)]
    #[case(">go", None)]
    #[case("@km:abc", Some(0))]
    fn enter_keeps_editing_a_blocked_query(#[case] text: &str, #[case] selected: Option<usize>) {
        assert_eq!(
            enter(text, text.len(), selected, &catalog(), TODAY),
            Enter::Edit(text.to_owned(), text.len())
        );
    }

    #[rstest]
    #[case(
        "tag:me ac",
        Span::new(7, 9),
        sug("account:", SuggestionKind::Field),
        "tag:me account:",
        15
    )]
    #[case(
        "\"x\"",
        Span::new(3, 3),
        sug("or ", SuggestionKind::Keyword),
        "\"x\" or ",
        7
    )]
    #[case("(ta", Span::new(1, 3), sug("tag:", SuggestionKind::Field), "(tag:", 5)]
    #[case("-ta", Span::new(1, 3), sug("tag:", SuggestionKind::Field), "-tag:", 5)]
    #[case("(a", Span::new(2, 2), sug(")", SuggestionKind::Keyword), "(a)", 3)]
    #[case(
        "account:foo x",
        Span::new(8, 11),
        sug("Food", SuggestionKind::Account),
        "account:Food x",
        12
    )]
    #[case(
        "café ac",
        Span::new(6, 8),
        sug("account:", SuggestionKind::Field),
        "café account:",
        14
    )]
    fn accept_inserts_at_the_span(
        #[case] text: &str,
        #[case] replace: Span,
        #[case] suggestion: Suggestion,
        #[case] expected: &str,
        #[case] cursor: usize,
    ) {
        assert_eq!(
            accept(text, replace, &suggestion),
            (expected.to_owned(), cursor)
        );
    }

    #[rstest]
    #[case("abc", 2, 2)]
    #[case("café x", 6, 5)]
    #[case("😀x", 4, 2)]
    #[case("😀x", 5, 3)]
    #[case("café account:", 14, 13)]
    fn offsets_convert_between_bytes_and_units(
        #[case] text: &str,
        #[case] byte: usize,
        #[case] units: usize,
    ) {
        assert_eq!(byte_to_utf16(text, byte), units);
        assert_eq!(utf16_to_byte(text, units), byte);
    }

    #[test]
    fn a_unit_past_the_end_clamps() {
        assert_eq!(utf16_to_byte("ab", 9), 2);
    }

    #[test]
    fn segments_split_at_every_boundary() {
        let text = "tag:me x";
        let got = segments(
            text,
            &tokens(text),
            &[
                (Span::new(4, 8), Severity::Warning),
                (Span::new(5, 6), Severity::Error),
            ],
        );
        assert_eq!(
            got,
            vec![
                seg("tag:", Some(TokenKind::Field), None),
                seg("m", Some(TokenKind::Value), Some(Severity::Warning)),
                seg("e", Some(TokenKind::Value), Some(Severity::Error)),
                seg(" ", None, Some(Severity::Warning)),
                seg("x", Some(TokenKind::Text), Some(Severity::Warning)),
            ]
        );
    }

    #[test]
    fn names_cover_every_kind_and_severity() {
        let kinds: Vec<&str> = [
            TokenKind::Field,
            TokenKind::Key,
            TokenKind::Operator,
            TokenKind::Value,
            TokenKind::Text,
            TokenKind::Keyword,
            TokenKind::Paren,
        ]
        .into_iter()
        .map(kind_name)
        .collect();
        assert_eq!(
            kinds,
            vec![
                "field", "key", "operator", "value", "text", "keyword", "paren"
            ]
        );
        let severities: Vec<&str> = [Severity::Error, Severity::Warning, Severity::Hint]
            .into_iter()
            .map(severity_name)
            .collect();
        assert_eq!(severities, vec!["error", "warning", "hint"]);
        let ranks: Vec<u8> = [Severity::Hint, Severity::Error, Severity::Warning]
            .into_iter()
            .map(severity_rank)
            .collect();
        assert_eq!(ranks, vec![2, 0, 1]);
    }
}
