//! The palette's behaviour, free of Leptos: what the input shows, what the
//! hint line says, what the dropdown offers, and what Tab and Enter do.

use bc_query::Catalog;
use bc_query::Expr;
use bc_query::Severity;
use bc_query::Span;
use bc_query::complete::CompletionContext;
use bc_query::complete::CompletionKind;
use bc_query::describe::describe;
use bc_query::highlight::Token;
use bc_query::highlight::TokenKind;
use bc_query::highlight::tokens;
use bc_query::parse;
use bc_query::parse_partial;
use bc_query::resolve;
use bc_query::suggest::StoredValue;
use bc_query::suggest::Suggestion;
use bc_query::suggest::SuggestionKind;
use bc_query::suggest::TextQuery;
use bc_query::suggest::suggest;
use bc_query::suggest::text_query;
use bc_query::suggest::text_values;
use jiff::civil::Date;

/// The hint line for an empty palette.
pub const EMPTY_HINT: &str = "Type to search descriptions, or a field: account: tag: @payee: …";

/// The hint line for input starting with `>`, which is reserved for commands.
pub const COMMAND_HINT: &str = "'>' starts a command; there are none yet";

/// The hint line when the catalog failed to load and a term needed it.
pub const CATALOG_FAILED_HINT: &str =
    "Couldn't load accounts, tags, commodities and keys; the server will check this query.";

/// The hint line while the first catalog fetch is in flight and a term needed it.
pub const CATALOG_LOADING_HINT: &str =
    "Still loading accounts, tags, commodities and keys; the server will check this query.";

/// Whether the catalog that terms resolve against is the ledger's own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Load {
    /// A snapshot exists.
    Ready,
    /// No snapshot yet; the first fetch is in flight.
    Loading,
    /// No snapshot; the fetch failed.
    Failed,
}

impl Load {
    /// The state for a store that `has_snapshot` and whose last fetch
    /// `failed`. A failed refetch behind a snapshot is still `Ready`.
    ///
    /// # Arguments
    ///
    /// * `has_snapshot` - Whether a snapshot exists.
    /// * `failed` - Whether the latest fetch failed.
    #[must_use]
    pub const fn from_store(has_snapshot: bool, failed: bool) -> Self {
        match (has_snapshot, failed) {
            (true, _) => Self::Ready,
            (false, false) => Self::Loading,
            (false, true) => Self::Failed,
        }
    }
}

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
    /// The text a chosen suggestion replaces: the token under the cursor,
    /// through to its end.
    pub replace: Span,
    /// The part of `replace` before the cursor, which the suggestions match.
    pub typed: Span,
    /// The text a suggestion other than an operator replaces: `replace`,
    /// except just after a field's colon, where it runs through the value
    /// already there.
    pub value_replace: Span,
    /// What Enter would commit.
    pub ready: Ready,
    /// The text key and needle whose stored values the dropdown wants, when
    /// the cursor sits in a text key's value.
    pub lookup: Option<TextQuery>,
}

impl Analysis {
    /// The text `suggestion` replaces. Just after a colon, an operator
    /// inserts and keeps the value after it, and anything else replaces that
    /// value.
    ///
    /// # Arguments
    ///
    /// * `suggestion` - The suggestion being accepted.
    #[must_use]
    pub fn span_for(&self, suggestion: &Suggestion) -> Span {
        if suggestion.kind == SuggestionKind::Operator {
            self.replace
        } else {
            self.value_replace
        }
    }
}

/// The most suggestions the dropdown shows.
const SUGGESTION_LIMIT: usize = 50;

/// The server's reply to a text-value fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValueReply {
    /// The key fetched.
    pub key: String,
    /// The needle fetched with.
    pub needle: String,
    /// The values, in the server's order.
    pub values: Vec<StoredValue>,
}

/// Whether `reply` answers what the caret asks for now. A reply for an older
/// needle or another key is dropped on arrival.
///
/// # Arguments
///
/// * `lookup` - What the caret asks for, from [`Analysis::lookup`].
/// * `reply` - The reply that arrived.
#[must_use]
pub fn reply_applies(lookup: Option<&TextQuery>, reply: &ValueReply) -> bool {
    lookup.is_some_and(|query| query.key == reply.key && query.needle == reply.needle)
}

