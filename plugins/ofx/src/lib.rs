//! BorrowChecker plugin for the OFX/QFX open financial exchange format.
//!
//! Implements [`bc_sdk::Importer`] for OFX v1 (SGML) and OFX v2 (XML) files.

mod ast;
mod config;
mod parser;
mod sgml;

use bc_sdk::Amount;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::RawPosting;
use bc_sdk::RawTransaction;
use bc_sdk::SourceLocation;
use rust_decimal::Decimal;

use crate::config::Config;
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

    /// Parses `cfg.source_file` as an OFX or QFX file and returns the transactions.
    ///
    /// Auto-detects OFX v1 (SGML) vs OFX v2 (XML) based on the file header.
    ///
    /// # Arguments
    ///
    /// * `config` - Importer configuration; must supply `account` and `source_file`.
    ///
    /// # Returns
    ///
    /// A list of [`RawTransaction`] values parsed from the statement.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::BadValue`] if `source_file` cannot be read, or
    /// [`ImportError::Parse`] if the file cannot be parsed or if an amount
    /// value cannot be represented as an `i64` minor-unit integer.
    #[inline]
    fn import(&self, config: ImportConfig) -> Result<Vec<RawTransaction>, ImportError> {
        let cfg: Config = config.as_typed()?;
        let bytes = std::fs::read(&cfg.source_file).map_err(|e| ImportError::BadValue {
            field: "source_file".to_owned(),
            detail: format!("cannot read {:?}: {e}", cfg.source_file),
        })?;
        let stmt = parse(&bytes).map_err(ImportError::Parse)?;
        let file = &cfg.source_file;

        stmt.transactions
            .into_iter()
            .enumerate()
            .map(|(index, tx)| {
                let index = index.saturating_add(1);
                let amount = decimal_to_amount(tx.amount, &stmt.currency)?;
                let description = tx
                    .memo
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(tx.name.as_deref())
                    .unwrap_or("")
                    .to_owned();
                let reference = Some(tx.fitid).filter(|s| !s.is_empty());
                Ok(RawTransaction::builder()
                    .date(tx.date)
                    .maybe_payee(tx.name)
                    .description(description)
                    .maybe_reference(reference)
                    .source_location(
                        SourceLocation::builder()
                            .display(format!("{file} transaction {index}"))
                            .build(),
                    )
                    .postings(vec![
                        RawPosting::builder()
                            .account(cfg.account.clone())
                            .amount(amount)
                            .build(),
                    ])
                    .build())
            })
            .collect()
    }

    /// Accepts every configuration without checking it.
    ///
    /// Config validation rules for the OFX importer are not implemented yet;
    /// see [issue #361](https://github.com/JP-Ellis/borrow-checker/issues/361).
    #[inline]
    fn validate(&self, _config: ImportConfig) -> Result<(), ImportError> {
        Ok(())
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
    use std::io::Write as _;

    use bc_sdk::Amount;
    use bc_sdk::Date;
    use bc_sdk::ImportConfig;
    use bc_sdk::Importer as _;
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

    /// Writes `bytes` to a fresh file named `name` inside a fresh temp
    /// directory unique to `test_name`, returning its absolute path.
    fn write_temp_file(test_name: &str, name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("bc-ofx-{test_name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).expect("create");
        f.write_all(bytes).expect("write");
        path.to_str().expect("utf8 path").to_owned()
    }

    /// Returns a test [`ImportConfig`] with `account` set to `"Assets:Bank:Checking"`
    /// and `source_file` pointing at a temp file containing `bytes`.
    fn test_config(test_name: &str, bytes: &[u8]) -> ImportConfig {
        let source_file = write_temp_file(test_name, "statement.ofx", bytes);
        ImportConfig::from_json_string(
            serde_json::json!({
                "account": "Assets:Bank:Checking",
                "source_file": source_file,
            })
            .to_string(),
        )
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn imports_v1_two_transactions() {
        let txs = OfxImporter::new()
            .import(test_config("imports_v1_two_transactions", OFX_V1))
            .expect("import");
        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].date, Date::new(2025, 1, 15));
        assert_eq!(txs[0].reference.as_deref(), Some("REF001"));
        assert_eq!(txs[0].payee.as_deref(), Some("Woolworths"));
        assert_eq!(txs[0].description, "Groceries");
        assert_eq!(txs[0].postings.len(), 1);
        assert_eq!(txs[0].postings[0].account, "Assets:Bank:Checking");
        assert_eq!(
            txs[0].postings[0].amount,
            Some(Amount::new(-5000, "AUD", 2))
        );
        assert_eq!(txs[1].description, "Employer");
        assert_eq!(txs[1].postings.len(), 1);
        assert_eq!(txs[1].postings[0].account, "Assets:Bank:Checking");
        assert_eq!(
            txs[1].postings[0].amount,
            Some(Amount::new(300_000, "AUD", 2))
        );
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn payee_falls_back_to_name_when_no_memo() {
        let txs = OfxImporter::new()
            .import(test_config("payee_falls_back_to_name_when_no_memo", OFX_V1))
            .expect("import");
        // Second transaction has no MEMO, so description = NAME.
        assert_eq!(txs[1].description, "Employer");
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test code: panicking on wrong index is the desired behaviour"
    )]
    fn source_location_names_file_and_transaction_ordinal() {
        let source_file = write_temp_file(
            "source_location_names_file_and_transaction_ordinal",
            "statement.ofx",
            OFX_V1,
        );
        let config = ImportConfig::from_json_string(
            serde_json::json!({
                "account": "Assets:Bank:Checking",
                "source_file": source_file,
            })
            .to_string(),
        );
        let txs = OfxImporter::new().import(config).expect("import");
        assert_eq!(txs.len(), 2);

        let location0 = txs[0]
            .source_location
            .as_ref()
            .expect("source location should be set");
        assert_eq!(location0.display, format!("{source_file} transaction 1"));

        let location1 = txs[1]
            .source_location
            .as_ref()
            .expect("source location should be set");
        assert_eq!(location1.display, format!("{source_file} transaction 2"));
    }

    #[test]
    fn empty_fitid_becomes_none_reference() {
        let input: &[u8] = b"\
OFXHEADER:100\r\nDATA:OFXSGML\r\n\r\n\
<OFX><BANKMSGSRSV1><STMTTRNRS><STMTRS>\
<CURDEF>AUD<BANKACCTFROM><ACCTID>999</BANKACCTFROM>\
<BANKTRANLIST>\
<STMTTRN><TRNTYPE>DEBIT<DTPOSTED>20250115<TRNAMT>-50.00<NAME>Test</STMTTRN>\
</BANKTRANLIST></STMTRS></STMTTRNRS></BANKMSGSRSV1></OFX>";
        let txs = OfxImporter::new()
            .import(test_config("empty_fitid_becomes_none_reference", input))
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.reference, None);
        assert_eq!(tx.postings.len(), 1);
        assert_eq!(tx.postings[0].account, "Assets:Bank:Checking");
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
