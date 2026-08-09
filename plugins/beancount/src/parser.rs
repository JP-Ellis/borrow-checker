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

/// Undated directive keywords the importer knowingly ignores.
///
/// Listing them explicitly means an unrecognised keyword can be warned about
/// without also warning about every legitimate non-transaction line.
const TOLERATED_UNDATED: [&str; 6] = [
    "option", "plugin", "pushtag", "poptag", "pushmeta", "popmeta",
];

/// Dated directive keywords the importer knowingly ignores.
///
/// `open`, `close`, `commodity` and `balance` are parsed into their own
/// variants and so are absent here.
const TOLERATED_DATED: [&str; 7] = [
    "note", "price", "event", "document", "pad", "query", "custom",
];

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
    let mut lines = input.lines().enumerate().peekable();

    while let Some((idx, line)) = lines.next() {
        let line_no = idx.saturating_add(1);
        let trimmed = line.trim();

        if trimmed.is_empty()
            || trimmed.starts_with(';')
            || trimmed.starts_with('*')
            || trimmed.starts_with('#')
        {
            continue;
        }

        // An indented line belongs to the directive above it — a posting leg
        // or a metadata key — and is never a top-level directive. Postings are
        // consumed by `collect_postings`; anything else here is metadata.
        if line.starts_with([' ', '\t']) {
            continue;
        }

        if !trimmed.starts_with(|c: char| c.is_ascii_digit()) {
            directives.push(undated_directive(trimmed, line_no)?);
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
                line: line_no,
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
            directives.push(dated_directive(rest, line_no));
        }
    }

    Ok(directives)
}

/// Classifies a top-level line that does not begin with a date.
///
/// # Arguments
///
/// * `trimmed` - The line with surrounding whitespace removed.
/// * `line` - The 1-based source line number.
///
/// # Returns
///
/// [`Directive::Include`] for an `include`, [`Directive::Other`] for a
/// tolerated keyword, and [`Directive::Unknown`] otherwise.
///
/// # Errors
///
/// Returns an error if an `include` directive has no quoted path argument.
fn undated_directive(trimmed: &str, line: usize) -> Result<Directive, String> {
    let keyword = trimmed.split_whitespace().next().unwrap_or_default();

    if keyword == "include" {
        let rest = trimmed
            .get(keyword.len()..)
            .unwrap_or_default()
            .trim_start();
        let mut input = rest;
        let path = quoted_string(&mut input)
            .map_err(|_| format!("line {line}: include directive has no quoted path"))?;
        return Ok(Directive::Include { path, line });
    }

    if TOLERATED_UNDATED.contains(&keyword) {
        Ok(Directive::Other)
    } else {
        Ok(Directive::Unknown {
            keyword: keyword.to_owned(),
            line,
        })
    }
}

