//! Minimal SGML tokenizer for OFX v1 files.
//!
//! OFX v1 is not valid XML: leaf-value elements have no closing tag.
//! This tokenizer emits [`SgmlToken`] values from a raw byte slice.

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

    while let Some(lt_pos) = remaining.find('<') {
        // Advance to the `<`.
        remaining = remaining.get(lt_pos..).unwrap_or_default();

        // Find the matching `>`.
        let Some(gt_pos) = remaining.find('>') else {
            // Malformed — no closing `>`, skip rest of input.
            break;
        };

        // The tag raw text sits between `<` and `>`.
        let tag_raw = remaining.get(1..gt_pos).unwrap_or_default();

        if let Some(close_tag) = tag_raw.strip_prefix('/') {
            // Close tag: `</TAGNAME>`
            let tag = close_tag.trim().to_ascii_uppercase();
            tokens.push(SgmlToken::Close(tag));
            remaining = remaining
                .get(gt_pos.saturating_add(1)..)
                .unwrap_or_default();
        } else {
            let tag = tag_raw.trim().to_ascii_uppercase();
            // Advance past `>` and read the value (up to next `<` or newline).
            remaining = remaining
                .get(gt_pos.saturating_add(1)..)
                .unwrap_or_default();
            let value_end = remaining.find(['<', '\n', '\r']).unwrap_or(remaining.len());
            let value = remaining.get(..value_end).unwrap_or_default().trim();
            if value.is_empty() {
                // `<TAG>` with nothing after — aggregate open tag.
                tokens.push(SgmlToken::Open(tag));
            } else {
                // `<TAG>value` — leaf element.
                tokens.push(SgmlToken::Leaf {
                    tag,
                    value: value.to_owned(),
                });
            }
            remaining = remaining.get(value_end..).unwrap_or_default();
        }
    }

    tokens
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
