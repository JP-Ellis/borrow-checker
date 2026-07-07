//! Line-oriented parser for the Beancount format.

use rust_decimal::Decimal;
use winnow::ModalResult;
use winnow::Parser;
use winnow::combinator::preceded;
use winnow::combinator::repeat;
use winnow::error::ParserError;
use winnow::token::take_till;
use winnow::token::take_while;

use crate::ast::Directive;
use crate::ast::Posting;
use crate::ast::PostingAmount;
use crate::ast::Transaction;
use crate::ast::TxFlag;

/// Parses a complete Beancount file and returns its directives.
///
/// # Arguments
///
/// * `input` - The Beancount file content as a string slice.
///
/// # Returns
///
/// A list of parsed directives in document order.
///
/// # Errors
///
/// Returns a string describing the first parse error encountered.
pub(crate) fn parse(input: &str) -> Result<Vec<Directive>, String> {
    let mut directives = Vec::new();
    let mut lines = input.lines().peekable();

    while let Some(line) = lines.next() {
        let trimmed = line.trim();

        if trimmed.is_empty()
            || trimmed.starts_with(';')
            || trimmed.starts_with('*')
            || trimmed.starts_with('#')
        {
            continue;
        }

        if !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }

        // A Beancount directive needs at least "YYYY-MM-DD" (10 bytes); a
        // shorter digit-leading line is not a directive. A full-length prefix
        // that fails to parse as a date is a hard error, as before.
        let Some(date_str) = trimmed.get(..10) else {
            directives.push(Directive::Other);
            continue;
        };
        let mut date_input = date_str;
        let date = date(&mut date_input).map_err(|_| format!("invalid date in '{date_str}'"))?;
        let rest = trimmed.get(10..).unwrap_or_default().trim_start();

        if let Some((flag, r)) = rest
            .strip_prefix("* ")
            .map(|r| (TxFlag::Complete, r))
            .or_else(|| rest.strip_prefix("! ").map(|r| (TxFlag::Incomplete, r)))
        {
            let (payee, narration, tags) = parse_payee_narration(r.trim_start())?;
            let postings = collect_postings(&mut lines)?;
            directives.push(Directive::Transaction(Transaction {
                date,
                flag,
                payee,
                narration,
                tags,
                postings,
            }));
        } else if let Some(r) = rest.strip_prefix("open ") {
            let mut parts = r.trim_start().splitn(2, ' ');
            let account = parts.next().unwrap_or("").to_owned();
            let currency = parts
                .next()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            directives.push(Directive::Open {
                date,
                account,
                currency,
            });
        } else if let Some(r) = rest.strip_prefix("close ") {
            directives.push(Directive::Close {
                date,
                account: r.trim().to_owned(),
            });
        } else if let Some(r) = rest.strip_prefix("commodity ") {
            directives.push(Directive::Commodity {
                date,
                code: r.trim().to_owned(),
            });
        } else if let Some(r) = rest.strip_prefix("balance ") {
            let mut parts = r.trim_start().splitn(3, ' ');
            let account = parts.next().unwrap_or("").to_owned();
            let amount_str = parts.next().unwrap_or("0");
            let currency = parts.next().unwrap_or("").trim().to_owned();
            let amount: Decimal = amount_str
                .parse()
                .map_err(|e| format!("bad balance amount: '{amount_str}': {e}"))?;
            directives.push(Directive::Balance {
                date,
                account,
                amount,
                currency,
            });
        } else {
            directives.push(Directive::Other);
        }
    }

    Ok(directives)
}

