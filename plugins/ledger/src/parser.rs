//! Line-oriented parser for the Ledger plain-text format.

use rust_decimal::Decimal;

use crate::ast::ClearedStatus;
use crate::ast::Entry;
use crate::ast::Posting;
use crate::ast::PostingAmount;
use crate::ast::Transaction;

// TODO: consider migrating to nom parser combinators

/// Parses a complete Ledger file and returns its entries.
///
/// # Errors
///
/// Returns a string describing the parse error.
pub(crate) fn parse(input: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut lines = input.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Comment lines (`;`, `#`, `%`, `|`)
        if trimmed.starts_with([';', '#', '%', '|']) {
            let content = trimmed.get(1..).unwrap_or_default().trim();
            entries.push(Entry::Comment(content.to_owned()));
            continue;
        }

        // `*` at line-start is a top-level comment in Ledger, not a cleared flag.
        // (A `*` opener can never start with a digit, so no additional guard is needed.)
        if trimmed.starts_with('*') {
            let content = trimmed.get(1..).unwrap_or_default().trim();
            entries.push(Entry::Comment(content.to_owned()));
            continue;
        }

        if let Some(rest) = trimmed.strip_prefix("account ") {
            entries.push(Entry::AccountDecl(rest.trim().to_owned()));
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("commodity ") {
            entries.push(Entry::CommodityDecl(rest.trim().to_owned()));
            continue;
        }

        // Transaction header: starts with a digit (date)
        if trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            let tx = parse_transaction_header(trimmed, &mut lines)
                .map_err(|e| format!("parse error on '{trimmed}': {e}"))?;
            entries.push(Entry::Transaction(tx));
        }
    }

    Ok(entries)
}

/// Parses the transaction header line plus its indented posting lines.
fn parse_transaction_header<'a>(
    header: &str,
    lines: &mut core::iter::Peekable<impl Iterator<Item = &'a str>>,
) -> Result<Transaction, String> {
    if header.len() < 10 {
        return Err(format!("transaction header too short: '{header}'"));
    }
    let (date_str, after_date) = header.split_at(10);
    let date = parse_date(date_str)?;
    let header_rest = after_date.trim_start();

    let (cleared, payee_part) = if let Some(r) = header_rest.strip_prefix("* ") {
        (ClearedStatus::Cleared, r.trim_start())
    } else if let Some(r) = header_rest.strip_prefix("! ") {
        (ClearedStatus::Pending, r.trim_start())
    } else {
        (ClearedStatus::Uncleared, header_rest)
    };

    let (payee, comment) = split_comment(payee_part);

    let mut postings = Vec::new();
    while lines
        .peek()
        .is_some_and(|next| next.starts_with(' ') || next.starts_with('\t'))
    {
        let Some(posting_line) = lines.next() else {
            break;
        };
        let trimmed = posting_line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        postings.push(parse_posting(trimmed)?);
    }

    if postings.is_empty() {
        return Err("transaction has no postings".into());
    }

    Ok(Transaction {
        date,
        cleared,
        payee: payee.to_owned(),
        comment: comment.map(str::to_owned),
        postings,
    })
}

/// Parses a single posting line into a [`Posting`].
fn parse_posting(line: &str) -> Result<Posting, String> {
    let (account_and_amount, comment) = split_comment(line);
    let s = account_and_amount.trim();

    let amount = if let Some(pos) = find_double_space(s) {
        let amount_str = s.get(pos..).unwrap_or_default().trim();
        Some(parse_posting_amount(amount_str)?)
    } else {
        None
    };

    let account_end = find_double_space(s).unwrap_or(s.len());
    let account = s.get(..account_end).unwrap_or(s).trim().to_owned();

    Ok(Posting {
        account,
        amount,
        comment: comment.map(str::to_owned),
    })
}

/// Parses a Ledger posting amount string.
///
/// Accepts two styles:
/// - `50.00 AUD` (value then commodity)
/// - `$50.00` / `-$50.00` (symbol-prefixed)
#[expect(
    clippy::arithmetic_side_effects,
    reason = "negation of a parsed Decimal cannot overflow in practice"
)]
fn parse_posting_amount(raw: &str) -> Result<PostingAmount, String> {
    let s = raw.trim();

    // Try `<value> <commodity>` style first (most common).
    if let Some((value_part, commodity_part)) = s.rsplit_once(' ') {
        let maybe_value = value_part.trim();
        let maybe_commodity = commodity_part.trim();
        if let Ok(value) = maybe_value.parse::<Decimal>() {
            return Ok(PostingAmount {
                value,
                commodity: maybe_commodity.to_owned(),
            });
        }
    }

    // Try `<symbol><value>` style (e.g. `$50.00`, `-$50.00`).
    let (negative, magnitude) = if let Some(m) = s.strip_prefix('-') {
        (true, m)
    } else {
        (false, s)
    };
    let digit_start = magnitude
        .find(|c: char| c.is_ascii_digit())
        .ok_or_else(|| format!("cannot parse amount: '{s}'"))?;
    let (symbol, num_str) = magnitude.split_at(digit_start);
    let commodity = symbol.trim().to_owned();
    let abs_value: Decimal = num_str
        .parse()
        .map_err(|e| format!("cannot parse decimal '{num_str}': {e}"))?;
    let value = if negative { -abs_value } else { abs_value };
    Ok(PostingAmount { value, commodity })
}

