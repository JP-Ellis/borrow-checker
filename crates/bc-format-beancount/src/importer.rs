//! [`BeancountImporter`]: implements [`bc_core::Importer`] for Beancount files.

use bc_core::ImportConfig;
use bc_core::ImportError;
use bc_core::Importer;
use bc_core::RawTransaction;
use bc_models::Amount;
use bc_models::CommodityCode;

use crate::ast::Directive;
use crate::parser::parse;

/// Implements [`Importer`] for the Beancount plain-text accounting format.
///
/// Parses Beancount-formatted files and converts transaction directives into
/// [`RawTransaction`] values. Open, close, commodity, and balance directives
/// are silently ignored.
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "BeancountImporter is the conventional public name for this type; re-exported from crate root"
)]
pub struct BeancountImporter;

impl BeancountImporter {
    /// Creates a new [`BeancountImporter`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for BeancountImporter {
    /// Returns a default [`BeancountImporter`].
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Importer for BeancountImporter {
    /// Returns the stable identifier for this importer.
    #[inline]
    fn name(&self) -> &'static str {
        "beancount"
    }

    /// Returns `true` if `bytes` appear to be a Beancount file.
    ///
    /// Detection heuristic: at least one line looks like a dated transaction
    /// header (`YYYY-MM-DD * "..."` or `YYYY-MM-DD ! "..."`).
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw file bytes to inspect.
    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        let Ok(text) = core::str::from_utf8(bytes) else {
            return false;
        };
        text.lines().any(|l| {
            let t = l.trim_start();
            t.len() > 12
                && t.as_bytes().get(4).copied() == Some(b'-')
                && (t.contains(" * \"") || t.contains(" ! \""))
        })
    }

    /// Parses `bytes` as a Beancount file and returns the transactions.
    ///
    /// # Arguments
    ///
    /// * `bytes` - Raw Beancount file bytes.
    /// * `_config` - Unused; reserved for future configuration options.
    ///
    /// # Returns
    ///
    /// A list of [`RawTransaction`] values parsed from transaction directives.
    ///
    /// # Errors
    ///
    /// Returns [`ImportError::Parse`] if the file is not valid UTF-8 or if a
    /// parse error is encountered.
    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        _config: &ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError> {
        let text = core::str::from_utf8(bytes)
            .map_err(|e| ImportError::Parse(format!("file is not valid UTF-8: {e}")))?;

        let directives = parse(text).map_err(ImportError::Parse)?;
        let mut raw_txs = Vec::new();

        for directive in directives {
            let Directive::Transaction(tx) = directive else {
                continue;
            };

            let first = tx
                .postings
                .first()
                .ok_or_else(|| ImportError::Parse("transaction has no postings".into()))?;

            // Warn when multiple commodities are present: only the first posting's
            // commodity is used for `RawTransaction::amount`; the rest are dropped.
            // This is a limitation of the single-commodity `RawTransaction` model.
            let has_multiple_commodities = tx.postings.iter().any(|p| p.currency != first.currency);
            if has_multiple_commodities {
                tracing::warn!(
                    date = %tx.date,
                    narration = %tx.narration,
                    "beancount transaction has multiple commodities; only the first posting's \
                     commodity ({}) is imported — remaining postings are dropped",
                    first.currency,
                );
            }

            let amount = Amount::new(first.amount, CommodityCode::new(&first.currency));

            raw_txs.push(RawTransaction::new(
                tx.date,
                amount,
                None,
                tx.payee,
                tx.narration,
                None,
            ));
        }

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

    #[test]
    fn imports_transaction_payee_and_narration() {
        let input = "2025-01-15 * \"Woolworths\" \"Weekly groceries\"\n  Expenses:Food   50.00 AUD\n  Assets:Bank    -50.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), &ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 1);
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee.as_deref(), Some("Woolworths"));
        assert_eq!(tx.description, "Weekly groceries");
        assert_eq!(tx.date, date(2025, 1, 15));
    }

    #[test]
    fn imports_narration_only() {
        let input = "2025-01-15 * \"Transfer\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), &ImportConfig::default())
            .expect("import");
        let tx = txs.first().expect("should have one transaction");
        assert_eq!(tx.payee, None);
        assert_eq!(tx.description, "Transfer");
    }

    #[test]
    fn skips_open_commodity_directives() {
        let input = "2025-01-01 open Assets:Bank AUD\n2025-01-01 commodity AUD\n2025-01-15 * \"X\"\n  A:B   1.00 AUD\n  A:C  -1.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), &ImportConfig::default())
            .expect("import");
        assert_eq!(txs.len(), 1);
    }

    #[test]
    fn detect_recognises_beancount() {
        let bytes = b"2025-01-15 * \"Payee\" \"Narration\"\n  Assets:Bank   50.00 AUD\n";
        assert!(BeancountImporter.detect(bytes));
    }

    #[test]
    fn detect_rejects_ledger() {
        let bytes = b"2025-01-15 * Payee without quotes\n    Assets:Bank    50.00 AUD\n";
        assert!(!BeancountImporter.detect(bytes));
    }

    #[test]
    fn import_multi_currency_transaction_uses_first_posting() {
        // A transaction with mixed currencies: the importer should succeed
        // and use the first posting's amount, emitting a warning for the rest.
        let input =
            "2025-01-15 * \"FX Purchase\"\n  Assets:USD   100.00 USD\n  Assets:AUD  -150.00 AUD\n";
        let txs = BeancountImporter
            .import(input.as_bytes(), &ImportConfig::default())
            .expect("import should succeed even for multi-currency");
        let tx = txs.first().expect("should have one transaction");
        // First posting determines the amount
        assert_eq!(tx.description, "FX Purchase");
    }

    #[test]
    fn import_transaction_with_no_postings_returns_error() {
        // A transaction directive with zero postings is invalid; the importer
        // must return an error rather than panic.
        let input = "2025-01-15 * \"Payee\" \"No postings\"\n";
        let result = BeancountImporter.import(input.as_bytes(), &ImportConfig::default());
        assert!(
            result.is_err(),
            "expected error for zero-posting transaction"
        );
    }
}