/// Analyses `text` with the cursor at byte `cursor`.
///
/// # Arguments
///
/// * `text` - The input.
/// * `cursor` - The cursor's byte offset.
/// * `catalog` - The ledger facts; empty when `load` is not `Ready`.
/// * `load` - Whether `catalog` is the ledger's own. When it is not, errors
///   that exist only because the catalog is empty are neither underlined nor
///   blocking, and a hint says why.
/// * `today` - The date that period suggestions start from.
/// * `reply` - The latest text-value reply, narrowed here to the current
///   needle; `None` before any arrives.
#[must_use]
pub fn analyse<C>(
    text: &str,
    cursor: usize,
    catalog: &C,
    load: Load,
    today: Date,
    reply: Option<&ValueReply>,
) -> Analysis
where
    C: Catalog,
{
    let context = parse_partial(text, cursor);
    let lookup = text_query(&context, catalog);
    let suggestions = if text.trim_start().starts_with('>') {
        Vec::new()
    } else {
        let mut list = suggest(&context, catalog, today);
        if let (Some(query), Some(stored)) = (&lookup, reply)
            && stored.key == query.key
        {
            list.extend(text_values(&stored.values, &query.needle));
            list.truncate(SUGGESTION_LIMIT);
        }
        list
    };
    let (ready, hints, marks) = check(text, cursor, catalog, load);
    let (replace, typed) = match (&lookup, &context.kind) {
        (Some(query), CompletionKind::Value { .. }) => {
            let rest = text.get(query.typed.end..).unwrap_or_default();
            let quoted = text
                .get(query.typed.start..)
                .is_some_and(|from| from.starts_with('"'));
            let len = if quoted {
                quoted_len(rest)
            } else {
                bare_len(rest)
            };
            (
                Span::new(query.typed.start, query.typed.end.saturating_add(len)),
                query.typed,
            )
        }
        _ => (
            Span::new(context.replace.start, token_end(text, &context)),
            context.replace,
        ),
    };
    let value_replace = match context.kind {
        CompletionKind::Operator { .. } => {
            let rest = text.get(replace.end..).unwrap_or_default();
            Span::new(
                replace.start,
                replace
                    .end
                    .saturating_add(existing_value_len(rest, lookup.is_some())),
            )
        }
        CompletionKind::Start
        | CompletionKind::AfterTerm
        | CompletionKind::Text
        | CompletionKind::Field { .. }
        | CompletionKind::Key { .. }
        | CompletionKind::Value { .. } => replace,
    };
    Analysis {
        segments: segments(text, &tokens(text), &marks),
        hints,
        suggestions,
        replace,
        typed,
        value_replace,
        ready,
        lookup,
    }
}

/// Where the token that `context` completes ends, at or after the cursor.
///
/// A field or key runs through its name and the colon after it. A value runs
/// to whitespace, a parenthesis, a quote or a `..`. A quoted value runs
/// through its closing quote. An empty `context.replace` inserts at the
/// cursor and swallows nothing.
fn token_end(text: &str, context: &CompletionContext) -> usize {
    let replace = context.replace;
    let Some(rest) = text.get(replace.end..) else {
        return replace.end;
    };
    if replace.start >= replace.end {
        return replace.end;
    }
    let quoted = text
        .get(replace.start..)
        .is_some_and(|from| from.starts_with('"'));
    let len = match context.kind {
        CompletionKind::Field { .. } | CompletionKind::Key { .. } => name_len(rest),
        CompletionKind::Value { .. } | CompletionKind::Text if quoted => quoted_len(rest),
        CompletionKind::Value { .. } => value_len(rest),
        CompletionKind::Start
        | CompletionKind::AfterTerm
        | CompletionKind::Text
        | CompletionKind::Operator { .. } => 0,
    };
    replace.end.saturating_add(len)
}

