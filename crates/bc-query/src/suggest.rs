//! What the palette offers at the cursor.

use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::ast::Field;
use crate::ast::Op;
use crate::catalog::Catalog;
use crate::catalog::MetaKey;
use crate::catalog::MetaType;
use crate::catalog::PathEntry;
use crate::catalog::shortest_endings;
use crate::complete::CompletionContext;
use crate::complete::CompletionKind;
use crate::currency::MarkerSource as _;
use crate::printer::value_text;
use crate::span::Span;

/// The most suggestions offered at once.
const LIMIT: usize = 50;

/// The built-in fields with their one-line descriptions, in offer order.
const FIELDS: [(&str, &str); 8] = [
    ("description", "text in the description"),
    ("account", "legs on an account and its subaccounts"),
    ("tag", "a tag on the leg or its transaction"),
    ("status", "reconciliation or balance"),
    ("date", "a day, month or year, or a range"),
    ("amount", "a leg's amount, either sign"),
    ("commodity", "a leg's commodity"),
    ("any", "some leg of the transaction matches"),
];

/// The `status:` words with the status they test.
const STATUS_WORDS: [(&str, &str); 5] = [
    ("unreconciled", "reconciliation"),
    ("flagged", "reconciliation"),
    ("reconciled", "reconciliation"),
    ("balanced", "balance"),
    ("unbalanced", "balance"),
];

/// The two boolean values.
const BOOLEANS: [(&str, &str); 2] = [("true", ""), ("false", "")];

/// Operators on a date or timestamp, with their meaning.
const DATE_OPERATORS: [(&str, &str); 5] = [
    ("=", "within the period"),
    (">", "after the period"),
    (">=", "from the period's start"),
    ("<", "before the period"),
    ("<=", "through the period's end"),
];

/// Operators on a number or amount, with their meaning.
const NUMBER_OPERATORS: [(&str, &str); 5] = [
    ("=", "exactly"),
    (">", "over"),
    (">=", "at least"),
    ("<", "under"),
    ("<=", "at most"),
];

/// What a suggestion completes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[expect(
    clippy::exhaustive_enums,
    reason = "the palette renders every kind; a new kind needs a new rendering"
)]
pub enum SuggestionKind {
    /// A built-in field name.
    Field,
    /// A metadata key.
    Key,
    /// An operator or `*`.
    Operator,
    /// `or`, `(` or `)`.
    Keyword,
    /// An account path.
    Account,
    /// A tag path.
    Tag,
    /// Any other value.
    Value,
}

/// One dropdown row.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Suggestion {
    /// The text that replaces the context's span.
    pub insert: String,
    /// What the row shows.
    pub label: String,
    /// A short description beside the label; may be empty.
    pub detail: String,
    /// What it completes.
    pub kind: SuggestionKind,
    /// Whether Enter may take it before committing. The palette takes an
    /// account or tag path only when it is the sole path offered and the
    /// typed path does not resolve on its own.
    pub accept_on_enter: bool,
}

impl Suggestion {
    /// Creates a suggestion.
    ///
    /// # Arguments
    ///
    /// * `insert` - The text that replaces the context's span.
    /// * `label` - What the row shows.
    /// * `detail` - A short description; may be empty.
    /// * `kind` - What it completes.
    /// * `accept_on_enter` - Whether Enter may take it before committing.
    #[must_use]
    pub fn new(
        insert: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
        kind: SuggestionKind,
        accept_on_enter: bool,
    ) -> Self {
        Self {
            insert: insert.into(),
            label: label.into(),
            detail: detail.into(),
            kind,
            accept_on_enter,
        }
    }
}

/// One stored value of a text key, and how many entries hold it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct StoredValue {
    /// The value's text.
    pub value: String,
    /// How many entries hold it.
    pub count: u64,
}

