//! Minimal SGML tokenizer for OFX v1 files.
//!
//! OFX v1 is not valid XML: leaf-value elements have no closing tag.
//! This tokenizer emits [`SgmlToken`] values from a raw byte slice.

use winnow::ModalResult;
use winnow::Parser;
use winnow::token::take_until;
use winnow::token::take_while;

/// A token from an OFX v1 SGML stream.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SgmlToken {
    /// An aggregate open tag: `<STMTTRN>`.
    Open(String),
    /// An aggregate close tag: `</STMTTRN>`.
    Close(String),
    /// A leaf value element: `<TRNAMT>-50.00` (no closing tag).
    Leaf {
        /// The uppercased tag name, e.g. `"TRNAMT"`.
        tag: String,
        /// The trimmed text value following the tag.
        value: String,
    },
}

/// Tokenises raw OFX v1 SGML text.
///
/// OFX v1 header key:value lines (lines without `<`) are silently skipped.
/// Handles both one-tag-per-line and multiple-tags-per-line layouts. Leaf
/// elements (`<TAG>value`) have no closing tag; their value runs until the
/// next `<` or end-of-line.
pub(crate) fn tokenise(input: &str) -> Vec<SgmlToken> {
    let mut tokens = Vec::new();
    let mut remaining = input;
    while let Some(token) = next_token(&mut remaining) {
        tokens.push(token);
    }
    tokens
}

/// Consumes the next SGML token from `input`, or returns `None` at end.
fn next_token(input: &mut &str) -> Option<SgmlToken> {
    // Skip everything up to the next `<` (header lines, whitespace, values).
    skip_to_lt.parse_next(input).ok()?;

    // Consume the tag body between `<` and `>`.
    let tag_raw: &str = tag_body.parse_next(input).ok()?;

    if let Some(close) = tag_raw.strip_prefix('/') {
        return Some(SgmlToken::Close(close.trim().to_ascii_uppercase()));
    }

    let tag = tag_raw.trim().to_ascii_uppercase();
    // Value runs to the next `<`, `\n`, or `\r`.
    let value = leaf_value.parse_next(input).unwrap_or_default().trim();
    let token = if value.is_empty() {
        SgmlToken::Open(tag)
    } else {
        SgmlToken::Leaf {
            tag,
            value: value.to_owned(),
        }
    };
    Some(token)
}

/// Discards input up to (but not including) the next `<`.
///
/// # Errors
///
/// Returns an error if no `<` remains in the input.
fn skip_to_lt(input: &mut &str) -> ModalResult<()> {
    take_until(0.., "<").void().parse_next(input)
}

/// Consumes `<...>` and returns the raw text between the angle brackets.
///
/// # Errors
///
/// Returns an error if no closing `>` is found.
fn tag_body<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    '<'.parse_next(input)?;
    let body = take_until(0.., ">").parse_next(input)?;
    '>'.parse_next(input)?;
    Ok(body)
}

/// Consumes a leaf value: everything up to the next `<`, `\n`, or `\r`.
///
/// This parser is infallible; it never returns an error.
fn leaf_value<'i>(input: &mut &'i str) -> ModalResult<&'i str> {
    take_while(0.., |c| c != '<' && c != '\n' && c != '\r').parse_next(input)
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn tokenises_leaf_value_element() {
        let input = "<TRNAMT>-50.00\n<FITID>12345\n";
        let tokens = tokenise(input);
        assert_eq!(
            tokens,
            vec![
                SgmlToken::Leaf {
                    tag: "TRNAMT".into(),
                    value: "-50.00".into()
                },
                SgmlToken::Leaf {
                    tag: "FITID".into(),
                    value: "12345".into()
                },
            ]
        );
    }

    #[test]
    fn tokenises_open_and_close_tags() {
        let input = "<STMTTRN>\n<TRNAMT>-50.00\n</STMTTRN>\n";
        let tokens = tokenise(input);
        assert_eq!(
            tokens,
            vec![
                SgmlToken::Open("STMTTRN".into()),
                SgmlToken::Leaf {
                    tag: "TRNAMT".into(),
                    value: "-50.00".into()
                },
                SgmlToken::Close("STMTTRN".into()),
            ]
        );
    }

    #[test]
    fn skips_ofx_header_lines() {
        let input = "OFXHEADER:100\nDATA:OFXSGML\n\n<OFX>\n<CURDEF>AUD\n</OFX>\n";
        let tokens = tokenise(input);
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, SgmlToken::Open(s) if s == "OFX"))
        );
        assert!(
            tokens
                .iter()
                .any(|t| matches!(t, SgmlToken::Leaf { tag, .. } if tag == "CURDEF"))
        );
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn value_is_trimmed() {
        let input = "<NAME>  Woolworths  \n";
        let tokens = tokenise(input);
        assert_eq!(
            tokens[0],
            SgmlToken::Leaf {
                tag: "NAME".into(),
                value: "Woolworths".into()
            }
        );
    }

    #[test]
    fn tokenises_inline_multiple_tags_on_one_line() {
        let input = "<STMTTRN><TRNTYPE>DEBIT<TRNAMT>-50.00</STMTTRN>";
        let tokens = tokenise(input);
        assert_eq!(
            tokens,
            vec![
                SgmlToken::Open("STMTTRN".into()),
                SgmlToken::Leaf {
                    tag: "TRNTYPE".into(),
                    value: "DEBIT".into()
                },
                SgmlToken::Leaf {
                    tag: "TRNAMT".into(),
                    value: "-50.00".into()
                },
                SgmlToken::Close("STMTTRN".into()),
            ]
        );
    }
}
