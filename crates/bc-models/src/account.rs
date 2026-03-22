//! Account domain types.

use jiff::Timestamp;

use crate::{ids::AccountId, money::CommodityCode};

/// The classification of an account in the chart of accounts.
#[expect(
    clippy::module_name_repetitions,
    reason = "AccountType is the canonical domain name regardless of module path"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    /// Money you own (bank accounts, cash, investments).
    Asset,
    /// Money you owe (credit cards, loans).
    Liability,
    /// Net worth (assets minus liabilities).
    Equity,
    /// Money coming in.
    Income,
    /// Money going out.
    Expense,
}

/// A financial account in the chart of accounts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Account {
    /// Unique identifier.
    pub id: AccountId,
    /// Human-readable name.
    pub name: String,
    /// Account classification.
    pub account_type: AccountType,
    /// Default commodity denomination (e.g. `"AUD"`).
    pub commodity: CommodityCode,
    /// Optional description.
    pub description: Option<String>,
    /// When the account was created.
    pub created_at: Timestamp,
    /// When the account was archived, if ever.
    pub archived_at: Option<Timestamp>,
}

impl Account {
    /// Creates a new [`Account`] with all fields.
    ///
    /// This constructor is required because the struct is `#[non_exhaustive]`.
    #[inline]
    #[must_use]
    pub fn new(
        id: AccountId,
        name: String,
        account_type: AccountType,
        commodity: CommodityCode,
        description: Option<String>,
        created_at: Timestamp,
        archived_at: Option<Timestamp>,
    ) -> Self {
        Self {
            id,
            name,
            account_type,
            commodity,
            description,
            created_at,
            archived_at,
        }
    }

    /// Returns `true` if the account has not been archived.
    #[inline]
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.archived_at.is_none()
    }
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;

    use super::*;
    use crate::ids::AccountId;

    #[test]
    fn account_type_variants_exist() {
        _ = (
            AccountType::Asset,
            AccountType::Liability,
            AccountType::Equity,
            AccountType::Income,
            AccountType::Expense,
        );
    }

    #[test]
    fn account_is_active_when_not_archived() {
        let acct = Account {
            id: AccountId::new(),
            name: "Checking".to_owned(),
            account_type: AccountType::Asset,
            commodity: crate::money::CommodityCode::new("AUD"),
            description: None,
            created_at: Timestamp::now(),
            archived_at: None,
        };
        assert!(acct.is_active());
    }
}