impl StoredValue {
    /// Creates a stored value.
    ///
    /// # Arguments
    ///
    /// * `value` - The value's text.
    /// * `count` - How many entries hold it.
    #[must_use]
    pub fn new(value: impl Into<String>, count: u64) -> Self {
        Self {
            value: value.into(),
            count,
        }
    }
}

#[cfg(feature = "ipc")]
impl From<bc_ipc::MetaValueCount> for StoredValue {
    /// Converts the server's value count.
    fn from(dto: bc_ipc::MetaValueCount) -> Self {
        Self::new(dto.value, dto.count)
    }
}

/// A text key's value at the cursor: the values to fetch, and the text a
/// chosen value replaces.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextQuery {
    /// The key, as the catalog registers it.
    pub key: String,
    /// Everything typed after the operator, unquoted.
    pub needle: String,
    /// The typed value, its opening quote included; empty just after the
    /// colon.
    pub typed: Span,
}

/// The text key's value the cursor sits in, if any.
///
/// `None` anywhere else: built-in fields, keys of other types, unknown keys,
/// an operator a text key rejects, and `*`.
///
/// # Arguments
///
/// * `context` - What the cursor sits in, from [`crate::parse_partial`].
/// * `catalog` - The ledger facts.
#[must_use]
pub fn text_query<C>(context: &CompletionContext, catalog: &C) -> Option<TextQuery>
where
    C: Catalog,
{
    let (field, op, needle, typed) = match &context.kind {
        CompletionKind::Operator { field } => (field, Op::Match, "", context.replace),
        CompletionKind::Value {
            field,
            op,
            whole,
            whole_span,
            ..
        } => (field, *op, whole.as_str(), *whole_span),
        CompletionKind::Start
        | CompletionKind::AfterTerm
        | CompletionKind::Text
        | CompletionKind::Field { .. }
        | CompletionKind::Key { .. } => return None,
    };
    if !field.meta || !matches!(op, Op::Match | Op::Equal) || needle == "*" {
        return None;
    }
    let key = catalog
        .meta_keys()
        .iter()
        .find(|k| k.key.eq_ignore_ascii_case(&field.name) && matches!(k.ty, MetaType::Text))?;
    Some(TextQuery {
        key: key.key.clone(),
        needle: needle.to_owned(),
        typed,
    })
}

/// The suggestions from `values` whose text contains `needle`, ignoring
/// ASCII case: values starting with it first, then by count descending, then
/// by value; at most fifty. Each inserts its value, quoted when needed.
///
/// # Arguments
///
/// * `values` - The stored values to choose from.
/// * `needle` - The text typed so far.
#[must_use]
pub fn text_values(values: &[StoredValue], needle: &str) -> Vec<Suggestion> {
    let folded = needle.to_ascii_lowercase();
    let mut found: Vec<(bool, &StoredValue)> = values
        .iter()
        .filter_map(|stored| {
            let lower = stored.value.to_ascii_lowercase();
            lower
                .contains(folded.as_str())
                .then(|| (lower.starts_with(folded.as_str()), stored))
        })
        .collect();
    found.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.count.cmp(&a.1.count))
            .then_with(|| a.1.value.cmp(&b.1.value))
    });
    found
        .into_iter()
        .take(LIMIT)
        .map(|(_, stored)| {
            Suggestion::new(
                value_text(&stored.value),
                stored.value.clone(),
                uses(stored.count),
                SuggestionKind::Value,
                false,
            )
        })
        .collect()
}

/// "1 use" or "N uses".
fn uses(count: u64) -> String {
    if count == 1 {
        "1 use".to_owned()
    } else {
        format!("{count} uses")
    }
}

/// What a value position holds.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    /// An account path.
    Account,
    /// A tag path.
    Tag,
    /// A status word.
    Status,
    /// A date or timestamp period.
    Date,
    /// An amount with an optional marker.
    Amount,
    /// A commodity code.
    Commodity,
    /// `true` or `false`.
    Boolean,
    /// Free text or a number: nothing to offer.
    Other,
}

