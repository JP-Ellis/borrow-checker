//! [`Exporter`]: implements [`bc_core::Exporter`] for Beancount files.

use bc_core::ExportData;
use bc_core::ExportError;
use jiff::civil::date;

use crate::writer::render_commodity;
use crate::writer::render_open;
use crate::writer::render_transaction;

/// Implements [`bc_core::Exporter`] for the Beancount plain-text accounting format.
///
/// Produces a well-formed Beancount file containing `commodity` directives,
/// `open` directives for all accounts, and all non-voided transactions with
/// their postings rendered using the colon-separated account path.
#[non_exhaustive]
pub struct Exporter;

impl Exporter {
    /// Creates a new [`Exporter`].
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for Exporter {
    /// Returns a default [`Exporter`].
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl bc_core::Exporter for Exporter {
    /// Returns the stable identifier for this exporter.
    #[inline]
    fn name(&self) -> &'static str {
        "beancount"
    }

    /// Serialises `data` into Beancount format.
    ///
    /// The output contains:
    /// 1. `commodity` directives for all known commodities.
    /// 2. `open` directives for all accounts (using a fixed epoch date of 1970-01-01).
    /// 3. Transaction directives for all non-voided transactions, with one posting per leg.
    ///
    /// # Arguments
    ///
    /// * `data` - A snapshot of the domain data to export.
    ///
    /// # Returns
    ///
    /// The Beancount file contents as UTF-8 bytes.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError::AccountNotFound`] if a posting references an account
    /// that is not present in `data.accounts`.
    #[inline]
    fn export(&self, data: &ExportData<'_>) -> Result<Vec<u8>, ExportError> {
        let mut out = String::new();

        // Commodity directives
        let epoch = date(1970, 1, 1);
        for commodity in data.commodities {
            out.push_str(&render_commodity(epoch, commodity.code()));
        }
        if !data.commodities.is_empty() {
            out.push('\n');
        }

        // Open directives
        for account in data.accounts {
            let path = data.account_path(account);
            out.push_str(&render_open(epoch, &path, None));
        }
        if !data.accounts.is_empty() {
            out.push('\n');
        }

        // Transaction directives (sorted by date)
        let mut sorted_txs: Vec<_> = data.transactions.iter().collect();
        sorted_txs.sort_by_key(|tx| tx.date());

        for tx in sorted_txs {
            let mut postings = Vec::new();
            for posting in tx.postings() {
                let account = data.account_by_id(posting.account_id()).ok_or_else(|| {
                    ExportError::AccountNotFound(posting.account_id().to_string())
                })?;
                let path = data.account_path(account);
                postings.push((path, posting.amount().clone()));
            }

            let posting_refs: Vec<(&str, bc_models::Amount)> = postings
                .iter()
                .map(|(p, a)| (p.as_str(), a.clone()))
                .collect();

            let rendered = render_transaction(
                tx.date(),
                tx.status(),
                tx.payee(),
                tx.description(),
                &posting_refs,
            );

            if !rendered.is_empty() {
                out.push_str(&rendered);
                out.push('\n');
            }
        }

        Ok(out.into_bytes())
    }
}

#[cfg(test)]
mod tests {
    use bc_core::ExportData;
    use bc_core::Exporter as _;
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn export_empty_data_produces_empty_output() {
        let data = ExportData::new(&[], &[], &[], &[]);
        let bytes = Exporter.export(&data).expect("export");
        assert_eq!(bytes, b"");
    }

    #[test]
    fn exporter_name_is_beancount() {
        assert_eq!(Exporter.name(), "beancount");
    }
}
