//! BorrowChecker plugin for the OFX/QFX open financial exchange format.
//!
//! Implements [`bc_sdk::Importer`] for OFX v1 (SGML) and OFX v2 (XML) files.

mod ast;
mod parser;
mod sgml;

use bc_sdk::{Amount, ImportConfig, ImportError, RawTransaction};
use rust_decimal::Decimal;

use crate::parser::parse;

/// Implements [`bc_sdk::Importer`] for OFX v1 (SGML) and OFX v2 (XML) bank statement files.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct OfxImporter;

impl OfxImporter {
    /// Creates a new [`OfxImporter`].
    ///
    /// # Returns
    ///
    /// A new [`OfxImporter`] instance.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

#[bc_sdk::importer]
impl bc_sdk::Importer for OfxImporter {
    /// Returns the stable identifier for this importer.
    #[inline]
    fn name(&self) -> &str {
        "ofx"
    }

    /// Returns `true` if `bytes` appear to be an OFX or QFX file.
    ///
    /// Detection heuristic: checks for OFX v1 (`OFXHEADER:1`) or OFX v2
    /// (`OFXHEADER="200"` or `<?xml` prefix with `<OFX>` or `<OFX ` tag).
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        let is_v1 = bytes.windows(11).any(|w| w == b"OFXHEADER:1");
        let is_v2 = bytes.windows(15).any(|w| w == b"OFXHEADER=\"200\"")
            || (bytes.starts_with(b"<?xml")
                && (bytes.windows(5).any(|w| w == b"<OFX>")
                    || bytes.windows(5).any(|w| w == b"<OFX ")));
        is_v1 || is_v2
    }

    /// Parses `bytes` as an OFX or QFX file and returns the transactions.
    ///
    /// Auto-detects OFX v1 (SGML) vs OFX v2 (XML) based on the file header.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw OFX/QFX file bytes.
    /// * `_config` - Unused; reserved for future configuration options.
    ///
    /// # Returns
    ///
    /// A list of [`RawTransaction`] values parsed from the statement.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::Parse`] if the file cannot be parsed or if an
    /// amount value cannot be represented as an `i64` minor-unit integer.
    #[inline]
    fn import(&self, bytes: &[u8], _config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        let stmt = parse(bytes).map_err(ImportError::Parse)?;

        stmt.transactions
            .into_iter()
            .map(|tx| {
                let amount = decimal_to_amount(tx.amount, &stmt.currency)?;
                let description = tx
                    .memo
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(tx.name.as_deref())
                    .unwrap_or("")
                    .to_owned();
                Ok(RawTransaction::new(
                    tx.date,
                    amount,
                    None,
                    tx.name,
                    description,
                    Some(tx.fitid).filter(|s| !s.is_empty()),
                ))
            })
            .collect()
    }
}

/// Converts a [`Decimal`] value and currency string into a [`bc_sdk::Amount`].
///
/// # Arguments
///
/// * `value` - The decimal value to convert.
/// * `currency` - The ISO 4217 currency code (e.g. `"AUD"`).
///
/// # Returns
///
/// A [`bc_sdk::Amount`] with `minor_units`, `currency`, and `scale` derived
/// from the decimal's mantissa and exponent.
///
/// # Errors
///
/// Returns [`ImportError::Parse`] if the decimal value cannot be represented
/// as an `i64` minor-unit integer.
#[inline]
fn decimal_to_amount(value: Decimal, currency: impl Into<String>) -> Result<Amount, ImportError> {
    let scale = value.scale();
    // mantissa() gives the unscaled integer: 50.00 → mantissa=5000, scale=2 → minor_units=5000
    let minor_units = i64::try_from(value.mantissa())
        .map_err(|_| ImportError::Parse(format!("amount out of i64 range: {value}")))?;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "rust_decimal's max scale is 28, well within u8 range"
    )]
    Ok(Amount::new(minor_units, currency, scale as u8))
}

#[cfg(test)]
mod tests {
    use bc_sdk::Importer as _;
    use bc_sdk::{Amount, Date, ImportConfig};
    use pretty_assertions::assert_eq;

    use super::*;

    const OFX_V1: &[u8] = b"\
OFXHEADER:100\r\nDATA:OFXSGML\r\n\r\n\
<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\
<CURDEF>AUD<BANKACCTFROM><ACCTID>999</BANKACCTFROM>\
<BANKTRANLIST>\
<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20250115<TRNAMT>-50.00<FITID>REF001<NAME>Woolworths<MEMO>Groceries</STMTTRN>\
<STMTTRN><TRNTYPE>CREDIT<DTPOSTED>20250116<TRNAMT>3000.00<FITID>REF002<NAME>Employer</STMTTRN>\
</BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn imports_v1_two_transactions() {
        let txs = OfxImporter::new()
            .import(OFX_V1, ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].date, Date::new(2025, 1, 15));
        assert_eq!(txs[0].reference.as_deref(), Some("REF001"));
        assert_eq!(txs[0].payee.as_deref(), Some("Woolworths"));
        assert_eq!(txs[0].description, "Groceries");
        assert_eq!(txs[1].description, "Employer");
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn payee_falls_back_to_name_when_no_memo() {
        let txs = OfxImporter::new()
            .import(OFX_V1, ImportConfig::default())
            .expect("import");
        // Second transaction has no MEMO, so description = NAME.
        assert_eq!(txs[1].description, "Employer");
    }

    #[test]
    fn detect_recognises_ofx_v1() {
        assert!(OfxImporter::new().detect(b"OFXHEADER:100\nDATA:OFXSGML\n"));
    }

    #[test]
    fn detect_recognises_ofx_v2() {
        assert!(OfxImporter::new().detect(b"<?xml version=\"1.0\"?>\n<?OFX OFXHEADER=\"200\"?>"));
    }

    #[test]
    fn detect_rejects_csv() {
        assert!(!OfxImporter::new().detect(b"Date,Amount\n2025-01-15,-50.00\n"));
    }

    #[test]
    fn empty_fitid_becomes_none_reference() {
        let input = b"\
OFXHEADER:100\r\nDATA:OFXSGML\r\n\r\n\
<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\
<CURDEF>AUD<BANKACCTFROM><ACCTID>999</BANKACCTFROM>\
<BANKTRANLIST>\
<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20250115<TRNAMT>-50.00<NAME>Test</STMTTRN>\
</BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
        let txs = OfxImporter::new()
            .import(input, ImportConfig::default())
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.reference, None);
    }

    #[test]
    fn decimal_to_amount_round_trips_typical_bank_values() {
        use rust_decimal_macros::dec;
        assert_eq!(
            decimal_to_amount(dec!(50.00), "AUD").expect("no overflow"),
            Amount::new(5000, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(-1234.56), "AUD").expect("no overflow"),
            Amount::new(-123_456, "AUD", 2)
        );
        assert_eq!(
            decimal_to_amount(dec!(0.00), "AUD").expect("no overflow"),
            Amount::new(0, "AUD", 2)
        );
    }

    #[test]
    fn decimal_to_amount_overflow_returns_error() {
        use rust_decimal_macros::dec;
        let huge = dec!(99999999999999999999999999.99);
        assert!(
            decimal_to_amount(huge, "AUD").is_err(),
            "expected error for out-of-range mantissa"
        );
    }
}
