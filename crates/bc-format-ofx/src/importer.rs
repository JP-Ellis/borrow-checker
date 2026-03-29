//! [`OfxImporter`]: implements [`bc_core::Importer`] for OFX/QFX files.

use bc_core::ImportConfig;
use bc_core::ImportError;
use bc_core::Importer;
use bc_core::RawTransaction;
use bc_models::Amount;
use bc_models::CommodityCode;

use crate::parser::parse;

/// Implements [`Importer`] for OFX v1 (SGML) and OFX v2 (XML) files.
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "OfxImporter lives in the importer module; the name repetition is intentional for clarity at the call site"
)]
pub struct OfxImporter;

impl OfxImporter {
    /// Creates a new [`OfxImporter`].
    ///
    /// # Returns
    ///
    /// A new [`OfxImporter`] instance.
    #[inline]
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for OfxImporter {
    /// Returns a default [`OfxImporter`].
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Importer for OfxImporter {
    #[inline]
    fn name(&self) -> &'static str {
        "ofx"
    }

    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        let is_v1 = bytes.windows(11).any(|w| w == b"OFXHEADER:1");
        let is_v2 = bytes.windows(15).any(|w| w == b"OFXHEADER=\"200\"")
            || (bytes.starts_with(b"<?xml") && bytes.windows(4).any(|w| w == b"<OFX"));
        is_v1 || is_v2
    }

    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        _config: &ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError> {
        let stmt = parse(bytes).map_err(ImportError::Parse)?;

        let raw_txs = stmt
            .transactions
            .into_iter()
            .map(|tx| {
                let amount = Amount::new(tx.amount, CommodityCode::new(&stmt.currency));
                let description = tx
                    .memo
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(tx.name.as_deref())
                    .unwrap_or("")
                    .to_owned();
                RawTransaction::new(
                    tx.date,
                    amount,
                    None,
                    tx.name,
                    description,
                    Some(tx.fitid).filter(|s| !s.is_empty()),
                )
            })
            .collect();

        Ok(raw_txs)
    }
}

#[cfg(test)]
mod tests {
    use bc_core::ImportConfig;
    use bc_core::Importer as _;
    use jiff::civil::date;
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
            .import(OFX_V1, &ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].date, date(2025, 1, 15));
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
            .import(OFX_V1, &ImportConfig::default())
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
}
