//! Auto-detecting OFX parser: handles v1 (SGML) and v2 (XML).

use quick_xml::Reader;
use quick_xml::events::Event;
use rust_decimal::Decimal;

use crate::ast::OfxStatement;
use crate::ast::OfxTransaction;
use crate::sgml::SgmlToken;
use crate::sgml::tokenise;

/// Parses an OFX or QFX file (auto-detects v1 SGML vs v2 XML).
///
/// # Arguments
///
/// * `bytes` - Raw file bytes.
///
/// # Returns
///
/// A parsed [`OfxStatement`] containing all transactions.
///
/// # Errors
///
/// Returns a string describing the parse error.
pub(crate) fn parse(bytes: &[u8]) -> Result<OfxStatement, String> {
    // OFX v2 starts with `<?xml` or contains `OFXHEADER="200"`.
    // OFX v1 starts with `OFXHEADER:100` (no XML declaration).
    let prefix = bytes.get(..bytes.len().min(64)).unwrap_or(bytes);
    if prefix.windows(5).any(|w| w == b"<?xml")
        || prefix.windows(15).any(|w| w == b"OFXHEADER=\"200\"")
    {
        parse_v2(bytes)
    } else {
        parse_v1(bytes)
    }
}

// ── OFX v1 (SGML) ────────────────────────────────────────────────────────────

/// Parses OFX v1 (SGML) bytes into an [`OfxStatement`].
///
/// # Arguments
///
/// * `bytes` - Raw OFX v1 file bytes.
///
/// # Errors
///
/// Returns a string error if the file is not valid UTF-8 or required elements
/// (`CURDEF`, `ACCTID`) are missing.
fn parse_v1(bytes: &[u8]) -> Result<OfxStatement, String> {
    let text =
        core::str::from_utf8(bytes).map_err(|e| format!("OFX file is not valid UTF-8: {e}"))?;
    build_statement(tokenise(text), "v1")
}

// ── OFX v2 (XML) ─────────────────────────────────────────────────────────────

/// Parses OFX v2 (XML) bytes into an [`OfxStatement`].
///
/// # Arguments
///
/// * `bytes` - Raw OFX v2 file bytes.
///
/// # Errors
///
/// Returns a string error if the XML is malformed or required elements are
/// missing.
fn parse_v2(bytes: &[u8]) -> Result<OfxStatement, String> {
    build_statement(tokenise_xml(bytes)?, "v2")
}

/// Drives quick-xml over OFX v2 (XML) bytes, yielding the same [`SgmlToken`]
/// stream the v1 SGML lexer produces (tag names upper-cased; text folded into
/// a `Leaf` under the enclosing tag).
///
/// # Arguments
///
/// * `bytes` - Raw OFX v2 file bytes.
///
/// # Returns
///
/// The ordered token stream.
///
/// # Errors
///
/// Returns a string error if the XML is malformed.
fn tokenise_xml(bytes: &[u8]) -> Result<Vec<SgmlToken>, String> {
    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut tokens = Vec::new();
    let mut current_tag = String::new();
    let mut buf = Vec::new();
    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|xml_err| xml_err.to_string())?
        {
            Event::Start(ref e) => {
                current_tag = e.name().as_ref().to_ascii_uppercase();
                tokens.push(SgmlToken::Open(current_tag.clone()));
            }
            Event::End(ref e) => {
                let tag = e.name().as_ref().to_ascii_uppercase();
                tokens.push(SgmlToken::Close(tag));
            }
            Event::Text(ref e) => {
                let text = quick_xml::escape::unescape(e)
                    .map_err(|xml_err| xml_err.to_string())?
                    .trim()
                    .to_owned();
                if !text.is_empty() {
                    tokens.push(SgmlToken::Leaf {
                        tag: current_tag.clone(),
                        value: text,
                    });
                }
            }
            Event::Eof => break,
            Event::Empty(_)
            | Event::Comment(_)
            | Event::CData(_)
            | Event::Decl(_)
            | Event::PI(_)
            | Event::DocType(_)
            | Event::GeneralRef(_) => {}
        }
        buf.clear();
    }
    Ok(tokens)
}

