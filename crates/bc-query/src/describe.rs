//! Plain words for the term under the cursor, for the palette's hint line.

use jiff::SignedDuration;
use jiff::Timestamp;

use crate::ast::Criterion;
use crate::ast::Expr;
use crate::catalog::Catalog;
use crate::catalog::PathEntry;
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
use crate::resolve::resolve;

/// What a sentence starts with when an odd number of `-` or `not` applies to
/// its term alone.
const NEGATED: &str = "Not: ";

/// What a sentence starts with when a `-` or `not` covers a group of terms
/// that holds it, so the negation does not apply to the term alone.
const IN_NEGATED_GROUP: &str = "In a negated group: ";

/// One sentence describing the term or word under `cursor`. A negated term
/// reads as its positive sentence after "Not: "; a term inside a negated
/// group of several terms reads after "In a negated group: ".
///
/// # Arguments
///
/// * `expr` - The parsed query.
/// * `cursor` - The cursor's byte offset into the text `expr` was parsed from.
/// * `catalog` - The ledger facts.
///
/// # Returns
///
/// `None` when the cursor is on no term or word, or the term does not resolve.
#[must_use]
pub fn describe<C>(expr: &Expr, cursor: usize, catalog: &C) -> Option<String>
where
    C: Catalog,
{
    let found = leaf_at(expr, cursor)?;
    let leaf = found.expr;
    let resolved = resolve(leaf, catalog).expr?;
    let sentence = match resolved {
        ResolvedExpr::Pred(pred) => pred_words(&pred, catalog),
        ResolvedExpr::Any(_) => "Transactions with a leg that matches the group".to_owned(),
        ResolvedExpr::Or(_) | ResolvedExpr::And(_) | ResolvedExpr::Not(_) => return None,
    };
    let group = if found.in_negated_group {
        IN_NEGATED_GROUP
    } else {
        ""
    };
    let negated = if found.negated { NEGATED } else { "" };
    Some(format!("{group}{negated}{sentence}."))
}

/// The term or word under the cursor, and the negations around it.
struct Found<'a> {
    /// The term or word.
    expr: &'a Expr,
    /// Whether an odd number of negations applies to the leaf alone, with no
    /// group of several terms between them and it.
    negated: bool,
    /// Whether a negation covers a group of several terms holding the leaf.
    in_negated_group: bool,
    /// Whether a group of several terms lies between the leaf and the level
    /// being returned to.
    grouped: bool,
}

impl<'a> Found<'a> {
    /// A leaf with nothing around it yet.
    const fn bare(expr: &'a Expr) -> Self {
        Self {
            expr,
            negated: false,
            in_negated_group: false,
            grouped: false,
        }
    }
}

/// The term or word whose text holds `cursor`. Inside `any:(…)`, the inner
/// term when the cursor is on one; a negation outside the group still counts.
fn leaf_at(expr: &Expr, cursor: usize) -> Option<Found<'_>> {
    match expr {
        Expr::Or(items, _) | Expr::And(items, _) => {
            let mut found = items.iter().find_map(|item| leaf_at(item, cursor))?;
            found.grouped |= items.len() > 1;
            Some(found)
        }
        Expr::Not(inner, _) => {
            let mut found = leaf_at(inner, cursor)?;
            if found.grouped {
                found.in_negated_group = true;
            } else {
                found.negated = !found.negated;
            }
            Some(found)
        }
        Expr::Term(term) => {
            if cursor < term.span.start || cursor > term.span.end {
                return None;
            }
            if let Criterion::Group(inner, _) = &term.criterion
                && let Some(found) = leaf_at(inner, cursor)
            {
                return Some(found);
            }
            Some(Found::bare(expr))
        }
        Expr::Word(value) => {
            (value.span.start <= cursor && cursor <= value.span.end).then(|| Found::bare(expr))
        }
    }
}

