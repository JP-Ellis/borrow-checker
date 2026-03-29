//! [`Exporter`]: implements [`bc_core::Exporter`] for Ledger files.

use bc_core::ExportData;
use bc_core::ExportError;

use crate::writer::render_account_decl;
use crate::writer::render_commodity_decl;
use crate::writer::render_transaction;

/// Implements [`bc_core::Exporter`] for the Ledger plain-text accounting format.
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
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl bc_core::Exporter for Exporter {
    #[inline]
    fn name(&self) -> &'static str {
        "ledger"
    }

    #[inline]
    fn export(&self, data: &ExportData<'_>) -> Result<Vec<u8>, ExportError> {
        let mut out = String::new();

        // Commodity declarations
        for commodity in data.commodities {
            out.push_str(&render_commodity_decl(commodity.code()));
        }
        if !data.commodities.is_empty() {
            out.push('\n');
        }

        // Account declarations
        for account in data.accounts {
            let path = data.account_path(account);
            out.push_str(&render_account_decl(&path));
        }
        if !data.accounts.is_empty() {
            out.push('\n');
        }

        // Transactions sorted by date
        let mut sorted_txs: Vec<_> = data.transactions.iter().collect();
        sorted_txs.sort_by_key(|tx| tx.date());

        for tx in sorted_txs {
            let posting_data: Vec<(String, bc_models::Amount)> = tx
                .postings()
                .iter()
                .map(|p| {
                    let path = data
                        .account_by_id(p.account_id())
                        .map_or_else(|| p.account_id().to_string(), |a| data.account_path(a));
                    (path, p.amount().clone())
                })
                .collect();

            let posting_refs: Vec<(&str, bc_models::Amount)> = posting_data
                .iter()
                .map(|(s, a)| (s.as_str(), a.clone()))
                .collect();

            let payee = tx.payee().unwrap_or(tx.description());
            let comment = (tx.description() != payee && !tx.description().is_empty())
                .then(|| tx.description());

            let rendered =
                render_transaction(tx.date(), tx.status(), payee, comment, &posting_refs);
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
    fn exports_empty_data_produces_empty_output() {
        let data = ExportData::new(&[], &[], &[], &[]);
        let bytes = Exporter.export(&data).expect("export");
        assert_eq!(bytes, b"");
    }
}
