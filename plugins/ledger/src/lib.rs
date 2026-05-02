//! BorrowChecker plugin for the Ledger plain-text accounting format.
//!
//! This crate implements [`bc_sdk::Importer`] for Ledger files and is compiled
//! to a WASM component for use with the BorrowChecker plugin host.

use std::collections::BTreeMap;

use bc_sdk::Amount;
use bc_sdk::ImportConfig;
use bc_sdk::ImportError;
use bc_sdk::RawTransaction;
use rust_decimal::Decimal;

mod ast;
mod parser;

use ast::Entry;
use ast::PostingAmount;
use parser::parse;

/// Implements [`bc_sdk::Importer`] for the Ledger plain-text accounting format.
#[non_exhaustive]
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

#[bc_sdk::importer]
impl bc_sdk::Importer for LedgerImporter {
    #[inline]
    fn name(&self) -> &str {
        "ledger"
    }

    #[inline]
    fn detect(&self, bytes: &[u8]) -> bool {
        let Ok(text) = core::str::from_utf8(bytes) else {
            return false;
        };
        // Heuristic: at least one line matches `YYYY[-/]MM[-/]DD ` and does NOT
        // look like a Beancount transaction header.
        //
        // Beancount uses `YYYY-MM-DD * "Payee" "Narration"` — quoted strings
        // immediately follow the `*` or `!` flag.  Ledger uses unquoted payees:
        // `YYYY-MM-DD * Payee`.  We exclude lines where the first non-space
        // character after the date+flag is a double-quote.
        text.lines().any(|l| {
            let b = l.as_bytes();
            let is_date_line = b.get(..4).is_some_and(|s| s.iter().all(u8::is_ascii_digit))
                && b.get(4).is_some_and(|&c| c == b'-' || c == b'/')
                && b.get(5..7)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
                && b.get(7).is_some_and(|&c| c == b'-' || c == b'/')
                && b.get(8..10)
                    .is_some_and(|s| s.iter().all(u8::is_ascii_digit))
                && b.get(10).is_some_and(|&c| c == b' ');

            if !is_date_line {
                return false;
            }

            // Exclude Beancount: skip optional flag char + spaces, then reject
            // if the payee/narration starts with a quote.
            let rest = b.get(11..).unwrap_or(&[]);
            let after_flag = if rest.first().is_some_and(|&c| c == b'*' || c == b'!') {
                rest.get(1..).unwrap_or(&[]).trim_ascii_start()
            } else {
                rest.trim_ascii_start()
            };
            !after_flag.starts_with(b"\"")
        })
    }

    #[inline]
    fn import(
        &self,
        bytes: &[u8],
        _config: ImportConfig,
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
                let amount = decimal_to_amount(first.value, first.commodity.as_str())?;
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

/// Converts a [`rust_decimal::Decimal`] and currency string into a [`bc_sdk::Amount`].
///
/// The minor units are derived from the decimal's mantissa (already in minor-unit form).
///
/// # Arguments
///
/// * `value` - The decimal value to convert.
/// * `currency` - The ISO 4217 currency code, e.g. `"AUD"`.
///
/// # Returns
///
/// A [`bc_sdk::Amount`] with `minor_units`, `currency`, and `scale` set.
///
/// # Errors
///
/// Returns [`ImportError::Parse`] if the decimal mantissa does not fit in an
/// `i64` (i.e. the value is too large to represent as minor units).
#[inline]
fn decimal_to_amount(
    value: Decimal,
    currency: impl Into<String>,
) -> Result<Amount, ImportError> {
    // Decimal::mantissa() is already the unscaled integer (minor units).
    // For 50.00: mantissa=5000, scale=2 → minor_units=5000 (correct: 50.00 AUD = 5000 cents)
    let minor_units = i64::try_from(value.mantissa()).map_err(|_| {
        ImportError::Parse(format!(
            "amount mantissa overflows i64: {value} is too large to represent"
        ))
    })?;
    // rust_decimal caps scale at 28, well within u8::MAX (255); try_from makes
    // this invariant explicit and returns an error if it ever breaks.
    let scale = u8::try_from(value.scale()).map_err(|_| {
        ImportError::Parse(format!(
            "decimal scale {} exceeds u8 maximum ({})",
            value.scale(),
            u8::MAX
        ))
    })?;
    Ok(Amount::new(minor_units, currency, scale))
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
#[inline]
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
    let mut sums: BTreeMap<String, Decimal> = BTreeMap::new();
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
    use bc_sdk::Importer as _;
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
            .import(input.as_bytes(), empty_config())
            .expect("import");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].payee.as_deref(), Some("Woolworths"));
        assert_eq!(txs[0].date, bc_sdk::Date::new(2025_i32, 1_u8, 15_u8));
    }

    #[test]
    fn elided_amount_inferred_for_balance() {
        let input = "2025-01-17 Rent\n    Expenses:Rent    1500.00 AUD\n    Assets:Bank\n";
        let txs = LedgerImporter
            .import(input.as_bytes(), empty_config())
            .expect("import");
        assert!(!txs.is_empty());
    }

    #[test]
    fn comments_and_blank_lines_ignored() {
        let input = "; comment\n\n2025-01-15 * A\n    X    1.00 AUD\n    Y   -1.00 AUD\n";
        let txs = LedgerImporter
            .import(input.as_bytes(), empty_config())
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

    #[test]
    fn detect_rejects_beancount_syntax() {
        // Beancount uses quoted payees/narrations; Ledger does not.
        let bytes =
            b"2025-01-15 * \"Woolworths\" \"Weekly groceries\"\n  Expenses:Food   50.00 AUD\n";
        assert!(!LedgerImporter.detect(bytes));
    }
}
