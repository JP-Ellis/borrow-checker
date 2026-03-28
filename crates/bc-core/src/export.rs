//! Export types and the [`Exporter`] trait.
//!
//! This module defines the data snapshot passed to each exporter ([`ExportData`]),
//! the error type produced during export ([`ExportError`]), and the
//! [`Exporter`] trait that format-specific crates implement.

use std::collections::HashSet;

/// A snapshot of the domain data made available to an [`Exporter`].
///
/// All slices are borrowed from the calling context for the duration of the
/// export operation; no data is copied.
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "exported at the crate root as ExportData; the module-prefixed name is intentional for API clarity"
)]
pub struct ExportData<'a> {
    /// Accounts in the chart of accounts.
    pub accounts: &'a [bc_models::Account],
    /// Known commodities (currencies, securities, etc.).
    pub commodities: &'a [bc_models::Commodity],
    /// All transactions to export.
    pub transactions: &'a [bc_models::Transaction],
    /// Tags available for cross-cutting labels.
    pub tags: &'a [bc_models::Tag],
}

impl<'a> ExportData<'a> {
    /// Constructs a new [`ExportData`] snapshot.
    ///
    /// # Arguments
    ///
    /// * `accounts` - Slice of all accounts.
    /// * `commodities` - Slice of all commodities.
    /// * `transactions` - Slice of all transactions.
    /// * `tags` - Slice of all tags.
    ///
    /// # Returns
    ///
    /// A new [`ExportData`] referencing the provided slices.
    #[inline]
    #[must_use]
    pub fn new(
        accounts: &'a [bc_models::Account],
        commodities: &'a [bc_models::Commodity],
        transactions: &'a [bc_models::Transaction],
        tags: &'a [bc_models::Tag],
    ) -> Self {
        Self {
            accounts,
            commodities,
            transactions,
            tags,
        }
    }

    /// Finds an account by its ID using a linear scan.
    ///
    /// # Arguments
    ///
    /// * `id` - The [`bc_models::AccountId`] to search for.
    ///
    /// # Returns
    ///
    /// `Some(&Account)` if found, `None` otherwise.
    #[inline]
    #[must_use]
    pub fn account_by_id(&self, id: &bc_models::AccountId) -> Option<&bc_models::Account> {
        self.accounts.iter().find(|a| a.id() == id)
    }

    /// Builds the full colon-separated path for an account by walking its
    /// `parent_id` chain up to the root.
    ///
    /// For example, given the hierarchy `Assets → CommBank → Savings`,
    /// calling this method on the `Savings` account returns
    /// `"Assets:CommBank:Savings"`.
    ///
    /// The method guards against cycles in the parent chain: if an ID is
    /// encountered a second time the walk stops immediately.
    ///
    /// # Arguments
    ///
    /// * `account` - The account whose path to compute.
    ///
    /// # Returns
    ///
    /// The colon-separated path string, root first.
    #[inline]
    #[must_use]
    pub fn account_path(&self, account: &bc_models::Account) -> String {
        let mut segments: Vec<&str> = Vec::new();
        let mut seen: HashSet<&bc_models::AccountId> = HashSet::new();

        let mut current = account;
        loop {
            if !seen.insert(current.id()) {
                // Cycle detected — stop walking.
                break;
            }
            segments.push(current.name());
            match current.parent_id() {
                None => break,
                Some(parent_id) => match self.account_by_id(parent_id) {
                    None => break,
                    Some(parent) => current = parent,
                },
            }
        }

        segments.reverse();
        segments.join(":")
    }
}

/// Errors produced during an export operation.
#[non_exhaustive]
#[derive(Debug, thiserror::Error)]
#[expect(
    clippy::module_name_repetitions,
    reason = "exported at the crate root as ExportError; the module-prefixed name is intentional for API clarity"
)]
pub enum ExportError {
    /// An account referenced during export could not be found.
    #[error("account not found: {0}")]
    AccountNotFound(String),
    /// A commodity referenced during export could not be found.
    #[error("commodity not found: {0}")]
    CommodityNotFound(String),
    /// A write error occurred while producing the output.
    #[error("write error: {0}")]
    Write(String),
}

/// An object-safe trait implemented by every format-specific exporter.
///
/// Implementors are expected to be `Send + Sync + 'static` so they can be
/// stored in `Arc<dyn Exporter>` and used across async tasks.
///
/// # Example
///
/// ```rust,ignore
/// struct LedgerExporter;
///
/// impl bc_core::Exporter for LedgerExporter {
///     fn name(&self) -> &str { "ledger" }
///
///     fn export(
///         &self,
///         data: &bc_core::ExportData<'_>,
///     ) -> Result<Vec<u8>, bc_core::ExportError> {
///         todo!()
///     }
/// }
/// ```
pub trait Exporter: Send + Sync + 'static {
    /// A short, stable identifier for this exporter (e.g. `"ledger"`, `"beancount"`).
    fn name(&self) -> &str;

    /// Serialises `data` into the exporter's format.
    ///
    /// # Arguments
    ///
    /// * `data` - A snapshot of the domain data to export.
    ///
    /// # Returns
    ///
    /// The serialised bytes of the export output.
    ///
    /// # Errors
    ///
    /// Returns [`ExportError`] if an account or commodity is missing, or if a
    /// write error occurs while producing the output.
    fn export(&self, data: &ExportData<'_>) -> Result<Vec<u8>, ExportError>;
}

#[cfg(test)]
mod tests {
    use bc_models::AccountType;

    use super::*;

    /// Builds a minimal root account with no parent.
    fn make_root_account(name: &str, account_type: AccountType) -> bc_models::Account {
        bc_models::Account::builder()
            .name(name)
            .account_type(account_type)
            .build()
    }

    /// Builds a child account whose parent is `parent`.
    fn make_child_account(
        name: &str,
        account_type: AccountType,
        parent: &bc_models::Account,
    ) -> bc_models::Account {
        bc_models::Account::builder()
            .name(name)
            .account_type(account_type)
            .parent_id(parent.id().clone())
            .build()
    }

    #[test]
    fn account_path_for_root_account_returns_just_its_name() {
        let assets = make_root_account("Assets", AccountType::Asset);
        let accounts = [assets.clone()];
        let data = ExportData::new(&accounts, &[], &[], &[]);

        let path = data.account_path(&assets);

        pretty_assertions::assert_eq!(path, "Assets");
    }

    #[test]
    fn account_path_for_three_level_hierarchy_returns_colon_path() {
        let assets = make_root_account("Assets", AccountType::Asset);
        let commbank = make_child_account("CommBank", AccountType::Asset, &assets);
        let savings = make_child_account("Savings", AccountType::Asset, &commbank);

        let accounts = [assets, commbank, savings.clone()];
        let data = ExportData::new(&accounts, &[], &[], &[]);

        let path = data.account_path(&savings);

        pretty_assertions::assert_eq!(path, "Assets:CommBank:Savings");
    }

    #[test]
    fn account_by_id_finds_existing_account() {
        let assets = make_root_account("Assets", AccountType::Asset);
        let id = assets.id().clone();
        let accounts = [assets];
        let data = ExportData::new(&accounts, &[], &[], &[]);

        assert!(data.account_by_id(&id).is_some());
    }

    #[test]
    fn account_by_id_returns_none_for_unknown_id() {
        let data = ExportData::new(&[], &[], &[], &[]);
        let unknown_id = bc_models::AccountId::new();

        assert!(data.account_by_id(&unknown_id).is_none());
    }
}