/// The suggestions for `context`, best first, at most fifty.
///
/// # Arguments
///
/// * `context` - What the cursor sits in, from [`crate::parse_partial`].
/// * `catalog` - The ledger facts.
/// * `today` - The date that period suggestions start from.
#[must_use]
pub fn suggest<C>(context: &CompletionContext, catalog: &C, today: Date) -> Vec<Suggestion>
where
    C: Catalog,
{
    let mut out = match &context.kind {
        CompletionKind::Start => {
            let mut list = fields("");
            list.extend(keys(catalog, ""));
            list.push(Suggestion::new(
                "(",
                "(",
                "group",
                SuggestionKind::Keyword,
                false,
            ));
            list
        }
        CompletionKind::AfterTerm => {
            let mut list = vec![Suggestion::new(
                "or ",
                "or",
                "either side",
                SuggestionKind::Keyword,
                false,
            )];
            if context.open_groups > 0 {
                list.push(Suggestion::new(
                    ")",
                    ")",
                    "close the group",
                    SuggestionKind::Keyword,
                    false,
                ));
            }
            list.extend(fields(""));
            list.extend(keys(catalog, ""));
            list
        }
        CompletionKind::Text => Vec::new(),
        CompletionKind::Field { partial } => {
            let mut list = fields(partial);
            list.extend(keys(catalog, partial));
            list
        }
        CompletionKind::Key { partial } => keys(catalog, partial),
        CompletionKind::Operator { field } => {
            let mut list = operators(field, catalog);
            list.extend(values(field, "", catalog, today));
            list
        }
        CompletionKind::Value { field, partial, .. } => values(field, partial, catalog, today),
    };
    out.truncate(LIMIT);
    out
}

/// The built-in fields whose name starts with `partial`.
fn fields(partial: &str) -> Vec<Suggestion> {
    let needle = partial.to_ascii_lowercase();
    FIELDS
        .into_iter()
        .filter(|(name, _)| name.starts_with(needle.as_str()))
        .map(|(name, detail)| {
            let insert = if name == "any" {
                "any:(".to_owned()
            } else {
                format!("{name}:")
            };
            Suggestion::new(
                insert,
                format!("{name}:"),
                detail,
                SuggestionKind::Field,
                false,
            )
        })
        .collect()
}

/// The registered keys starting with `partial`, by key.
fn keys<C>(catalog: &C, partial: &str) -> Vec<Suggestion>
where
    C: Catalog,
{
    let needle = partial.to_ascii_lowercase();
    let mut found: Vec<&MetaKey> = catalog
        .meta_keys()
        .iter()
        .filter(|k| k.key.starts_with(needle.as_str()))
        .collect();
    found.sort_by(|a, b| a.key.cmp(&b.key));
    found
        .into_iter()
        .map(|k| {
            Suggestion::new(
                format!("@{}:", k.key),
                format!("@{}", k.key),
                k.ty.name(),
                SuggestionKind::Key,
                false,
            )
        })
        .collect()
}

/// The registered type of the key `field` names, if it is a known key.
fn key_type<C>(field: &Field, catalog: &C) -> Option<MetaType>
where
    C: Catalog,
{
    catalog
        .meta_keys()
        .iter()
        .find(|k| k.key.eq_ignore_ascii_case(&field.name))
        .map(|k| k.ty)
}