/// Byte length of the field or key name at the start of `rest`, with the
/// colon after it.
fn name_len(rest: &str) -> usize {
    let name = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
        .unwrap_or(rest.len());
    let colon = rest.get(name..).is_some_and(|after| after.starts_with(':'));
    if colon { name.saturating_add(1) } else { name }
}

/// Byte length of the bare run at the start of `rest`: up to whitespace, a
/// parenthesis or a quote.
fn bare_len(rest: &str) -> usize {
    rest.find(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '"'))
        .unwrap_or(rest.len())
}

/// Byte length of the bare value at the start of `rest`: up to whitespace, a
/// parenthesis, a quote, or the `..` of a range.
fn value_len(rest: &str) -> usize {
    let run = bare_len(rest);
    let bare = rest.get(..run).unwrap_or_default();
    bare.find("..").unwrap_or(run)
}

/// Byte length of the value already at the start of `rest`, just after a
/// colon: a quoted string through its closing quote, or a bare value, which
/// runs past `..` when `text_value`. Zero when an operator or `*` comes
/// first, since a value there would not replace them.
fn existing_value_len(rest: &str, text_value: bool) -> usize {
    if rest.starts_with(['=', '<', '>', '*']) {
        return 0;
    }
    match rest.strip_prefix('"') {
        Some(inner) => quoted_len(inner).saturating_add(1),
        None if text_value => bare_len(rest),
        None => value_len(rest),
    }
}

/// Byte length of the rest of a quoted string at the start of `rest`, through
/// its closing quote; all of `rest` when the quote never closes.
fn quoted_len(rest: &str) -> usize {
    let mut escaped = false;
    for (at, c) in rest.char_indices() {
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == '"' {
            return at.saturating_add(1);
        }
    }
    rest.len()
}