/// Parses `YYYY-MM-DD` or `YYYY/MM/DD` into a [`bc_sdk::Date`].
fn parse_date(s: &str) -> Result<bc_sdk::Date, String> {
    let s = s.replace('/', "-");
    let bytes = s.as_bytes();
    if bytes.len() < 10 {
        return Err(format!("date too short: '{s}'"));
    }
    let year: i32 = s[0..4].parse().map_err(|_| format!("bad year in '{s}'"))?;
    let month: u8 = s[5..7].parse().map_err(|_| format!("bad month in '{s}'"))?;
    let day: u8 = s[8..10].parse().map_err(|_| format!("bad day in '{s}'"))?;
    bc_sdk::Date::try_new(year, month, day)
        .map_err(|e| format!("invalid date in '{s}': {e}"))
}

/// Splits a line at the first `;`, returning `(before, comment_text)`.
fn split_comment(s: &str) -> (&str, Option<&str>) {
    if let Some((raw_before, raw_after)) = s.split_once(';') {
        let before = raw_before.trim_end();
        let after = raw_after.trim();
        (before, if after.is_empty() { None } else { Some(after) })
    } else {
        (s, None)
    }
}

/// Returns the byte position of the first run of two or more spaces, or `None`.
fn find_double_space(s: &str) -> Option<usize> {
    s.as_bytes()
        .windows(2)
        .position(|w| matches!(w, [b' ', b' ', ..]))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::ast::ClearedStatus;
    use crate::ast::Entry;

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    #[expect(clippy::unwrap_used, reason = "test code; failure is a test failure")]
    fn parses_simple_transaction() {
        let input = "2025-01-15 * Woolworths\n    Expenses:Food    50.00 AUD\n    Assets:Bank   -50.00 AUD\n";
        let entries = parse(input).expect("parse");
        assert_eq!(entries.len(), 1);
        let Entry::Transaction(tx) = &entries[0] else {
            panic!("expected tx")
        };
        assert_eq!(tx.date, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(tx.cleared, ClearedStatus::Cleared);
        assert_eq!(tx.payee, "Woolworths");
        assert_eq!(tx.postings.len(), 2);
        assert_eq!(tx.postings[0].amount.as_ref().unwrap().value, dec!(50.00));
        assert_eq!(tx.postings[0].amount.as_ref().unwrap().commodity, "AUD");
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn parses_date_with_slashes() {
        let input =
            "2025/01/15 Salary\n    Assets:Bank    3000.00 AUD\n    Income:Salary  -3000.00 AUD\n";
        let entries = parse(input).expect("parse");
        let Entry::Transaction(tx) = &entries[0] else {
            panic!()
        };
        assert_eq!(tx.date, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(tx.cleared, ClearedStatus::Uncleared);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn parses_elided_last_posting() {
        let input = "2025-01-17 Rent\n    Expenses:Rent    1500.00 AUD\n    Assets:Bank\n";
        let entries = parse(input).expect("parse");
        let Entry::Transaction(tx) = &entries[0] else {
            panic!()
        };
        assert_eq!(tx.postings[1].amount, None);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn parses_comment_line() {
        let input = "; This is a comment\n";
        let entries = parse(input).expect("parse");
        assert_eq!(entries.len(), 1);
        assert!(matches!(entries[0], Entry::Comment(_)));
    }

    #[test]
    fn parses_multiple_transactions() {
        let input = "2025-01-15 * A\n    X    1.00 AUD\n    Y   -1.00 AUD\n\n2025-01-16 B\n    X    2.00 AUD\n    Y   -2.00 AUD\n";
        let entries = parse(input).expect("parse");
        let txs: Vec<_> = entries
            .iter()
            .filter(|e| matches!(e, Entry::Transaction(_)))
            .collect();
        assert_eq!(txs.len(), 2);
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn parses_pending_status() {
        let input = "2025-01-15 ! Pending\n    X    1.00 AUD\n    Y   -1.00 AUD\n";
        let entries = parse(input).expect("parse");
        let Entry::Transaction(tx) = &entries[0] else {
            panic!()
        };
        assert_eq!(tx.cleared, ClearedStatus::Pending);
    }
}