/// Classifies a dated line whose keyword has no dedicated AST variant.
///
/// # Arguments
///
/// * `rest` - The portion of the line following the date, left-trimmed.
/// * `line` - The 1-based source line number.
///
/// # Returns
///
/// [`Directive::Other`] for a tolerated keyword, [`Directive::Unknown`]
/// otherwise.
fn dated_directive(rest: &str, line: usize) -> Directive {
    let keyword = rest.split_whitespace().next().unwrap_or_default();
    if TOLERATED_DATED.contains(&keyword) {
        Directive::Other
    } else {
        Directive::Unknown {
            keyword: keyword.to_owned(),
            line,
        }
    }
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
/// stripped; `^`-prefixed link tokens are recognised and ignored. A trailing
/// `;` comment is stripped before tags are collected, and empty `#` tokens are
/// dropped.
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
    // A trailing `;` comment must not contribute tags; strip it first (mirrors
    // `parse_posting`). Empty `#` tokens are dropped.
    let tag_region = input.split(';').next().unwrap_or(input);
    let tags: Vec<String> = tag_region
        .split_whitespace()
        .filter_map(|token| token.strip_prefix('#'))
        .filter(|tag| !tag.is_empty())
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
    lines: &mut core::iter::Peekable<impl Iterator<Item = (usize, &'a str)>>,
) -> Result<Vec<Posting>, String> {
    let mut postings = Vec::new();
    while let Some(&(_, next)) = lines.peek() {
        let starts_indented = next.starts_with(' ') || next.starts_with('\t');
        if !starts_indented {
            break;
        }
        // We just peeked successfully, so `next()` must return `Some`.
        let Some((_, line)) = lines.next() else {
            break;
        };
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
    fn transaction_line_number_is_the_header_line_not_line_one() {
        // A comment and a blank line precede the transaction, so the header
        // sits on line 3. This catches both an off-by-one and a bug that
        // anchors the count to the wrong line (e.g. the first posting).
        let input = "; comment\n\n2025-01-15 * \"Woolworths\" \"Groceries\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank  -50.00 AUD\n";
        let directives = parse(input).expect("parse");
        let Directive::Transaction(tx) = directives.first().expect("directive") else {
            panic!("expected Transaction directive")
        };
        assert_eq!(
            tx.line, 3,
            "header is on line 3, not the first posting line"
        );
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
    fn parse_transaction_header_ignores_tags_in_comment() {
        let input =
            "2025-06-27 * \"Payee\" \"Narration\" #real ; note #fake\n  A:B   1.00 AUD\n  A:C\n";
        let directives = parse(input).expect("parse");
        let Directive::Transaction(tx) = directives.first().expect("directive") else {
            panic!("expected Transaction directive")
        };
        assert_eq!(
            tx.tags,
            vec!["real".to_owned()],
            "a #token inside a trailing ; comment must not be collected as a tag"
        );
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

    #[test]
    fn parses_include_directive() {
        let directives = parse("include \"ledger/2025-01.bean\"\n").expect("parse");
        assert_eq!(
            directives,
            vec![Directive::Include {
                path: "ledger/2025-01.bean".to_owned(),
                line: 1,
            }]
        );
    }

    #[test]
    fn parses_include_path_containing_spaces_and_colons() {
        // The path is a quoted string, so neither a space nor the account
        // separator terminates it.
        let directives = parse("include \"my ledger/a:b.bean\"\n").expect("parse");
        assert_eq!(
            directives,
            vec![Directive::Include {
                path: "my ledger/a:b.bean".to_owned(),
                line: 1,
            }]
        );
    }

    #[test]
    fn include_without_a_quoted_path_is_a_parse_error() {
        // An include the parser cannot read is not a warning: the importer would
        // have no way to know what it failed to pull in.
        let err = parse("include ledger.bean\n").expect_err("malformed include must fail");
        assert!(
            err.contains("include") && err.contains("1"),
            "the error names the directive and its line: {err}"
        );
    }

    #[test]
    fn tolerated_undated_directives_are_other() {
        let directives =
            parse("option \"title\" \"Household\"\nplugin \"beancount.plugins.auto\"\n")
                .expect("parse");
        assert_eq!(directives, vec![Directive::Other, Directive::Other]);
    }

    #[test]
    fn tolerated_dated_directives_are_other() {
        let directives = parse(
            "2025-01-01 custom \"budget\" Expenses:Food\n2025-01-02 note Assets:Bank \"hi\"\n",
        )
        .expect("parse");
        assert_eq!(directives, vec![Directive::Other, Directive::Other]);
    }

    #[test]
    fn unrecognised_undated_keyword_is_unknown() {
        let directives = parse("; a comment\nfrobnicate whatever\n").expect("parse");
        assert_eq!(
            directives,
            vec![Directive::Unknown {
                keyword: "frobnicate".to_owned(),
                line: 2,
            }]
        );
    }

    #[test]
    fn unrecognised_dated_keyword_is_unknown() {
        // `txn` is a real Beancount transaction form this parser does not handle.
        // It must announce itself rather than vanish.
        let directives = parse("2025-01-01 txn \"Payee\" \"Narration\"\n").expect("parse");
        assert_eq!(
            directives,
            vec![Directive::Unknown {
                keyword: "txn".to_owned(),
                line: 1,
            }]
        );
    }

    #[test]
    fn indented_lines_are_never_top_level_directives() {
        // Metadata hanging off a directive is indented. Treating it as a
        // top-level directive would warn about every metadata key in the ledger.
        let directives =
            parse("2025-01-01 open Assets:Bank AUD\n  export: \"schwab\"\n").expect("parse");
        assert_eq!(
            directives,
            vec![Directive::Open {
                date: bc_sdk::Date::new(2025, 1, 1),
                account: "Assets:Bank".to_owned(),
                currency: Some("AUD".to_owned()),
            }]
        );
    }
}