// ── Shared walker ────────────────────────────────────────────────────────────

/// Walks an ordered [`SgmlToken`] stream into an [`OfxStatement`].
///
/// # Arguments
///
/// * `tokens` - Ordered tokens (from the v1 winnow lexer or the v2 quick-xml
///   adapter).
/// * `version_label` - Label used in error messages (e.g. `"v1"`, `"v2"`).
///
/// # Returns
///
/// The assembled [`OfxStatement`].
///
/// # Errors
///
/// Returns a string error if `CURDEF` or `ACCTID` is absent, or if a
/// transaction fails validation.
fn build_statement(
    tokens: impl IntoIterator<Item = SgmlToken>,
    version_label: &str,
) -> Result<OfxStatement, String> {
    let mut currency = String::new();
    let mut account_id = String::new();
    let mut transactions = Vec::new();
    let mut in_stmttrn = false;
    let mut current: Option<OfxTransactionBuilder> = None;

    for token in tokens {
        match token {
            SgmlToken::Open(ref tag) if tag == "STMTTRN" => {
                in_stmttrn = true;
                current = Some(OfxTransactionBuilder::default());
            }
            SgmlToken::Close(ref tag) if tag == "STMTTRN" => {
                in_stmttrn = false;
                if let Some(builder) = current.take() {
                    transactions.push(builder.build()?);
                }
            }
            SgmlToken::Leaf { tag, value } => {
                if in_stmttrn {
                    let builder = current.get_or_insert_with(OfxTransactionBuilder::default);
                    match tag.as_str() {
                        "TRNTYPE" => builder.trntype = value,
                        "DTPOSTED" => builder.dtposted = value,
                        "TRNAMT" => builder.trnamt = value,
                        "FITID" => builder.fitid = value,
                        "NAME" => builder.name = Some(value),
                        "MEMO" => builder.memo = Some(value),
                        _ => {}
                    }
                } else {
                    match tag.as_str() {
                        "CURDEF" => currency = value,
                        "ACCTID" => account_id = value,
                        _ => {}
                    }
                }
            }
            SgmlToken::Open(_) | SgmlToken::Close(_) => {}
        }
    }

    if currency.is_empty() {
        return Err(format!(
            "OFX {version_label}: missing CURDEF (currency) element"
        ));
    }
    if account_id.is_empty() {
        return Err(format!(
            "OFX {version_label}: missing ACCTID (account ID) element"
        ));
    }

    Ok(OfxStatement {
        currency,
        account_id,
        transactions,
    })
}

// ── Shared builder ────────────────────────────────────────────────────────────

/// Builder for [`OfxTransaction`] accumulating fields as they are parsed.
#[derive(Default)]
struct OfxTransactionBuilder {
    /// OFX transaction type.
    trntype: String,
    /// Value date string (raw OFX format).
    dtposted: String,
    /// Amount string (raw OFX format).
    trnamt: String,
    /// Unique transaction ID.
    fitid: String,
    /// Payee name.
    name: Option<String>,
    /// Memo / description.
    memo: Option<String>,
}

impl OfxTransactionBuilder {
    /// Consumes the builder and returns a validated [`OfxTransaction`].
    ///
    /// # Errors
    ///
    /// Returns a string error if `DTPOSTED` is absent or unparsable, or if
    /// `TRNAMT` is absent or not a valid decimal.  A missing `FITID` is
    /// permitted — it produces an empty string, which the importer converts
    /// to `reference = None`.
    #[inline]
    fn build(self) -> Result<OfxTransaction, String> {
        if self.dtposted.is_empty() {
            return Err("OFX transaction is missing required DTPOSTED element".into());
        }
        let date = parse_ofx_date(&self.dtposted)?;

        if self.trnamt.is_empty() {
            return Err("OFX transaction is missing required TRNAMT element".into());
        }
        let amount = self
            .trnamt
            .parse::<Decimal>()
            .map_err(|parse_err| format!("bad TRNAMT '{}': {parse_err}", self.trnamt))?;

        Ok(OfxTransaction {
            trntype: self.trntype,
            date,
            amount,
            fitid: self.fitid,
            name: self.name,
            memo: self.memo,
        })
    }
}