/// The sentence for one predicate, without its full stop.
fn pred_words<C>(pred: &Pred, catalog: &C) -> String
where
    C: Catalog,
{
    match pred {
        Pred::Description(TextMatch::Contains(text)) => {
            format!("Description contains \u{201c}{text}\u{201d}")
        }
        Pred::Description(TextMatch::Equals(text)) => {
            format!("Description is \u{201c}{text}\u{201d}")
        }
        Pred::Account { id, subtree } => {
            let scope = if *subtree {
                " and its subaccounts"
            } else {
                " only"
            };
            format!("Legs on {}{scope}", path_of(catalog.accounts(), id))
        }
        Pred::Tag(TagPred::Any) => "Legs or transactions with any tag".to_owned(),
        Pred::Tag(TagPred::Tag { id, subtree }) => {
            let scope = if *subtree {
                " or a tag beneath it"
            } else {
                " only"
            };
            format!("Tagged {}{scope}", path_of(catalog.tags(), id))
        }
        Pred::Status(status) => format!("Transactions that are {}", status_word(*status)),
        Pred::Date(range) => format!("Dated {}", date_words(*range)),
        Pred::Amount(amount) => format!(
            "Leg amount {}, either sign, {}",
            range_words(&amount.range),
            currency(amount.commodity.as_deref())
        ),
        Pred::Commodity(code) => format!("Legs in {code}"),
        Pred::Meta { key, pred: meta } => meta_words(key, meta),
    }
}

/// The sentence for a metadata predicate on `key`.
fn meta_words(key: &str, pred: &MetaPred) -> String {
    match pred {
        MetaPred::Exists => format!("Has @{key}"),
        MetaPred::Text(TextMatch::Contains(text)) => {
            format!("@{key} contains \u{201c}{text}\u{201d}")
        }
        MetaPred::Text(TextMatch::Equals(text)) => format!("@{key} is \u{201c}{text}\u{201d}"),
        MetaPred::Number(range) => format!("@{key} {}", range_words(range)),
        MetaPred::Amount(amount) => format!(
            "@{key} {}, {}",
            range_words(&amount.range),
            currency(amount.commodity.as_deref())
        ),
        MetaPred::Boolean(value) => format!("@{key} is {value}"),
        MetaPred::Date(range) => format!("@{key} dated {}", date_words(*range)),
        MetaPred::Timestamp(range) => format!("@{key} {}", time_words(*range)),
        MetaPred::Account { path, subtree } => {
            let scope = if *subtree {
                " and its subaccounts"
            } else {
                " only"
            };
            format!("@{key} on {}{scope}", path.join(":"))
        }
    }
}

/// The colon-joined path of the entry with `id`, or the id when none has it.
fn path_of(entries: &[PathEntry], id: &str) -> String {
    entries
        .iter()
        .find(|entry| entry.id == id)
        .map_or_else(|| id.to_owned(), PathEntry::display)
}

/// A status as its query word.
const fn status_word(status: Status) -> &'static str {
    match status {
        Status::Unreconciled => "unreconciled",
        Status::Flagged => "flagged",
        Status::Reconciled => "reconciled",
        Status::Balanced => "balanced",
        Status::Unbalanced => "unbalanced",
    }
}

/// "in AUD", or "in any currency" without a marker.
fn currency(code: Option<&str>) -> String {
    code.map_or_else(|| "in any currency".to_owned(), |c| format!("in {c}"))
}

/// A numeric range in words: "over 100", "from 5 to 10", "exactly 150".
fn range_words(range: &NumRange) -> String {
    match (range.lo, range.hi) {
        (Some(lo), Some(hi)) if lo.value == hi.value && lo.inclusive && hi.inclusive => {
            format!("exactly {}", lo.value)
        }
        (Some(lo), Some(hi)) if lo.inclusive && hi.inclusive => {
            format!("from {} to {}", lo.value, hi.value)
        }
        (Some(lo), Some(hi)) => format!("{} and {}", lower(lo), upper(hi)),
        (Some(lo), None) => lower(lo),
        (None, Some(hi)) => upper(hi),
        (None, None) => "of any size".to_owned(),
    }
}

/// A lower bound in words.
fn lower(bound: Bound) -> String {
    if bound.inclusive {
        format!("at least {}", bound.value)
    } else {
        format!("over {}", bound.value)
    }
}

/// An upper bound in words.
fn upper(bound: Bound) -> String {
    if bound.inclusive {
        format!("at most {}", bound.value)
    } else {
        format!("under {}", bound.value)
    }
}

/// A date range in words; the exclusive upper bound shows as the day before.
fn date_words(range: DateRange) -> String {
    let last = range.until.and_then(|until| until.yesterday().ok());
    match (range.from, last) {
        (Some(from), Some(to)) if from == to => format!("on {from}"),
        (Some(from), Some(to)) => format!("{from} to {to}"),
        (Some(from), None) => format!("{from} or later"),
        (None, Some(to)) => format!("{to} or earlier"),
        (None, None) => "at any time".to_owned(),
    }
}