/// The operators `field` accepts, with their meaning.
fn operators<C>(field: &Field, catalog: &C) -> Vec<Suggestion>
where
    C: Catalog,
{
    let mut table: Vec<(&str, &str)> = Vec::new();
    if field.meta {
        let ty = key_type(field, catalog);
        match ty {
            Some(MetaType::Text) => table.push(("=", "the whole value")),
            Some(MetaType::Number | MetaType::Amount) => table.extend(NUMBER_OPERATORS),
            Some(MetaType::Date | MetaType::Timestamp) => table.extend(DATE_OPERATORS),
            Some(MetaType::Account) => table.push(("=", "this account only")),
            Some(MetaType::Boolean) => table.push(("=", "exactly")),
            None => {}
        }
        if ty.is_some() {
            table.push(("*", "has any value"));
        }
    } else {
        match field.name.as_str() {
            "description" => table.push(("=", "the whole description")),
            "account" => table.push(("=", "this account only")),
            "tag" => table.extend([("=", "this tag only"), ("*", "any tag")]),
            "date" => table.extend(DATE_OPERATORS),
            "amount" => table.extend(NUMBER_OPERATORS),
            _ => {}
        }
    }
    table
        .into_iter()
        .map(|(op, detail)| Suggestion::new(op, op, detail, SuggestionKind::Operator, false))
        .collect()
}

/// What `field`'s values are.
fn value_kind<C>(field: &Field, catalog: &C) -> ValueKind
where
    C: Catalog,
{
    if field.meta {
        return match key_type(field, catalog) {
            Some(MetaType::Account) => ValueKind::Account,
            Some(MetaType::Date | MetaType::Timestamp) => ValueKind::Date,
            Some(MetaType::Amount) => ValueKind::Amount,
            Some(MetaType::Boolean) => ValueKind::Boolean,
            Some(MetaType::Text | MetaType::Number) | None => ValueKind::Other,
        };
    }
    match field.name.as_str() {
        "account" => ValueKind::Account,
        "tag" => ValueKind::Tag,
        "status" => ValueKind::Status,
        "date" => ValueKind::Date,
        "amount" => ValueKind::Amount,
        "commodity" => ValueKind::Commodity,
        _ => ValueKind::Other,
    }
}

/// The values for `field` matching `partial`.
fn values<C>(field: &Field, partial: &str, catalog: &C, today: Date) -> Vec<Suggestion>
where
    C: Catalog,
{
    match value_kind(field, catalog) {
        ValueKind::Account => paths(
            catalog.accounts(),
            partial,
            " :: ",
            SuggestionKind::Account,
            |id| catalog.is_archived(id),
        ),
        ValueKind::Tag => paths(catalog.tags(), partial, ":", SuggestionKind::Tag, |_| false),
        ValueKind::Status => words(&STATUS_WORDS, partial),
        ValueKind::Boolean => words(&BOOLEANS, partial),
        ValueKind::Commodity => commodities(catalog, partial),
        ValueKind::Amount => marked_amounts(catalog, partial),
        ValueKind::Date => periods(today, partial),
        ValueKind::Other => Vec::new(),
    }
}

/// Entries whose path contains `partial`, less those `hidden` names. An exact
/// ending comes first, then a leaf starting with it, then the rest, each group
/// by path. Each entry inserts its shortest unique ending.
///
/// Endings are computed over every entry, hidden ones included, because
/// resolution still reads hidden entries.
fn paths<H>(
    entries: &[PathEntry],
    partial: &str,
    separator: &str,
    kind: SuggestionKind,
    hidden: H,
) -> Vec<Suggestion>
where
    H: Fn(&str) -> bool,
{
    let needle = partial.to_ascii_lowercase();
    let suffix = format!(":{needle}");
    let endings = shortest_endings(entries);
    let mut found: Vec<(u8, String, &PathEntry)> = entries
        .iter()
        .filter_map(|entry| {
            if hidden(&entry.id) {
                return None;
            }
            let display = entry.display();
            let full = display.to_ascii_lowercase();
            if !full.contains(needle.as_str()) {
                return None;
            }
            let rank = if full == needle || full.ends_with(suffix.as_str()) {
                0
            } else if entry
                .path
                .last()
                .is_some_and(|leaf| leaf.to_ascii_lowercase().starts_with(needle.as_str()))
            {
                1
            } else {
                2
            };
            Some((rank, display, entry))
        })
        .collect();
    found.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    found
        .into_iter()
        .map(|(_, display, entry)| {
            let ending = endings.get(&entry.id).cloned().unwrap_or(display);
            Suggestion::new(
                value_text(&ending),
                entry.path.join(separator),
                "",
                kind,
                true,
            )
        })
        .collect()
}