/// Parses the payee/narration/tags portion of a transaction header.
///
/// # Arguments
///
/// * `s` - The string slice after the flag character.
///
/// # Returns
///
/// A tuple of `(Option<payee>, narration, tags)`. Tags are `#`-prefixed
/// tokens following the quoted strings, in source order with the `#`
/// stripped; `^`-prefixed link tokens are recognised and ignored.
///
/// # Errors
///
/// Returns an error if the number of quoted strings is not 1 or 2.
fn parse_payee_narration(s: &str) -> Result<(Option<String>, String, Vec<String>), String> {
    let mut input = s;
    let strings: Vec<String> = repeat(0.., preceded(take_till(0.., '"'), quoted_string))
        .parse_next(&mut input)
        .unwrap_or_default();
    if input.contains('"') {
        return Err(format!("unterminated string in: '{s}'"));
    }
    let tags: Vec<String> = input
        .split_whitespace()
        .filter_map(|token| token.strip_prefix('#'))
        .map(str::to_owned)
        .collect();
    match strings.as_slice() {
        [only] => Ok((None, only.clone(), tags)),
        [payee, narration] => Ok((Some(payee.clone()), narration.clone(), tags)),
        _ => Err(format!(
            "expected 1 or 2 quoted strings, got {}: '{s}'",
            strings.len()
        )),
    }
}

/// Collects indented posting lines following a transaction header.
///
/// # Arguments
///
/// * `lines` - A peekable iterator over remaining input lines.
///
/// # Returns
///
/// A list of parsed postings.
///
/// # Errors
///
/// Returns an error if any posting line fails to parse.
fn collect_postings<'a>(
    lines: &mut core::iter::Peekable<impl Iterator<Item = &'a str>>,
) -> Result<Vec<Posting>, String> {
    let mut postings = Vec::new();
    while let Some(&next) = lines.peek() {
        let starts_indented = next.starts_with(' ') || next.starts_with('\t');
        if !starts_indented {
            break;
        }
        // We just peeked successfully, so `next()` must return `Some`.
        let Some(line) = lines.next() else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        postings.push(parse_posting(trimmed)?);
    }
    Ok(postings)
}

/// Parses a single posting line.
///
/// The amount and currency are optional: Beancount lets a posting omit them
/// entirely (e.g. `  Assets:Bank`), leaving the tool to derive the elided
/// amount so the transaction balances. A line with an account but no
/// double-space-separated amount parses to a `None` amount; a line that has
/// an amount section but a malformed number or missing currency still
/// errors.
///
/// # Arguments
///
/// * `line` - A trimmed posting line (without leading whitespace).
///
/// # Returns
///
/// A [`Posting`] with an account and an optional amount/currency.
///
/// # Errors
///
/// Returns an error if the posting has a malformed amount or currency.
fn parse_posting(line: &str) -> Result<Posting, String> {
    let line_no_comment = line.split(';').next().unwrap_or(line).trim_end();

    // Find a double-space separator between the account and amount. Its
    // absence means the amount is elided, not an error.
    let Some(split_pos) = line_no_comment
        .as_bytes()
        .windows(2)
        .position(|w| w.first().copied() == Some(b' ') && w.get(1).copied() == Some(b' '))
    else {
        return Ok(Posting {
            account: line_no_comment.trim().to_owned(),
            amount: None,
        });
    };

    let account = line_no_comment
        .get(..split_pos)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let rest = line_no_comment.get(split_pos..).unwrap_or_default().trim();

    let last_space = rest
        .rfind(' ')
        .ok_or_else(|| format!("posting missing currency: '{rest}'"))?;

    let amount_str = rest.get(..last_space).unwrap_or_default().trim();
    // `last_space` is the byte index of ' ', so `last_space + 1` points to the
    // next character. Since ' ' is a single-byte ASCII codepoint the resulting
    // index is always on a UTF-8 boundary; we use `.get()` for safety.
    let currency_start = last_space.saturating_add(1);
    let currency = rest
        .get(currency_start..)
        .unwrap_or_default()
        .trim()
        .to_owned();
    let value: Decimal = amount_str
        .parse()
        .map_err(|e| format!("bad posting amount '{amount_str}' in: '{line_no_comment}': {e}"))?;

    Ok(Posting {
        account,
        amount: Some(PostingAmount { value, currency }),
    })
}

/// Parses a strict `YYYY-MM-DD` date into a [`bc_sdk::Date`].
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
/// Backtracks if the input is not a `YYYY-MM-DD` date.
fn date(input: &mut &str) -> ModalResult<bc_sdk::Date> {
    let year: i32 = take_while(4, |c: char| c.is_ascii_digit())
        .try_map(str::parse)
        .parse_next(input)?;
    let _ = '-'.parse_next(input)?;
    let month: u8 = take_while(2, |c: char| c.is_ascii_digit())
        .try_map(str::parse)
        .parse_next(input)?;
    let _ = '-'.parse_next(input)?;
    let day: u8 = take_while(2, |c: char| c.is_ascii_digit())
        .try_map(str::parse)
        .parse_next(input)?;
    bc_sdk::Date::try_new(year, month, day).map_err(|_| winnow::error::ErrMode::from_input(input))
}