/// The end of an RFC 3339 instant at midnight UTC.
const MIDNIGHT: &str = "T00:00:00Z";

/// An instant range in words. An instant at midnight UTC shows as its date;
/// a range spanning exactly one such day reads "on" that day.
fn time_words(range: TimeRange) -> String {
    let show = |t: Timestamp| {
        let text = t.to_string();
        text.strip_suffix(MIDNIGHT)
            .map_or_else(|| text.clone(), str::to_owned)
    };
    match (range.from, range.until) {
        (Some(from), Some(until))
            if until.duration_since(from) == SignedDuration::from_hours(24)
                && show(from).len() == "0000-00-00".len() =>
        {
            format!("on {}", show(from))
        }
        (Some(from), Some(until)) => format!("from {} to before {}", show(from), show(until)),
        (Some(from), None) => format!("at or after {}", show(from)),
        (None, Some(until)) => format!("before {}", show(until)),
        (None, None) => "at any time".to_owned(),
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;

    use super::describe;
    use super::range_words;
    use crate::catalog::MetaKey;
    use crate::catalog::MetaType;
    use crate::catalog::PathEntry;
    use crate::catalog::Snapshot;
    use crate::currency::Commodity;
    use crate::filter::Bound;
    use crate::filter::NumRange;
    use crate::parser::parse;

    /// An invented ledger.
    fn catalog() -> Snapshot {
        Snapshot::new(
            vec![
                PathEntry::new("a1", ["Expenses", "Food"]),
                PathEntry::new("a2", ["Expenses", "Food", "Groceries"]),
            ],
            vec![
                PathEntry::new("t1", ["me"]),
                PathEntry::new("t3", ["institution"]),
            ],
            vec![Commodity::new("AUD", Some("A$"), &[])],
            vec![
                MetaKey::new("payee", MetaType::Text, 0),
                MetaKey::new("km", MetaType::Number, 0),
                MetaKey::new("due", MetaType::Date, 0),
                MetaKey::new("reimbursed", MetaType::Boolean, 0),
                MetaKey::new("deposit", MetaType::Amount, 0),
                MetaKey::new("synced", MetaType::Timestamp, 0),
                MetaKey::new("owner", MetaType::Account, 0),
            ],
        )
    }

    /// The words for `text` with the cursor at `cursor`.
    fn words(text: &str, cursor: usize) -> Option<String> {
        describe(&parse(text).expect("parses"), cursor, &catalog())
    }

    #[rstest]
    #[case("account:Expenses:Food", "Legs on Expenses:Food and its subaccounts.")]
    #[case("account:=Expenses:Food", "Legs on Expenses:Food only.")]
    #[case("amount:>100", "Leg amount over 100, either sign, in any currency.")]
    #[case("amount:A$5..10", "Leg amount from 5 to 10, either sign, in AUD.")]
    #[case("amount:<=7", "Leg amount at most 7, either sign, in any currency.")]
    #[case("amount:150", "Leg amount exactly 150, either sign, in any currency.")]
    #[case("date:2026-03", "Dated 2026-03-01 to 2026-03-31.")]
    #[case("date:<2026-03-01", "Dated 2026-02-28 or earlier.")]
    #[case("date:>=2026-03-01", "Dated 2026-03-01 or later.")]
    #[case("date:2026-03-15", "Dated on 2026-03-15.")]
    #[case("tag:institution", "Tagged institution or a tag beneath it.")]
    #[case("tag:=me", "Tagged me only.")]
    #[case("tag:*", "Legs or transactions with any tag.")]
    #[case("status:flagged", "Transactions that are flagged.")]
    #[case("commodity:AUD", "Legs in AUD.")]
    #[case("coffee", "Description contains \u{201c}coffee\u{201d}.")]
    #[case("description:=Rent", "Description is \u{201c}rent\u{201d}.")]
    #[case("@payee:cafe", "@payee contains \u{201c}cafe\u{201d}.")]
    #[case("@km:>=5", "@km at least 5.")]
    #[case("@due:*", "Has @due.")]
    #[case("@due:2026", "@due dated 2026-01-01 to 2026-12-31.")]
    #[case("@reimbursed:true", "@reimbursed is true.")]
    #[case("@deposit:>=A$50", "@deposit at least 50, in AUD.")]
    #[case("@deposit:10..20", "@deposit from 10 to 20, in any currency.")]
    #[case("@synced:2026-03-01", "@synced on 2026-03-01.")]
    #[case("@synced:2026-03", "@synced from 2026-03-01 to before 2026-04-01.")]
    #[case("@synced:>=2026-03-01", "@synced at or after 2026-03-01.")]
    #[case("@synced:<2026-03-01", "@synced before 2026-03-01.")]
    #[case(
        "@synced:>=2026-03-01T10:30:00Z",
        "@synced at or after 2026-03-01T10:30:00Z."
    )]
    #[case("@owner:Expenses:Food", "@owner on Expenses:Food and its subaccounts.")]
    #[case("@owner:=Expenses:Food", "@owner on Expenses:Food only.")]
    #[case("-@km:>=5", "Not: @km at least 5.")]
    #[case("-@payee:cafe", "Not: @payee contains \u{201c}cafe\u{201d}.")]
    #[case(
        "-account:Expenses:Food",
        "Not: Legs on Expenses:Food and its subaccounts."
    )]
    #[case(
        "-any:(tag:me)",
        "Not: Transactions with a leg that matches the group."
    )]
    #[case("-coffee", "Not: Description contains \u{201c}coffee\u{201d}.")]
    #[case("not -coffee", "Description contains \u{201c}coffee\u{201d}.")]
    fn describes_one_term(#[case] text: &str, #[case] expected: &str) {
        assert_eq!(words(text, text.len()).as_deref(), Some(expected));
    }

    #[rstest]
    #[case::inclusive_lower_exclusive_upper(Some((5, true)), Some((10, false)), "at least 5 and under 10")]
    #[case::exclusive_lower_inclusive_upper(Some((5, false)), Some((10, true)), "over 5 and at most 10")]
    #[case::both_exclusive(Some((5, false)), Some((10, false)), "over 5 and under 10")]
    #[case::exclusive_point(Some((5, false)), Some((5, false)), "over 5 and under 5")]
    #[case::unbounded(None, None, "of any size")]
    fn range_words_for_mixed_bounds(
        #[case] lo: Option<(i64, bool)>,
        #[case] hi: Option<(i64, bool)>,
        #[case] expected: &str,
    ) {
        let bound = |(value, inclusive): (i64, bool)| Bound::new(Decimal::from(value), inclusive);
        let range = NumRange::new(lo.map(bound), hi.map(bound));
        assert_eq!(range_words(&range), expected);
    }

    #[test]
    fn the_cursor_picks_the_term() {
        assert_eq!(
            words("tag:me coffee", 2).as_deref(),
            Some("Tagged me or a tag beneath it.")
        );
        assert_eq!(
            words("tag:me coffee", 10).as_deref(),
            Some("Description contains \u{201c}coffee\u{201d}.")
        );
        assert_eq!(
            words("any:(tag:me)", 2).as_deref(),
            Some("Transactions with a leg that matches the group.")
        );
        assert_eq!(
            words("any:(tag:me)", 7).as_deref(),
            Some("Tagged me or a tag beneath it.")
        );
        assert_eq!(
            words("-any:(tag:me)", 8).as_deref(),
            Some("Not: Tagged me or a tag beneath it.")
        );
    }

    #[rstest]
    #[case("-coffee", 3, "Not: Description contains \u{201c}coffee\u{201d}.")]
    #[case("--coffee", 3, "Description contains \u{201c}coffee\u{201d}.")]
    #[case("-(coffee)", 3, "Not: Description contains \u{201c}coffee\u{201d}.")]
    #[case(
        "-(coffee tea)",
        3,
        "In a negated group: Description contains \u{201c}coffee\u{201d}."
    )]
    #[case(
        "-any:(tag:me coffee)",
        8,
        "In a negated group: Tagged me or a tag beneath it."
    )]
    #[case(
        "-any:(tag:me coffee)",
        2,
        "Not: Transactions with a leg that matches the group."
    )]
    #[case(
        "-(-coffee tea)",
        4,
        "In a negated group: Not: Description contains \u{201c}coffee\u{201d}."
    )]
    fn a_negation_over_a_group_does_not_negate_its_term(
        #[case] text: &str,
        #[case] cursor: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(words(text, cursor).as_deref(), Some(expected));
    }

    #[test]
    fn nothing_describes_whitespace_or_an_error() {
        assert_eq!(words("a  b", 2), None);
        assert_eq!(words("acount:x", 3), None);
    }
}
