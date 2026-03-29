//! Line-oriented parser for the Beancount format.

use jiff::civil::Date;
use rust_decimal::Decimal;

use crate::ast::Directive;
use crate::ast::Posting;
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

        // All Beancount date strings are ASCII so byte-boundary slicing is safe.
        // We need at least "YYYY-MM-DD " (11 bytes) for a valid directive.
        let Some(date_str) = trimmed.get(..10) else {
            directives.push(Directive::Other);
            continue;
        };
        let date = parse_date(date_str)?;
        let rest = trimmed.get(10..).unwrap_or_default().trim_start();

        if let Some(r) = rest.strip_prefix("* ").or_else(|| rest.strip_prefix("! ")) {
            let flag = if rest.starts_with('*') {
                TxFlag::Complete
            } else {
                TxFlag::Incomplete
            };
            let (payee, narration) = parse_payee_narration(r.trim_start())?;
            let postings = collect_postings(&mut lines)?;
            directives.push(Directive::Transaction(Transaction {
                date,
                flag,
                payee,
                narration,
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

/// Parses the payee/narration portion of a transaction header.
///
/// # Arguments
///
/// * `s` - The string slice after the flag character.
///
/// # Returns
///
/// A tuple of `(Option<payee>, narration)`.
///
/// # Errors
///
/// Returns an error if the number of quoted strings is not 1 or 2.
fn parse_payee_narration(s: &str) -> Result<(Option<String>, String), String> {
    let strings = extract_quoted_strings(s)?;
    match strings.as_slice() {
        [only] => Ok((None, only.clone())),
        [payee, narration] => Ok((Some(payee.clone()), narration.clone())),
        _ => Err(format!(
            "expected 1 or 2 quoted strings, got {}: '{s}'",
            strings.len()
        )),
    }
}

/// Extracts all double-quoted strings from a line.
///
/// # Arguments
///
/// * `s` - The string slice to scan.
///
/// # Returns
///
/// A list of unescaped string contents.
///
/// # Errors
///
/// Returns an error if a quoted string is unterminated.
fn extract_quoted_strings(s: &str) -> Result<Vec<String>, String> {
    let mut strings = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            let mut buf = String::new();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some('\\') => {
                        if let Some(escaped) = chars.next() {
                            buf.push(escaped);
                        }
                    }
                    Some(ch) => buf.push(ch),
                    None => return Err(format!("unterminated string in: '{s}'")),
                }
            }
            strings.push(buf);
        }
    }
    Ok(strings)
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
/// # Arguments
///
/// * `line` - A trimmed posting line (without leading whitespace).
///
/// # Returns
///
/// A [`Posting`] with account, amount, and currency.
///
/// # Errors
///
/// Returns an error if the posting cannot be parsed.
fn parse_posting(line: &str) -> Result<Posting, String> {
    let line_no_comment = line.split(';').next().unwrap_or(line).trim_end();

    // Find a double-space separator between the account and amount.
    let split_pos = line_no_comment
        .as_bytes()
        .windows(2)
        .position(|w| w.first().copied() == Some(b' ') && w.get(1).copied() == Some(b' '))
        .ok_or_else(|| format!("posting missing amount: '{line_no_comment}'"))?;

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
    let amount: Decimal = amount_str
        .parse()
        .map_err(|e| format!("bad posting amount '{amount_str}' in: '{line_no_comment}': {e}"))?;

    Ok(Posting {
        account,
        amount,
        currency,
    })
}

/// Parses a `YYYY-MM-DD` date string.
///
/// # Arguments
///
/// * `s` - A 10-character date string.
///
/// # Returns
///
/// The parsed [`Date`].
///
/// # Errors
///
/// Returns an error if the date cannot be parsed.
fn parse_date(s: &str) -> Result<Date, String> {
    jiff::civil::Date::strptime("%Y-%m-%d", s).map_err(|e| format!("bad date '{s}': {e}"))
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;
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
        assert_eq!(tx.date, date(2025, 1, 15));
        assert_eq!(tx.flag, TxFlag::Complete);
        assert_eq!(tx.payee.as_deref(), Some("Woolworths"));
        assert_eq!(tx.narration, "Weekly groceries");
        assert_eq!(tx.postings.len(), 2);
        let first_posting = tx.postings.first().expect("should have postings");
        assert_eq!(first_posting.amount, dec!(50.00));
        assert_eq!(first_posting.currency, "AUD");
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
}