/// A word with its detail.
type Word<'a> = (&'a str, &'a str);

/// Words containing `partial`, ignoring case, with an exact match first.
fn words(table: &[(&str, &str)], partial: &str) -> Vec<Suggestion> {
    let needle = partial.to_ascii_lowercase();
    let (exact, rest): (Vec<&Word<'_>>, Vec<&Word<'_>>) = table
        .iter()
        .filter(|(word, _)| word.contains(needle.as_str()))
        .partition(|(word, _)| *word == needle);
    exact
        .into_iter()
        .chain(rest)
        .map(|(word, detail)| Suggestion::new(*word, *word, *detail, SuggestionKind::Value, true))
        .collect()
}

/// Commodity codes starting with `partial`, an exact code first, then by code.
fn commodities<C>(catalog: &C, partial: &str) -> Vec<Suggestion>
where
    C: Catalog,
{
    let needle = partial.to_ascii_uppercase();
    let mut found: Vec<(&str, &str)> = catalog
        .commodities()
        .iter()
        .map(|c| (c.code(), c.symbol().unwrap_or_default()))
        .filter(|(code, _)| code.to_ascii_uppercase().starts_with(needle.as_str()))
        .collect();
    found.sort_by_key(|(code, _)| (!code.eq_ignore_ascii_case(partial), *code));
    found
        .into_iter()
        .map(|(code, symbol)| {
            Suggestion::new(value_text(code), code, symbol, SuggestionKind::Value, true)
        })
        .collect()
}

/// When `partial` is a number, that number with each commodity's code after it.
fn marked_amounts<C>(catalog: &C, partial: &str) -> Vec<Suggestion>
where
    C: Catalog,
{
    let number = partial.trim();
    if number.parse::<Decimal>().is_err() {
        return Vec::new();
    }
    let mut codes: Vec<(&str, &str)> = catalog
        .commodities()
        .iter()
        .map(|c| (c.code(), c.symbol().unwrap_or_default()))
        .collect();
    codes.sort_by_key(|(code, _)| *code);
    codes
        .into_iter()
        .map(|(code, symbol)| {
            let text = format!("{number} {code}");
            Suggestion::new(
                value_text(&text),
                text,
                symbol,
                SuggestionKind::Value,
                false,
            )
        })
        .collect()
}

