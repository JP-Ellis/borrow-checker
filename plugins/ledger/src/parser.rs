//! Line-oriented parser for the Ledger plain-text format.

use rust_decimal::Decimal;
use winnow::ModalResult;
use winnow::Parser;
use winnow::combinator::alt;
use winnow::error::ErrMode;
use winnow::error::ParserError;
use winnow::token::take_while;

use crate::ast::ClearedStatus;
use crate::ast::Entry;
use crate::ast::Posting;
use crate::ast::PostingAmount;
use crate::ast::Transaction;

/// Parses `YYYY-MM-DD` or `YYYY/MM/DD` into a [`bc_sdk::Date`].
///
/// # Arguments
///
/// * `input` - The remaining input; the date prefix is consumed on success.
///
/// # Returns
///
/// The parsed [`bc_sdk::Date`].
///
/// # Errors
///
/// Backtracks if the input is not a valid date.
fn date(input: &mut &str) -> ModalResult<bc_sdk::Date> {
    let year: i32 = take_while(4, |c: char| c.is_ascii_digit())
        .try_map(str::parse)
        .parse_next(input)?;
    let _ = alt(('-', '/')).parse_next(input)?;
    let month: u8 = take_while(2, |c: char| c.is_ascii_digit())
        .try_map(str::parse)
        .parse_next(input)?;
    let _ = alt(('-', '/')).parse_next(input)?;
    let day: u8 = take_while(2, |c: char| c.is_ascii_digit())
        .try_map(str::parse)
        .parse_next(input)?;
    bc_sdk::Date::try_new(year, month, day).map_err(|_| ErrMode::from_input(input))
}

/// Parses a complete Ledger file and returns its entries.
///
/// # Errors
///
/// Returns a string describing the parse error.
pub(crate) fn parse(input: &str) -> Result<Vec<Entry>, String> {
    let mut entries = Vec::new();
    let mut lines = input.lines().enumerate().peekable();

    while let Some((idx, line)) = lines.next() {
        let line_no = idx.saturating_add(1);
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
            let tx = parse_transaction_header(trimmed, line_no, &mut lines)
                .map_err(|e| format!("parse error on '{trimmed}': {e}"))?;
            entries.push(Entry::Transaction(tx));
        }
    }

    Ok(entries)
}

/// Parses the transaction header line plus its indented posting lines.
fn parse_transaction_header<'a>(
    header: &str,
    line_no: usize,
    lines: &mut core::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
) -> Result<Transaction, String> {
    let mut header_input = header;
    let date = date(&mut header_input).map_err(|_| format!("bad date in header: '{header}'"))?;
    let header_rest = header_input.trim_start();

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
        .is_some_and(|(_, next)| next.starts_with(' ') || next.starts_with('\t'))
    {
        let Some((_, posting_line)) = lines.next() else {
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
        line: line_no,
    })
}

/// Parses a single posting line into a [`Posting`].
fn parse_posting(line: &str) -> Result<Posting, String> {
    let (account_and_amount, comment) = split_comment(line);
    let s = account_and_amount.trim();

    let amount = if let Some(pos) = find_double_space(s) {
        let mut amount_str = s.get(pos..).unwrap_or_default().trim();
        Some(
            posting_amount(&mut amount_str)
                .map_err(|_| format!("cannot parse amount in posting: '{s}'"))?,
        )
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

/// Parses a Ledger posting amount.
///
/// Accepts `<value> <commodity>` (e.g. `50.00 AUD`) and `<symbol><value>`
/// (e.g. `$50.00`, `-$50.00`).
///
/// # Arguments
///
/// * `input` - The trimmed amount text (consumed to end on success).
///
/// # Returns
///
/// The parsed [`PostingAmount`].
///
/// # Errors
///
/// Backtracks if neither amount style matches.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "negation of a parsed Decimal cannot overflow in practice"
)]
fn posting_amount(input: &mut &str) -> ModalResult<PostingAmount> {
    let s = input.trim();

    // `<value> <commodity>` style (most common): split on the last space.
    if let Some((value_part, commodity_part)) = s.rsplit_once(' ')
        && let Ok(value) = value_part.trim().parse::<Decimal>()
    {
        *input = "";
        return Ok(PostingAmount {
            value,
            commodity: commodity_part.trim().to_owned(),
        });
    }

    // `<symbol><value>` style (e.g. `$50.00`, `-$50.00`).
    let (negative, magnitude) = match s.strip_prefix('-') {
        Some(m) => (true, m),
        None => (false, s),
    };
    let Some(digit_start) = magnitude.find(|c: char| c.is_ascii_digit()) else {
        return Err(ErrMode::from_input(input));
    };
    let (symbol, num_str) = magnitude.split_at(digit_start);
    let Ok(abs_value) = num_str.parse::<Decimal>() else {
        return Err(ErrMode::from_input(input));
    };
    *input = "";
    Ok(PostingAmount {
        value: if negative { -abs_value } else { abs_value },
        commodity: symbol.trim().to_owned(),
    })
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
    fn date_parses_hyphenated() {
        let mut input = "2025-01-15 rest";
        let d = date(&mut input).expect("date");
        assert_eq!(d, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(input, " rest");
    }

    #[test]
    fn date_parses_slashed() {
        let mut input = "2025/01/15";
        let d = date(&mut input).expect("date");
        assert_eq!(d, bc_sdk::Date::new(2025, 1, 15));
    }

    #[test]
    fn posting_amount_value_then_commodity() {
        let mut input = "50.00 AUD";
        let a = posting_amount(&mut input).expect("amount");
        assert_eq!(a.value, dec!(50.00));
        assert_eq!(a.commodity, "AUD");
    }

    #[test]
    fn posting_amount_symbol_prefixed() {
        let mut input = "-$50.00";
        let a = posting_amount(&mut input).expect("amount");
        assert_eq!(a.value, dec!(-50.00));
        assert_eq!(a.commodity, "$");
    }

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
    fn transaction_line_number_is_the_header_line_not_line_one() {
        // A comment and a blank line precede the transaction, so the header
        // sits on line 3. This catches both an off-by-one and a bug that
        // anchors the count to the wrong line (e.g. the first posting).
        let input = "; comment\n\n2025-01-15 * Woolworths\n    Expenses:Food    50.00 AUD\n    Assets:Bank   -50.00 AUD\n";
        let entries = parse(input).expect("parse");
        let Entry::Transaction(tx) = &entries[1] else {
            panic!("expected tx")
        };
        assert_eq!(
            tx.line, 3,
            "header is on line 3, not the first posting line"
        );
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