/// Parses an OFX date string: `YYYYMMDDHHMMSS[.mmm][TZ]` → [`bc_sdk::Date`].
///
/// Only the `YYYYMMDD` prefix (first 8 bytes) is used; time and timezone
/// components are ignored.
///
/// # Arguments
///
/// * `s` - Raw OFX date string.
///
/// # Returns
///
/// A [`bc_sdk::Date`] representing the date portion.
///
/// # Errors
///
/// Returns a string error if the date string is shorter than 8 characters or
/// if year, month, or day cannot be parsed as integers.
fn parse_ofx_date(s: &str) -> Result<bc_sdk::Date, String> {
    let ymd = s
        .get(..8)
        .ok_or_else(|| format!("OFX date too short: '{s}'"))?;
    let year: i32 = ymd[0..4]
        .parse()
        .map_err(|_| format!("bad year in OFX date '{s}'"))?;
    let month: u8 = ymd[4..6]
        .parse()
        .map_err(|_| format!("bad month in OFX date '{s}'"))?;
    let day: u8 = ymd[6..8]
        .parse()
        .map_err(|_| format!("bad day in OFX date '{s}'"))?;
    bc_sdk::Date::try_new(year, month, day).map_err(|e| format!("invalid OFX date '{s}': {e}"))
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;

    use super::*;

    const OFX_V1: &str = "\
OFXHEADER:100\r\nDATA:OFXSGML\r\nVERSION:102\r\n\r\n\
<OFX>\r\n<BANKMSGSRSV1>\r\n<STMTTRNRS>\r\n<STMTRS>\r\n\
<CURDEF>AUD\r\n<BANKACCTFROM>\r\n<ACCTID>123456789\r\n</BANKACCTFROM>\r\n\
<BANKTRANLIST>\r\n\
<STMTTRN>\r\n<TRNTYPE>DEBIT\r\n<DTPOSTED>20250115120000\r\n<TRNAMT>-50.00\r\n\
<FITID>20250115001\r\n<NAME>Woolworths\r\n<MEMO>Grocery purchase\r\n</STMTTRN>\r\n\
</BANKTRANLIST>\r\n</STMTRS>\r\n</STMTTRNRS>\r\n</BANKMSGSRSV1>\r\n</OFX>\r\n";

    const OFX_V2: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<?OFX OFXHEADER="200" VERSION="220"?>
<OFX>
  <BANKMSGSRSV1>
    <STMTTRNRS>
      <STMTRS>
        <CURDEF>AUD</CURDEF>
        <BANKACCTFROM><ACCTID>123456789</ACCTID></BANKACCTFROM>
        <BANKTRANLIST>
          <STMTTRN>
            <TRNTYPE>DEBIT</TRNTYPE>
            <DTPOSTED>20250115120000</DTPOSTED>
            <TRNAMT>-50.00</TRNAMT>
            <FITID>20250115001</FITID>
            <NAME>Woolworths</NAME>
            <MEMO>Grocery purchase</MEMO>
          </STMTTRN>
        </BANKTRANLIST>
      </STMTRS>
    </STMTTRNRS>
  </BANKMSGSRSV1>
</OFX>"#;

    #[test]
    fn build_statement_walks_token_stream() {
        let tokens = vec![
            SgmlToken::Leaf {
                tag: "CURDEF".into(),
                value: "AUD".into(),
            },
            SgmlToken::Leaf {
                tag: "ACCTID".into(),
                value: "123".into(),
            },
            SgmlToken::Open("STMTTRN".into()),
            SgmlToken::Leaf {
                tag: "DTPOSTED".into(),
                value: "20250115".into(),
            },
            SgmlToken::Leaf {
                tag: "TRNAMT".into(),
                value: "-50.00".into(),
            },
            SgmlToken::Leaf {
                tag: "FITID".into(),
                value: "X1".into(),
            },
            SgmlToken::Close("STMTTRN".into()),
        ];
        let stmt = build_statement(tokens, "test").expect("walk");
        assert_eq!(stmt.currency, "AUD");
        assert_eq!(stmt.account_id, "123");
        assert_eq!(stmt.transactions.len(), 1);
    }

    #[test]
    fn tokenise_xml_matches_sgml_token_shape() {
        let xml = "<OFX><CURDEF>AUD</CURDEF><STMTTRN><TRNAMT>-1.00</TRNAMT></STMTTRN></OFX>";
        let tokens = tokenise_xml(xml.as_bytes()).expect("tokenise xml");
        assert_eq!(
            tokens,
            vec![
                SgmlToken::Open("OFX".into()),
                SgmlToken::Open("CURDEF".into()),
                SgmlToken::Leaf {
                    tag: "CURDEF".into(),
                    value: "AUD".into()
                },
                SgmlToken::Close("CURDEF".into()),
                SgmlToken::Open("STMTTRN".into()),
                SgmlToken::Open("TRNAMT".into()),
                SgmlToken::Leaf {
                    tag: "TRNAMT".into(),
                    value: "-1.00".into()
                },
                SgmlToken::Close("TRNAMT".into()),
                SgmlToken::Close("STMTTRN".into()),
                SgmlToken::Close("OFX".into()),
            ]
        );
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn parses_ofx_v1() {
        let stmt = parse(OFX_V1.as_bytes()).expect("parse v1");
        assert_eq!(stmt.currency, "AUD");
        assert_eq!(stmt.account_id, "123456789");
        assert_eq!(stmt.transactions.len(), 1);
        let tx = &stmt.transactions[0];
        assert_eq!(tx.date, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(tx.amount, dec!(-50.00));
        assert_eq!(tx.fitid, "20250115001");
        assert_eq!(tx.name.as_deref(), Some("Woolworths"));
        assert_eq!(tx.memo.as_deref(), Some("Grocery purchase"));
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn parses_ofx_v2() {
        let stmt = parse(OFX_V2.as_bytes()).expect("parse v2");
        assert_eq!(stmt.currency, "AUD");
        assert_eq!(stmt.transactions.len(), 1);
        let tx = &stmt.transactions[0];
        assert_eq!(tx.date, bc_sdk::Date::new(2025, 1, 15));
        assert_eq!(tx.amount, dec!(-50.00));
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn parses_ofx_date_yyyymmdd_only() {
        let input = OFX_V1.replace("20250115120000", "20250115");
        let stmt = parse(input.as_bytes()).expect("parse");
        assert_eq!(stmt.transactions[0].date, bc_sdk::Date::new(2025, 1, 15));
    }

    #[test]
    fn missing_dtposted_returns_error() {
        let input = OFX_V1.replace("<DTPOSTED>20250115120000\r\n", "");
        let result = parse(input.as_bytes());
        assert!(
            result.is_err(),
            "expected error when DTPOSTED is absent, got: {result:?}"
        );
    }

    #[test]
    fn missing_trnamt_returns_error() {
        let input = OFX_V1.replace("<TRNAMT>-50.00\r\n", "");
        let result = parse(input.as_bytes());
        assert!(
            result.is_err(),
            "expected error when TRNAMT is absent, got: {result:?}"
        );
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is desired"
    )]
    fn empty_fitid_produces_none_reference_in_importer() {
        // A transaction with no FITID should succeed but produce no reference.
        let input = OFX_V1.replace("<FITID>20250115001\r\n", "");
        let stmt = parse(input.as_bytes()).expect("parse should succeed with missing FITID");
        assert!(
            stmt.transactions[0].fitid.is_empty(),
            "fitid should be empty string when absent"
        );
    }
}