/// This month and this year, when they start with `partial`, an exact one first.
fn periods(today: Date, partial: &str) -> Vec<Suggestion> {
    let month = format!("{:04}-{:02}", today.year(), today.month());
    let year = format!("{:04}", today.year());
    let mut out: Vec<Suggestion> = [(month, "this month"), (year, "this year")]
        .into_iter()
        .filter(|(text, _)| text.starts_with(partial))
        .map(|(text, detail)| {
            Suggestion::new(text.clone(), text, detail, SuggestionKind::Value, false)
        })
        .collect();
    out.sort_by_key(|s| s.insert != partial);
    out
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::Expr;
    use crate::ast::Criterion;
    use crate::catalog::MetaKey;
    use crate::catalog::PathEntry;
    use crate::catalog::Snapshot;
    use crate::complete::parse_partial;
    use crate::currency::Commodity;
    use crate::parser::parse;
    use crate::resolve::resolve;

    /// An invented ledger.
    fn catalog() -> Snapshot {
        Snapshot::new(
            vec![
                PathEntry::new("a1", ["Expenses", "Food"]),
                PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
                PathEntry::new("a3", ["Income", "Food"]),
                PathEntry::new("a4", ["Assets", "Bank"]),
                PathEntry::new("a5", ["Liabilities", "Credit Card"]),
            ],
            vec![
                PathEntry::new("t1", ["me"]),
                PathEntry::new("t2", ["partner"]),
                PathEntry::new("t3", ["institution"]),
                PathEntry::new("t4", ["institution", "bank-a"]),
            ],
            vec![
                Commodity::new("AUD", Some("A$"), &[]),
                Commodity::new("USD", Some("$"), &[]),
                Commodity::new("CAD", Some("C$"), &["$"]),
            ],
            vec![
                MetaKey::new("payee", MetaType::Text, 0),
                MetaKey::new("km", MetaType::Number, 2),
                MetaKey::new("deposit", MetaType::Amount, 0),
                MetaKey::new("reimbursed", MetaType::Boolean, 0),
                MetaKey::new("due", MetaType::Date, 0),
                MetaKey::new("owner", MetaType::Account, 0),
            ],
        )
    }

    /// The suggestions with the cursor at the end of `text`.
    fn at_end(text: &str) -> Vec<Suggestion> {
        suggest(
            &parse_partial(text, text.len()),
            &catalog(),
            date(2026, 10, 4),
        )
    }

    /// The suggestions' labels.
    fn labels(text: &str) -> Vec<String> {
        at_end(text).into_iter().map(|s| s.label).collect()
    }

    /// The suggestions' inserted text.
    fn inserts(text: &str) -> Vec<String> {
        at_end(text).into_iter().map(|s| s.insert).collect()
    }

    #[test]
    fn a_new_term_offers_fields_keys_and_a_group() {
        assert_eq!(
            labels(""),
            vec![
                "description:",
                "account:",
                "tag:",
                "status:",
                "date:",
                "amount:",
                "commodity:",
                "any:",
                "@deposit",
                "@due",
                "@km",
                "@owner",
                "@payee",
                "@reimbursed",
                "("
            ]
        );
        assert_eq!(inserts("an"), vec!["any:("]);
    }

    #[test]
    fn a_partial_word_offers_fields_and_keys_it_starts() {
        assert_eq!(labels("ac"), vec!["account:"]);
        assert_eq!(inserts("pay"), vec!["@payee:"]);
        let keys = at_end("@d");
        assert_eq!(
            keys.iter()
                .map(|s| (s.label.as_str(), s.detail.as_str()))
                .collect::<Vec<_>>(),
            vec![("@deposit", "amount"), ("@due", "date")]
        );
    }

    #[rstest]
    #[case("amount:", &["=", ">", ">=", "<", "<="])]
    #[case("@payee:", &["=", "*"])]
    #[case("@reimbursed:", &["=", "*", "true", "false"])]
    #[case("tag:", &["=", "*", "institution", "institution:bank-a", "me", "partner"])]
    #[case("status:", &["unreconciled", "flagged", "reconciled", "balanced", "unbalanced"])]
    #[case("commodity:", &["AUD", "CAD", "USD"])]
    #[case("date:", &["=", ">", ">=", "<", "<=", "2026-10", "2026"])]
    #[case("description:", &["="])]
    fn after_a_colon_offers_operators_then_values(#[case] text: &str, #[case] expected: &[&str]) {
        assert_eq!(labels(text), expected);
    }

    #[rstest]
    #[case("status:bal", &["balanced", "unbalanced"])]
    #[case("status:BALANCED", &["balanced", "unbalanced"])]
    #[case("status:reconciled", &["reconciled", "unreconciled"])]
    #[case("commodity:a", &["AUD"])]
    #[case("date:2026", &["2026", "2026-10"])]
    #[case("date:>2026-1", &["2026-10"])]
    #[case("amount:100", &["100 AUD", "100 CAD", "100 USD"])]
    #[case("amount:abc", &[])]
    #[case("@km:5", &[])]
    #[case("\"some te", &[])]
    fn values_match_what_was_typed(#[case] text: &str, #[case] expected: &[&str]) {
        assert_eq!(labels(text), expected);
    }

    #[test]
    fn a_leaf_starting_with_the_text_outranks_a_parent_holding_it() {
        assert_eq!(
            labels("account:foo"),
            vec![
                "Expenses :: Food",
                "Income :: Food",
                "Expenses :: Food :: Groceries"
            ]
        );
    }

    #[test]
    fn paths_rank_an_exact_ending_first_and_insert_the_shortest_one() {
        assert_eq!(
            labels("account:food"),
            vec![
                "Expenses :: Food",
                "Income :: Food",
                "Expenses :: Food :: Groceries"
            ]
        );
        assert_eq!(
            inserts("account:food"),
            vec!["Expenses:Food", "Income:Food", "Groceries"]
        );
        assert_eq!(
            inserts("account:groc").first().map(String::as_str),
            Some("Groceries")
        );
        assert_eq!(inserts("account:credit"), vec!["\"Credit Card\""]);
        assert_eq!(inserts("@owner:bank"), vec!["Bank"]);
        assert_eq!(labels("tag:bank"), vec!["institution:bank-a"]);
        assert_eq!(inserts("tag:bank"), vec!["bank-a"]);
    }

    #[test]
    fn archived_accounts_resolve_but_are_not_offered() {
        let snapshot = catalog().with_archived(vec!["a3".to_owned()]);
        let got = suggest(
            &parse_partial("account:food", 12),
            &snapshot,
            date(2026, 10, 4),
        );
        assert_eq!(
            got.iter().map(|s| s.insert.as_str()).collect::<Vec<_>>(),
            vec!["Expenses:Food", "Groceries"]
        );
        let resolved = resolve(&parse("account:Income:Food").expect("parses"), &snapshot);
        assert!(!resolved.has_errors(), "{:?}", resolved.diagnostics);
    }

    #[test]
    fn amounts_insert_the_number_before_its_code() {
        assert_eq!(
            inserts("amount:20.5"),
            vec!["\"20.5 AUD\"", "\"20.5 CAD\"", "\"20.5 USD\""]
        );
    }

    #[test]
    fn after_a_term_offers_or_then_a_close_when_a_group_is_open() {
        assert_eq!(
            labels("(tag:me ").get(..2),
            Some(&["or".to_owned(), ")".to_owned()][..])
        );
        assert_eq!(
            labels("tag:me ").get(..2),
            Some(&["or".to_owned(), "description:".to_owned()][..])
        );
    }

    #[rstest]
    #[case("account:food", true)]
    #[case("tag:m", true)]
    #[case("status:un", true)]
    #[case("commodity:a", true)]
    #[case("@reimbursed:t", true)]
    #[case("amount:100", false)]
    #[case("date:2026", false)]
    #[case("", false)]
    #[case("ac", false)]
    fn enter_takes_only_names_and_words(#[case] text: &str, #[case] accepts: bool) {
        assert_eq!(
            at_end(text).first().map(|s| s.accept_on_enter),
            Some(accepts)
        );
    }

    #[test]
    fn at_most_fifty_are_offered() {
        let many = Snapshot::new(
            (0_u32..60_u32)
                .map(|n| {
                    PathEntry::new(format!("x{n}"), ["Assets".to_owned(), format!("Box{n:02}")])
                })
                .collect(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        );
        let got = suggest(&parse_partial("account:box", 11), &many, date(2026, 10, 4));
        assert_eq!(got.len(), 50);
    }

    #[rstest]
    #[case("@payee:", Some(("payee", "", 7, 7)))]
    #[case("@payee:ca", Some(("payee", "ca", 7, 9)))]
    #[case("@PAYEE:a..b", Some(("payee", "a..b", 7, 11)))]
    #[case("@payee:\"Example C", Some(("payee", "Example C", 7, 17)))]
    #[case("@payee:=ex", Some(("payee", "ex", 8, 10)))]
    #[case("@payee:*", None)]
    #[case("@payee:>x", None)]
    #[case("@km:5", None)]
    #[case("@nokey:x", None)]
    #[case("description:ca", None)]
    #[case("account:x", None)]
    #[case("@pay", None)]
    #[case("cafe", None)]
    fn text_query_finds_a_text_keys_value(
        #[case] text: &str,
        #[case] expected: Option<(&str, &str, usize, usize)>,
    ) {
        let got = text_query(&parse_partial(text, text.len()), &catalog());
        assert_eq!(
            got.as_ref().map(|q| (
                q.key.as_str(),
                q.needle.as_str(),
                q.typed.start,
                q.typed.end
            )),
            expected
        );
    }

    /// Invented stored payees.
    fn stored() -> Vec<StoredValue> {
        vec![
            StoredValue::new("Example Cafe", 3),
            StoredValue::new("Cafe Uno", 1),
            StoredValue::new("Corner Cafe", 1),
            StoredValue::new("example cafe", 1),
            StoredValue::new("Bakery", 1),
            StoredValue::new("Archive Co", 150),
        ]
    }

    #[test]
    fn text_values_rank_prefix_matches_then_count_then_value() {
        let got = text_values(&stored(), "CAFE");
        assert_eq!(
            got,
            vec![
                Suggestion::new(
                    "\"Cafe Uno\"",
                    "Cafe Uno",
                    "1 use",
                    SuggestionKind::Value,
                    false
                ),
                Suggestion::new(
                    "\"Example Cafe\"",
                    "Example Cafe",
                    "3 uses",
                    SuggestionKind::Value,
                    false
                ),
                Suggestion::new(
                    "\"Corner Cafe\"",
                    "Corner Cafe",
                    "1 use",
                    SuggestionKind::Value,
                    false
                ),
                Suggestion::new(
                    "\"example cafe\"",
                    "example cafe",
                    "1 use",
                    SuggestionKind::Value,
                    false
                ),
            ]
        );
    }

    #[test]
    fn an_empty_needle_offers_the_most_used_first() {
        let labels: Vec<String> = text_values(&stored(), "")
            .into_iter()
            .map(|s| s.label)
            .collect();
        assert_eq!(
            labels,
            vec![
                "Archive Co",
                "Example Cafe",
                "Bakery",
                "Cafe Uno",
                "Corner Cafe",
                "example cafe"
            ]
        );
    }

    #[test]
    fn text_values_offer_at_most_fifty() {
        let many: Vec<StoredValue> = (0_usize..60_usize)
            .map(|n| StoredValue::new(format!("v{n:02}"), 1))
            .collect();
        assert_eq!(text_values(&many, "v").len(), 50);
    }

    #[test]
    fn case_folding_is_ascii_only() {
        let values = vec![StoredValue::new("Zoë Studio", 1)];
        assert_eq!(text_values(&values, "zoë").len(), 1);
        assert_eq!(text_values(&values, "ZOË").len(), 0);
    }

    #[rstest]
    #[case("Coles")]
    #[case("Say \"hi\"")]
    #[case("back\\slash")]
    #[case("a..b")]
    #[case("(paren)")]
    #[case(">=5")]
    #[case("*")]
    #[case("Zoë Studio")]
    fn a_text_value_inserts_text_that_reads_back_as_that_value(#[case] value: &str) {
        let found = text_values(&[StoredValue::new(value, 1)], "");
        let insert = &found.first().expect("offered").insert;
        let text = format!("@payee:{insert}");
        let Ok(Expr::Term(term)) = parse(&text) else {
            panic!("{text} does not parse as one term");
        };
        match &term.criterion {
            Criterion::Compare { value: read, .. } => assert_eq!(read.text, value, "{text}"),
            Criterion::Any(_) | Criterion::Range { .. } | Criterion::Group(_, _) => {
                panic!("{text} reads as unexpected criterion")
            }
        }
    }
}