/// Parses a single `"`-delimited string, unescaping `\`-prefixed characters.
///
/// # Arguments
///
/// * `input` - The remaining input, expected to start at a `"`.
///
/// # Returns
///
/// The unescaped string contents.
///
/// # Errors
///
/// Backtracks if the input does not start with `"` or the string is
/// unterminated.
fn quoted_string(input: &mut &str) -> ModalResult<String> {
    let _ = '"'.parse_next(input)?;
    let mut buf = String::new();
    loop {
        let mut chars = input.chars();
        match chars.next() {
            Some('"') => {
                *input = chars.as_str();
                return Ok(buf);
            }
            Some('\\') => {
                let escaped = chars.next();
                *input = chars.as_str();
                if let Some(c) = escaped {
                    buf.push(c);
                }
            }
            Some(c) => {
                *input = chars.as_str();
                buf.push(c);
            }
            None => return Err(winnow::error::ErrMode::from_input(input)),
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::ast::Directive;
    use crate::ast::TxFlag;

    #[test]
    fn parses_complete_transaction_with_payee_and_narration() {
        let input = "2025-01-15 * \"Woolworths\" \"Weekly groceries\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank    -50.00 AUD\n";
        let directives = parse(input).expect("parse");
        let first = directives
            .first()
            .expect("should have at least one directive");
        let Directive::Transaction(tx) = first else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.date, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(tx.flag, TxFlag::Complete);
        assert_eq!(tx.payee.as_deref(), Some("Woolworths"));
        assert_eq!(tx.narration, "Weekly groceries");
        assert_eq!(tx.postings.len(), 2);
        let first_posting = tx.postings.first().expect("should have postings");
        let amount = first_posting.amount.as_ref().expect("explicit amount");
        assert_eq!(amount.value, dec!(50.00));
        assert_eq!(amount.currency, "AUD");
    }

    #[test]
    fn parses_transaction_narration_only() {
        let input = "2025-01-15 * \"Just a narration\"\n  X:Y    1.00 AUD\n  X:Z   -1.00 AUD\n";
        let directives = parse(input).expect("parse");
        let first = directives
            .first()
            .expect("should have at least one directive");
        let Directive::Transaction(tx) = first else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.payee, None);
        assert_eq!(tx.narration, "Just a narration");
    }

    #[test]
    fn parses_payee_narration_with_non_whitespace_separator() {
        let input = "2025-01-15 * \"Payee\"X\"Narration\"\n  X:Y    1.00 AUD\n  X:Z   -1.00 AUD\n";
        let directives = parse(input).expect("parse");
        let first = directives
            .first()
            .expect("should have at least one directive");
        let Directive::Transaction(tx) = first else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.payee.as_deref(), Some("Payee"));
        assert_eq!(tx.narration, "Narration");
    }

    #[test]
    fn parses_incomplete_flag() {
        let input = "2025-01-15 ! \"Pending\"\n  X:Y    1.00 AUD\n  X:Z   -1.00 AUD\n";
        let directives = parse(input).expect("parse");
        let first = directives
            .first()
            .expect("should have at least one directive");
        let Directive::Transaction(tx) = first else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.flag, TxFlag::Incomplete);
    }

    #[test]
    fn parses_open_directive() {
        let input = "2025-01-01 open Assets:Bank AUD\n";
        let directives = parse(input).expect("parse");
        let first = directives
            .first()
            .expect("should have at least one directive");
        assert!(matches!(first, Directive::Open { account, .. } if account == "Assets:Bank"));
    }

    #[test]
    fn parses_commodity_directive() {
        let input = "2025-01-01 commodity AUD\n";
        let directives = parse(input).expect("parse");
        let first = directives
            .first()
            .expect("should have at least one directive");
        assert!(matches!(first, Directive::Commodity { code, .. } if code == "AUD"));
    }

    #[test]
    fn comment_lines_skipped() {
        let input =
            "; comment\n* also comment\n2025-01-15 * \"X\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let directives = parse(input).expect("parse");
        assert_eq!(directives.len(), 1);
        let first = directives
            .first()
            .expect("should have at least one directive");
        assert!(matches!(first, Directive::Transaction(_)));
    }

    #[test]
    fn parse_date_rejects_invalid_separators() {
        let mut bad = "2025X01Y15";
        assert!(date(&mut bad).is_err(), "non-hyphen separators should fail");
        let mut good = "2025-01-15";
        assert!(date(&mut good).is_ok(), "valid date should parse");
    }

    #[test]
    fn date_parses_hyphenated() {
        let mut input = "2025-01-15 rest";
        let d = date(&mut input).expect("date");
        assert_eq!(d, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(input, " rest");
    }

    #[test]
    fn date_rejects_non_hyphen_separator() {
        let mut input = "2025X01Y15";
        assert!(date(&mut input).is_err());
    }

    #[test]
    fn quoted_string_reads_escapes() {
        let mut input = r#""a\"b" tail"#;
        let s = quoted_string(&mut input).expect("string");
        assert_eq!(s, "a\"b");
        assert_eq!(input, " tail");
    }

    #[test]
    fn quoted_string_unterminated_errors() {
        let mut input = r#""oops"#;
        assert!(quoted_string(&mut input).is_err());
    }

    #[test]
    fn parse_rejects_invalid_full_length_date() {
        let input = "2025-13-45 * \"Groceries\"\n";
        assert!(parse(input).is_err());
    }

    #[test]
    fn parse_treats_short_digit_leading_line_as_other() {
        let input = "2025\n";
        let directives = parse(input).expect("parse");
        assert_eq!(directives, vec![Directive::Other]);
    }

    #[test]
    fn parse_rejects_unterminated_quoted_string() {
        let input = "2025-01-15 * \"Payee\" \"Unterminated\n";
        assert!(parse(input).is_err());
    }

    #[test]
    fn parse_posting_without_amount_is_elided() {
        let input =
            "2025-01-15 * \"Payee\" \"Elided leg\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank\n";
        let directives = parse(input).expect("parse");
        let Directive::Transaction(tx) = directives.first().expect("directive") else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.postings.len(), 2);
        let explicit = tx.postings.first().expect("first posting");
        let amount = explicit.amount.as_ref().expect("explicit amount");
        assert_eq!(amount.value, dec!(50.00));
        assert_eq!(amount.currency, "AUD");
        let elided = tx.postings.get(1).expect("second posting");
        assert_eq!(elided.account, "Assets:Bank");
        assert_eq!(elided.amount, None);
    }

    #[test]
    fn parse_posting_with_malformed_amount_still_errors() {
        let input =
            "2025-01-15 * \"Payee\" \"Bad amount\"\n  Expenses:Food   fifty AUD\n  Assets:Bank\n";
        assert!(parse(input).is_err());
    }

    #[test]
    fn parse_transaction_header_collects_tags_in_order() {
        let input =
            "2025-06-27 * \"Payee\" \"Narration\" #josh #groceries\n  A:B   1.00 AUD\n  A:C\n";
        let directives = parse(input).expect("parse");
        let Directive::Transaction(tx) = directives.first().expect("directive") else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.tags, vec!["josh".to_owned(), "groceries".to_owned()]);
    }

    #[test]
    fn parse_transaction_header_without_tags_is_empty() {
        let input = "2025-01-15 * \"Payee\" \"Narration\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let directives = parse(input).expect("parse");
        let Directive::Transaction(tx) = directives.first().expect("directive") else {
            panic!("expected Transaction directive")
        };
        assert!(tx.tags.is_empty());
    }

    #[test]
    fn parse_transaction_header_ignores_link_tokens() {
        let input = "2025-01-15 * \"Payee\" \"Narration\" ^some-link #josh\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let directives = parse(input).expect("parse");
        let Directive::Transaction(tx) = directives.first().expect("directive") else {
            panic!("expected Transaction directive")
        };
        assert_eq!(tx.tags, vec!["josh".to_owned()]);
    }
}