/// Parses and resolves `text`: what Enter would commit, the hint lines, and
/// the spans to underline.
fn check<C>(
    text: &str,
    cursor: usize,
    catalog: &C,
    load: Load,
) -> (Ready, Vec<HintLine>, Vec<(Span, Severity)>)
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
    let mut resolved = resolve(&expr, catalog);
    let before = resolved.diagnostics.len();
    if load != Load::Ready {
        resolved.diagnostics.retain(|d| !d.from_catalog);
    }
    let unchecked = resolved.diagnostics.len() < before;
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
    if unchecked {
        let why = if load == Load::Failed {
            CATALOG_FAILED_HINT
        } else {
            CATALOG_LOADING_HINT
        };
        hints.push(HintLine::new(Some(Severity::Hint), why));
    }
    if resolved.has_errors() {
        return (Ready::Blocked, hints, marks);
    }
    if load == Load::Ready
        && let Some(words) = describe(&expr, cursor, catalog)
    {
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
                .min_by_key(|severity| severity.rank());
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
/// it. `None` when `replace` is reversed, runs past the end of `text`, or
/// splits a character.
///
/// # Arguments
///
/// * `text` - The input.
/// * `replace` - The text the suggestion replaces.
/// * `suggestion` - The suggestion.
#[must_use]
pub fn accept(text: &str, replace: Span, suggestion: &Suggestion) -> Option<(String, usize)> {
    if replace.start > replace.end {
        return None;
    }
    let before = text.get(..replace.start)?;
    let after = text.get(replace.end..)?;
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
    Some((out, cursor))
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
/// A highlighted suggestion that accepts on Enter is inserted first, except
/// that an account or tag path is inserted only when it is the sole path
/// offered and the path under the cursor does not resolve on its own. An
/// ambiguous ending therefore blocks with the resolver's list of candidates,
/// and a path that already resolves commits as typed. The resulting text
/// commits when it has no errors; otherwise editing continues with that text.
///
/// # Arguments
///
/// * `text` - The input.
/// * `cursor` - The cursor's byte offset.
/// * `selected` - The highlighted suggestion's index, when the dropdown shows.
/// * `catalog` - The ledger facts; empty when `load` is not `Ready`.
/// * `load` - Whether `catalog` is the ledger's own.
/// * `today` - The date that period suggestions start from.
/// * `reply` - The latest text-value reply, as for [`analyse`].
#[must_use]
pub fn enter<C>(
    text: &str,
    cursor: usize,
    selected: Option<usize>,
    catalog: &C,
    load: Load,
    today: Date,
    reply: Option<&ValueReply>,
) -> Enter
where
    C: Catalog,
{
    let analysis = analyse(text, cursor, catalog, load, today, reply);
    let chosen = selected
        .and_then(|index| analysis.suggestions.get(index))
        .filter(|s| s.accept_on_enter)
        .filter(|s| !is_path(s) || takes_path(text, cursor, &analysis, catalog));
    let accepted =
        chosen.and_then(|suggestion| accept(text, analysis.span_for(suggestion), suggestion));
    let (next_text, next_cursor, ready) = match accepted {
        Some((inserted, at)) => {
            let ready = analyse(&inserted, at, catalog, load, today, reply).ready;
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

/// Whether `suggestion` completes an account or tag path.
const fn is_path(suggestion: &Suggestion) -> bool {
    matches!(
        suggestion.kind,
        SuggestionKind::Account | SuggestionKind::Tag
    )
}

/// Whether Enter may insert a path suggestion: the dropdown offers exactly
/// one path, and the path under the cursor does not resolve on its own.
fn takes_path<C>(text: &str, cursor: usize, analysis: &Analysis, catalog: &C) -> bool
where
    C: Catalog,
{
    let offered = analysis.suggestions.iter().filter(|s| is_path(s)).count();
    offered == 1 && !term_resolves(text, cursor, analysis.value_replace, catalog)
}

/// Whether the value in `replace`, read as a term of the field the cursor
/// completes, resolves with no error. The term is resolved alone, so an
/// error elsewhere in `text` does not count against it.
fn term_resolves<C>(text: &str, cursor: usize, replace: Span, catalog: &C) -> bool
where
    C: Catalog,
{
    let field = match parse_partial(text, cursor).kind {
        CompletionKind::Value { field, .. } | CompletionKind::Operator { field } => field,
        CompletionKind::Start
        | CompletionKind::AfterTerm
        | CompletionKind::Text
        | CompletionKind::Field { .. }
        | CompletionKind::Key { .. } => return false,
    };
    let Some(value) = text.get(replace.start..replace.end) else {
        return false;
    };
    if value.is_empty() {
        return false;
    }
    let sigil = if field.meta { "@" } else { "" };
    let term = format!("{sigil}{}:{value}", field.name);
    parse(&term).is_ok_and(|expr| !resolve(&expr, catalog).has_errors())
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
    use bc_query::suggest::StoredValue;
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
                PathEntry::new("a4", ["Expenses", "Crème"]),
                PathEntry::new("a5", ["Liabilities", "Visa"]),
                PathEntry::new("a6", ["Liabilities", "Visa2"]),
            ],
            vec![
                PathEntry::new("t1", ["me"]),
                PathEntry::new("t2", ["trip", "flights"]),
                PathEntry::new("t3", ["work", "flights"]),
            ],
            vec![Commodity::new("AUD", Some("A$"), &[])],
            vec![MetaKey::new("km", MetaType::Number, 2)],
        )
        .with_archived(vec!["a5".to_owned()])
    }

    /// The analysis with the cursor at the end of `text`.
    fn at_end(text: &str) -> Analysis {
        analyse(text, text.len(), &catalog(), Load::Ready, TODAY, None)
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
        let analysis = at_end("@kn:1");
        assert_eq!(analysis.ready, Ready::Blocked);
        assert_eq!(
            analysis.hints,
            vec![line(
                Some(Severity::Error),
                "unknown key '@kn' (did you mean '@km'?)"
            )]
        );
        assert_eq!(
            analysis.segments,
            vec![
                seg("@kn", Some(TokenKind::Key), Some(Severity::Error)),
                seg(":", Some(TokenKind::Key), None),
                seg("1", Some(TokenKind::Value), None),
            ]
        );
    }

    #[test]
    fn an_unknown_field_warns_and_still_commits() {
        let text = "acount:x"; // spellchecker:disable-line
        let analysis = at_end(text);
        assert!(
            matches!(analysis.ready, Ready::Query(_)),
            "{:?}",
            analysis.ready
        );
        assert_eq!(
            analysis.hints,
            vec![line(
                Some(Severity::Warning),
                "unknown field 'acount' is ignored (did you mean 'account'?)" // spellchecker:disable-line
            )]
        );
        assert_eq!(
            analysis.segments.first().and_then(|s| s.mark),
            Some(Severity::Warning)
        );
        let outcome = enter(text, text.len(), None, &catalog(), Load::Ready, TODAY, None);
        assert_eq!(committed(outcome).as_deref(), Some(text));
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
            vec![line(
                None,
                "Leg amount over 100, either sign, in any currency."
            )]
        );
    }

    #[rstest]
    #[case("status:unrec", Some(0), Some("status:unreconciled"))]
    #[case("account:groc", Some(0), Some("account:Groceries"))]
    #[case("account:Expenses:Food", Some(1), Some("account:Expenses:Food"))]
    #[case("account:visa", Some(0), Some("account:visa"))]
    #[case("tag:trip:fl", Some(0), Some("tag:trip:flights"))]
    #[case("amount:100", Some(0), Some("amount:100"))]
    #[case("des", Some(0), Some("des"))]
    #[case("", Some(0), None)]
    fn enter_commits(
        #[case] text: &str,
        #[case] selected: Option<usize>,
        #[case] expected: Option<&str>,
    ) {
        let outcome = enter(
            text,
            text.len(),
            selected,
            &catalog(),
            Load::Ready,
            TODAY,
            None,
        );
        assert_eq!(committed(outcome).as_deref(), expected);
    }

    #[rstest]
    #[case("@kn:1", Some(0))]
    #[case("status:", None)]
    #[case(">go", None)]
    #[case("@km:abc", Some(0))]
    #[case("account:food", Some(0))]
    #[case("account:food", Some(1))]
    #[case("tag:fl", Some(0))]
    fn enter_keeps_editing_a_blocked_query(#[case] text: &str, #[case] selected: Option<usize>) {
        assert_eq!(
            enter(
                text,
                text.len(),
                selected,
                &catalog(),
                Load::Ready,
                TODAY,
                None
            ),
            Enter::Edit(text.to_owned(), text.len())
        );
    }

    /// The analysis of `text` against an empty catalog in state `load`.
    fn unloaded(text: &str, load: Load) -> Analysis {
        analyse(text, text.len(), &Snapshot::default(), load, TODAY, None)
    }

    #[rstest]
    #[case::account("account:Anything")]
    #[case::tag("tag:anything")]
    #[case::commodity("commodity:AUD")]
    #[case::marker("amount:>=5AUD")]
    #[case::meta_key("@km:5")]
    fn a_missing_catalog_commits_what_it_cannot_check(
        #[case] text: &str,
        #[values(Load::Loading, Load::Failed)] load: Load,
    ) {
        let outcome = enter(
            text,
            text.len(),
            None,
            &Snapshot::default(),
            load,
            TODAY,
            None,
        );
        assert_eq!(committed(outcome).as_deref(), Some(text));
        let analysis = unloaded(text, load);
        assert!(analysis.segments.iter().all(|s| s.mark.is_none()));
    }

    #[rstest]
    #[case::failed(Load::Failed, CATALOG_FAILED_HINT)]
    #[case::loading(Load::Loading, CATALOG_LOADING_HINT)]
    fn a_missing_catalog_says_why(#[case] load: Load, #[case] hint: &str) {
        assert_eq!(
            unloaded("account:Anything", load).hints,
            vec![line(Some(Severity::Hint), hint)]
        );
    }

    #[rstest]
    #[case::wrong_operator("amount:*")]
    #[case::bad_number("amount:abc")]
    #[case::wrong_form("account:>5")]
    #[case::command(">go")]
    #[case::beside_a_catalog_term("account:Anything status:")]
    fn a_missing_catalog_still_blocks_what_it_cannot_excuse(#[case] text: &str) {
        let outcome = enter(
            text,
            text.len(),
            None,
            &Snapshot::default(),
            Load::Failed,
            TODAY,
            None,
        );
        assert_eq!(outcome, Enter::Edit(text.to_owned(), text.len()));
    }

    #[test]
    fn a_missing_catalog_stays_quiet_when_no_term_needs_it() {
        let analysis = unloaded("groceries", Load::Failed);
        assert!(analysis.hints.iter().all(|h| h.text != CATALOG_FAILED_HINT));
        assert!(matches!(analysis.ready, Ready::Query(_)));
    }

    #[rstest]
    #[case(true, false, Load::Ready)]
    #[case(true, true, Load::Ready)]
    #[case(false, false, Load::Loading)]
    #[case(false, true, Load::Failed)]
    fn load_state_from_the_store(
        #[case] has_snapshot: bool,
        #[case] failed: bool,
        #[case] expected: Load,
    ) {
        assert_eq!(Load::from_store(has_snapshot, failed), expected);
    }

    /// `marked` without its `|`, and the byte offset the `|` marked.
    fn caret(marked: &str) -> (String, usize) {
        let at = marked.find('|').expect("a | marks the caret");
        (marked.replacen('|', "", 1), at)
    }

    /// Tab at the `|` in `marked`: the first suggestion, inserted.
    fn tab(marked: &str) -> (String, usize) {
        let (text, at) = caret(marked);
        let analysis = analyse(&text, at, &catalog(), Load::Ready, TODAY, None);
        let first = analysis
            .suggestions
            .first()
            .expect("a suggestion at the caret");
        accept(&text, analysis.span_for(first), first).expect("the span sits on char boundaries")
    }

    #[rstest]
    #[case("status:unreconci|led", "status:unreconciled|")]
    #[case("status:unreconci|led x", "status:unreconciled| x")]
    #[case("account:Groc|eries x", "account:Groceries| x")]
    #[case("account:Crè|me x", "account:Crème| x")]
    #[case("account:\"Groc|eries\" x", "account:Groceries| x")]
    #[case("ac|ount:x", "account:|x")]
    #[case("café ac|ount:x", "café account:|x")]
    #[case("@k|m:2", "@km:|2")]
    #[case("date:2026-1|0..2027", "date:2026-10|..2027")]
    #[case("amount:1|0..20", "amount:\"1 AUD\"|..20")]
    #[case("(account:Groc|eries)", "(account:Groceries|)")]
    #[case("(account:Groc|eries or tag:me)", "(account:Groceries| or tag:me)")]
    #[case("account:\"Gro|c\\\"x\" y", "account:Groceries| y")]
    #[case("account:\"Gro|c", "account:Groceries|")]
    fn tab_replaces_the_whole_token_under_the_caret(#[case] marked: &str, #[case] expected: &str) {
        assert_eq!(tab(marked), caret(expected));
    }

    /// Tab at the `|` in `marked` with the suggestion inserting `insert`
    /// highlighted.
    fn tab_to(marked: &str, insert: &str) -> (String, usize) {
        let (text, at) = caret(marked);
        let analysis = analyse(&text, at, &catalog(), Load::Ready, TODAY, None);
        let chosen = analysis
            .suggestions
            .iter()
            .find(|s| s.insert == insert)
            .unwrap_or_else(|| panic!("{insert} is not offered at {marked}"));
        accept(&text, analysis.span_for(chosen), chosen).expect("the span sits on char boundaries")
    }

    #[rstest]
    #[case("status:|reconciled", "unreconciled", "status:unreconciled|")]
    #[case("status:|reconciled x", "flagged", "status:flagged| x")]
    #[case("account:|\"Expenses:Food\" x", "Groceries", "account:Groceries| x")]
    #[case("account:|Income:Food..x y", "Groceries", "account:Groceries|..x y")]
    #[case("amount:|100", ">=", "amount:>=|100")]
    #[case("tag:|me", "=", "tag:=|me")]
    #[case("tag:|*", "me", "tag:me|*")]
    fn after_a_colon_a_value_replaces_the_value_and_an_operator_keeps_it(
        #[case] marked: &str,
        #[case] insert: &str,
        #[case] expected: &str,
    ) {
        assert_eq!(tab_to(marked, insert), caret(expected));
    }

    #[rstest]
    #[case("status:unreconci|led", "status:unreconciled")]
    #[case("account:Groc|eries x", "account:Groceries x")]
    #[case("account:Crè|me", "account:Crème")]
    #[case("café account:Groc|eries", "café account:Groceries")]
    #[case("account:Vi|sa", "account:Visa")]
    fn enter_mid_token_commits_the_whole_token(#[case] marked: &str, #[case] expected: &str) {
        let (text, at) = caret(marked);
        let outcome = enter(&text, at, Some(0), &catalog(), Load::Ready, TODAY, None);
        assert_eq!(committed(outcome).as_deref(), Some(expected));
    }

    #[test]
    fn an_ambiguous_ending_lists_its_candidates() {
        assert_eq!(
            at_end("account:food").hints,
            vec![line(
                Some(Severity::Error),
                "'food' is ambiguous: Expenses:Food, Income:Food"
            )]
        );
    }

    #[test]
    fn enter_mid_field_name_keeps_the_rest_of_the_term() {
        let (text, at) = caret("acc|ount:Groceries");
        let outcome = enter(&text, at, Some(0), &catalog(), Load::Ready, TODAY, None);
        assert_eq!(committed(outcome).as_deref(), Some("account:Groceries"));
    }

    #[test]
    fn the_suggestions_still_match_only_the_text_before_the_caret() {
        let (text, at) = caret("account:Gro|xyz");
        let analysis = analyse(&text, at, &catalog(), Load::Ready, TODAY, None);
        assert_eq!(
            analysis.suggestions.first().map(|s| s.insert.as_str()),
            Some("Groceries")
        );
        assert_eq!(analysis.typed, Span::new(8, 11));
        assert_eq!(analysis.replace, Span::new(8, 14));
    }

    #[rstest]
    #[case("café", Span::new(4, 4))]
    #[case("café", Span::new(0, 4))]
    #[case("café", Span::new(3, 9))]
    #[case("café", Span::new(3, 2))]
    fn accept_leaves_a_span_off_char_boundaries_alone(#[case] text: &str, #[case] replace: Span) {
        assert_eq!(
            accept(text, replace, &sug("x", SuggestionKind::Value)),
            None
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
            Some((expected.to_owned(), cursor))
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
            .map(Severity::rank)
            .collect();
        assert_eq!(ranks, vec![2, 0, 1]);
    }

    /// The test ledger plus a text key, `payee`.
    fn with_payee() -> Snapshot {
        Snapshot::new(
            vec![
                PathEntry::new("a1", ["Expenses", "Food"]),
                PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
                PathEntry::new("a3", ["Income", "Food"]),
                PathEntry::new("a4", ["Expenses", "Crème"]),
            ],
            vec![PathEntry::new("t1", ["me"])],
            vec![Commodity::new("AUD", Some("A$"), &[])],
            vec![
                MetaKey::new("km", MetaType::Number, 2),
                MetaKey::new("payee", MetaType::Text, 0),
            ],
        )
    }

    /// A reply for `key` fetched with `needle`, holding invented payees.
    fn reply(key: &str, needle: &str) -> ValueReply {
        ValueReply {
            key: key.to_owned(),
            needle: needle.to_owned(),
            values: vec![
                StoredValue::new("Example Cafe", 3),
                StoredValue::new("Example Fuel Stop", 2),
                StoredValue::new("Corner Cafe", 1),
            ],
        }
    }

    /// The inserts offered at the `|` in `marked`, given `stored`.
    fn offered(marked: &str, stored: Option<&ValueReply>) -> Vec<String> {
        let (text, at) = caret(marked);
        analyse(&text, at, &with_payee(), Load::Ready, TODAY, stored)
            .suggestions
            .into_iter()
            .map(|s| s.insert)
            .collect()
    }

    #[test]
    fn a_text_value_asks_for_its_key_and_needle() {
        let (text, at) = caret("@payee:ca|");
        let lookup = analyse(&text, at, &with_payee(), Load::Ready, TODAY, None).lookup;
        assert_eq!(
            lookup.map(|q| (q.key, q.needle)),
            Some(("payee".to_owned(), "ca".to_owned()))
        );
        assert_eq!(offered("@payee:ca|", None), Vec::<String>::new());
        assert_eq!(
            analyse("@km:5", 5, &with_payee(), Load::Ready, TODAY, None).lookup,
            None
        );
    }

    #[test]
    fn a_reply_is_narrowed_to_the_current_needle() {
        assert_eq!(
            offered("@payee:caf|", Some(&reply("payee", "ca"))),
            vec!["\"Example Cafe\"", "\"Corner Cafe\""]
        );
    }

    #[test]
    fn a_reply_for_another_key_offers_nothing() {
        assert_eq!(
            offered("@payee:caf|", Some(&reply("memo", "caf"))),
            Vec::<String>::new()
        );
    }

    #[test]
    fn just_after_the_colon_operators_come_before_the_most_used_values() {
        assert_eq!(
            offered("@payee:|", Some(&reply("payee", ""))),
            vec![
                "=",
                "*",
                "\"Example Cafe\"",
                "\"Example Fuel Stop\"",
                "\"Corner Cafe\""
            ]
        );
    }

    /// Tab at the `|` in `marked` with `stored`: the first suggestion, inserted.
    fn tab_text(marked: &str, stored: &ValueReply) -> (String, usize) {
        let (text, at) = caret(marked);
        let analysis = analyse(&text, at, &with_payee(), Load::Ready, TODAY, Some(stored));
        let first = analysis.suggestions.first().expect("a suggestion");
        accept(&text, analysis.span_for(first), first).expect("boundaries")
    }

    #[rstest]
    #[case("@payee:corn|er..x y", "@payee:\"Corner Cafe\"| y")]
    #[case("@payee:\"Exam|ple Cafe\" y", "@payee:\"Example Cafe\"| y")]
    #[case("@payee:a..|b y", "@payee:\"a..b Example Cafe\"| y")]
    fn tab_replaces_the_whole_text_value(#[case] marked: &str, #[case] expected: &str) {
        let mut stored = reply("payee", "");
        stored.values.push(StoredValue::new("a..b Example Cafe", 0));
        assert_eq!(tab_text(marked, &stored), caret(expected));
    }

    #[rstest]
    #[case("\"Example Cafe\"", "@payee:\"Example Cafe\"| y")]
    #[case("=", "@payee:=|\"Corner Cafe\" y")]
    fn after_a_text_keys_colon_a_value_replaces_and_an_operator_keeps(
        #[case] insert: &str,
        #[case] expected: &str,
    ) {
        let (text, at) = caret("@payee:|\"Corner Cafe\" y");
        let stored = reply("payee", "");
        let analysis = analyse(&text, at, &with_payee(), Load::Ready, TODAY, Some(&stored));
        let chosen = analysis
            .suggestions
            .iter()
            .find(|s| s.insert == insert)
            .expect("offered");
        assert_eq!(
            accept(&text, analysis.span_for(chosen), chosen).expect("boundaries"),
            caret(expected)
        );
    }

    #[test]
    fn enter_commits_the_typed_text_without_inserting_a_value() {
        let stored = reply("payee", "caf");
        let outcome = enter(
            "@payee:caf",
            10,
            Some(0),
            &with_payee(),
            Load::Ready,
            TODAY,
            Some(&stored),
        );
        assert_eq!(committed(outcome).as_deref(), Some("@payee:caf"));
    }

    #[rstest]
    #[case("payee", "cof", true)]
    #[case("payee", "co", false)]
    #[case("memo", "cof", false)]
    fn only_a_reply_for_the_current_key_and_needle_applies(
        #[case] key: &str,
        #[case] needle: &str,
        #[case] applies: bool,
    ) {
        let (text, at) = caret("@payee:cof|");
        let lookup = analyse(&text, at, &with_payee(), Load::Ready, TODAY, None).lookup;
        assert_eq!(reply_applies(lookup.as_ref(), &reply(key, needle)), applies);
        assert!(!reply_applies(None, &reply("payee", "cof")));
    }
}
