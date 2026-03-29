//! [`LedgerImporter`]: implements [`bc_core::Importer`] for Ledger files.

use bc_core::ImportConfig;
use bc_core::ImportError;
use bc_core::Importer;
use bc_core::RawTransaction;
use bc_models::Amount;
use bc_models::CommodityCode;
use rust_decimal::Decimal;

use crate::ast::Entry;
use crate::ast::PostingAmount;
use crate::parser::parse;

/// Implements [`Importer`] for the Ledger plain-text accounting format.
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "the name LedgerImporter is conventional and unambiguous at the crate root"
)]
pub struct LedgerImporter;

impl LedgerImporter {
    /// Creates a new [`LedgerImporter`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for LedgerImporter {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Importer for LedgerImporter {
    #[inline]
    fn name(&self) -> &'static str {
        "ledger"
    }

    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        let Ok(text) = core::str::from_utf8(bytes) else {
            return false;
        };
        // Heuristic: at least one line starts with `YYYY[-/]MM[-/]DD ` — i.e. a
        // date immediately followed by a space (transaction header pattern).
        // This distinguishes Ledger from CSV where dates appear inside fields.
        text.lines().any(|l| {
            let b = l.as_bytes();
            b.get(..4).is_some_and(|s| s.iter().all(u8::is_ascii_digit))
                && b.get(4).is_some_and(|&c| c == b'-' || c == b'/')
                && b.get(5..7)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
                && b.get(7).is_some_and(|&c| c == b'-' || c == b'/')
                && b.get(8..10)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
                && b.get(10).is_some_and(|&c| c == b' ')
        })
    }

    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        _config: &ImportConfig,
    ) -> Result<Vec<RawTransaction>, ImportError> {
        let text = core::str::from_utf8(bytes)
            .map_err(|e| ImportError::Parse(format!("file is not valid UTF-8: {e}")))?;

        let entries = parse(text).map_err(ImportError::Parse)?;

        let mut raw_txs = Vec::new();

        for entry in entries {
            let Entry::Transaction(tx) = entry else {
                continue;
            };

            // Resolve elided amounts so every posting has an explicit value.
            let postings = resolve_elided(&tx.postings).map_err(ImportError::Parse)?;

            // Emit one RawTransaction per Ledger transaction using the first posting's amount.
            if let Some(first) = postings.first() {
                let amount = Amount::new(first.value, CommodityCode::new(first.commodity.as_str()));
                let payee = if tx.payee.is_empty() {
                    None
                } else {
                    Some(tx.payee.clone())
                };
                let description = tx.comment.clone().unwrap_or_else(|| tx.payee.clone());

                raw_txs.push(RawTransaction::new(
                    tx.date,
                    amount,
                    None,
                    payee,
                    description,
                    None,
                ));
            }
        }

        Ok(raw_txs)
    }
}

/// Resolves elided posting amounts.
///
/// Ledger allows the last posting to omit its amount.  The missing amount is
/// computed as the negated sum of all other postings for the same commodity.
///
/// # Errors
///
/// Returns a string error if more than one posting has an elided amount, or if
/// the transaction mixes commodities making resolution ambiguous.
#[expect(
    clippy::arithmetic_side_effects,
    reason = "decimal amounts cannot realistically overflow in financial ledger files"
)]
#[expect(
    clippy::expect_used,
    reason = "the elided_count == 0 branch already verified that all amounts are Some"
)]
fn resolve_elided(postings: &[crate::ast::Posting]) -> Result<Vec<PostingAmount>, String> {
    let elided_count = postings.iter().filter(|p| p.amount.is_none()).count();

    if elided_count > 1 {
        return Err(format!(
            "transaction has {elided_count} postings with elided amounts; at most 1 is allowed"
        ));
    }

    if elided_count == 0 {
        return Ok(postings
            .iter()
            .map(|p| {
                p.amount
                    .clone()
                    .expect("already verified all amounts are present")
            })
            .collect());
    }

    // Sum explicit postings per commodity.
    let mut sums: std::collections::BTreeMap<String, Decimal> = std::collections::BTreeMap::new();
    for p in postings {
        if let Some(amt) = &p.amount {
            *sums.entry(amt.commodity.clone()).or_default() += amt.value;
        }
    }

    if sums.len() > 1 {
        return Err("cannot resolve elided amount in a multi-commodity transaction".into());
    }

    let (commodity, total) = sums
        .into_iter()
        .next()
        .ok_or_else(|| "transaction has only elided postings".to_owned())?;

    Ok(postings
        .iter()
        .map(|p| {
            p.amount.clone().unwrap_or(PostingAmount {
                value: -total,
                commodity: commodity.clone(),
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use bc_core::ImportConfig;
    use bc_core::Importer as _;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;

    use super::*;

    fn empty_config() -> ImportConfig {
        ImportConfig::default()
    }

    #[test]
    #[expect(
        clippy::indexing_slicing,
        reason = "test indices are known to be valid"
    )]
    fn imports_simple_transaction() {
        let input = "2025-01-15 * Woolworths\n    Expenses:Food    50.00 AUD\n    Assets:Bank   -50.00 AUD\n";
        let txs = LedgerImporter
            .import(input.as_bytes(), &empty_config())
            .expect("import");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].payee.as_deref(), Some("Woolworths"));
        assert_eq!(txs[0].date, date(2025, 1, 15));
    }

    #[test]
    fn elided_amount_inferred_for_balance() {
        let input = "2025-01-17 Rent\n    Expenses:Rent    1500.00 AUD\n    Assets:Bank\n";
        let txs = LedgerImporter
            .import(input.as_bytes(), &empty_config())
            .expect("import");
        assert!(!txs.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let input = "; comment\n\n2025-01-15 * A\n    X    1.00 AUD\n    Y   -1.00 AUD\n";
        let txs = LedgerImporter
            .import(input.as_bytes(), &empty_config())
            .expect("import");
        assert_eq!(txs.len(), 1);
    }

    #[test]
    fn detect_recognises_ledger_syntax() {
        let bytes = b"2025-01-15 * Payee\n    Assets:Bank    50.00 AUD\n";
        assert!(LedgerImporter.detect(bytes));
    }

    #[test]
    fn detect_rejects_csv() {
        let bytes = b"Date,Amount\n2025-01-15,-50.00\n";
        assert!(!LedgerImporter.detect(bytes));
    }
}
