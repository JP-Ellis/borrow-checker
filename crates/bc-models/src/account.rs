//! Account domain types.

use core::fmt;
use core::str::FromStr;

use jiff::Timestamp;
use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

use crate::CommodityId;
use crate::TagId;

crate::define_id!(AccountId, "account");

/// The classification of an account in the chart of accounts.
///
/// Re-exported from the crate root as [`crate::AccountType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Type {
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

/// Error returned when an account field fails validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ValidationError {
    /// The name must be non-empty.
    #[error("account name must not be empty")]
    EmptyName,
}

/// A financial account in the chart of accounts.
///
/// # Example
///
/// ```
/// use bc_models::{Account, AccountType};
/// use jiff::Timestamp;
///
/// let account = Account::builder()
///     .name("Checking")
///     .account_type(AccountType::Asset)
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(account.name(), "Checking");
/// assert!(account.is_active());
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Account {
    /// Stable, opaque identifier for this account (a prefixed UUIDv7). Defaults to a
    /// freshly generated value; only supply this when re-hydrating a record from storage.
    #[builder(default)]
    id: AccountId,

    /// Display name shown in reports and the user interface. Must not be empty —
    /// [`Account::set_name`] enforces this on mutation, but the builder does not
    /// validate at construction time.
    #[builder(into)]
    name: String,

    /// Classification in the chart of accounts. Determines how the account
    /// contributes to the balance sheet and P&L statements.
    ///
    /// A sub-account (one with a `parent_id`) must share the root ancestor's
    /// type; this invariant is enforced by `bc-core`, not here.
    #[expect(
        clippy::struct_field_names,
        reason = "account_type is the natural field name; renaming would reduce clarity"
    )]
    account_type: Type,

    /// Commodities this account may hold. An empty list means unrestricted —
    /// the account can hold any commodity. When non-empty, the *first* entry is
    /// used as the default commodity for display purposes. Defaults to empty.
    #[builder(default)]
    commodities: Vec<CommodityId>,

    /// Optional free-text description providing context about this account.
    /// `None` means no description has been recorded.
    #[builder(into)]
    description: Option<String>,

    /// Parent account ID, if this is a sub-account.
    ///
    /// `None` indicates a root account whose [`Type`] is authoritative.
    /// `Some(id)` indicates a child whose type must match its root ancestor
    /// (enforced in `bc-core`, not here).
    parent_id: Option<AccountId>,

    /// Tags for cross-cutting labels (e.g. reporting categories). Applied at the
    /// account level; individual transactions and postings carry their own tags.
    /// Defaults to empty; managed in `bc-core`.
    #[builder(default)]
    tag_ids: Vec<TagId>,

    /// Timestamp recorded when this account was first persisted. Callers
    /// constructing a new account should pass [`jiff::Timestamp::now()`].
    created_at: Timestamp,

    /// Timestamp at which this account was archived, or `None` if still active.
    /// Set via [`Account::archive`]; do not assign directly via the builder
    /// unless re-hydrating an already-archived record.
    archived_at: Option<Timestamp>,
}

impl Account {
    /// Returns the account ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &AccountId {
        &self.id
    }

    /// Returns the account name.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the account name, rejecting empty strings.
    ///
    /// # Errors
    ///
    /// Returns [`ValidationError::EmptyName`] if `name` is empty.
    #[inline]
    pub fn set_name(&mut self, name: String) -> Result<(), ValidationError> {
        if name.is_empty() {
            return Err(ValidationError::EmptyName);
        }
        self.name = name;
        Ok(())
    }

    /// Returns the account type.
    #[inline]
    #[must_use]
    pub fn account_type(&self) -> Type {
        self.account_type
    }

    /// Returns the allowed commodities (empty = unrestricted).
    #[inline]
    #[must_use]
    pub fn commodities(&self) -> &[CommodityId] {
        &self.commodities
    }

    /// Returns the first commodity as the display default.
    #[inline]
    #[must_use]
    pub fn default_commodity(&self) -> Option<&CommodityId> {
        self.commodities.first()
    }

    /// Returns the description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the parent account ID, if any.
    #[inline]
    #[must_use]
    pub fn parent_id(&self) -> Option<&AccountId> {
        self.parent_id.as_ref()
    }

    /// Returns the tag IDs associated with this account.
    #[inline]
    #[must_use]
    pub fn tag_ids(&self) -> &[TagId] {
        &self.tag_ids
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }

    /// Returns the archive timestamp, if archived.
    #[inline]
    #[must_use]
    pub fn archived_at(&self) -> Option<&Timestamp> {
        self.archived_at.as_ref()
    }

    /// Returns `true` if the account has not been archived.
    #[inline]
    #[must_use]
    pub fn is_active(&self) -> bool {
        self.archived_at.is_none()
    }

    /// Archives the account at the given timestamp.
    #[inline]
    pub fn archive(&mut self, at: Timestamp) {
        self.archived_at = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::CommodityId;

    #[test]
    fn account_is_active_when_not_archived() {
        let acct = Account::builder()
            .id(AccountId::new())
            .name("Checking")
            .account_type(Type::Asset)
            .created_at(jiff::Timestamp::now())
            .build();
        assert!(acct.is_active());
    }

    #[test]
    fn account_with_parent_records_parent_id() {
        let parent_id = AccountId::new();
        let acct = Account::builder()
            .id(AccountId::new())
            .name("Savings")
            .account_type(Type::Asset)
            .parent_id(parent_id.clone())
            .created_at(jiff::Timestamp::now())
            .build();
        assert_eq!(acct.parent_id(), Some(&parent_id));
    }

    #[test]
    fn account_root_has_no_parent() {
        let acct = Account::builder()
            .id(AccountId::new())
            .name("Assets")
            .account_type(Type::Asset)
            .created_at(jiff::Timestamp::now())
            .build();
        assert!(acct.parent_id().is_none());
    }

    #[test]
    fn account_builder_constructs_account() {
        use jiff::Timestamp;
        let id = AccountId::new();
        let acct = Account::builder()
            .id(id.clone())
            .name("Savings")
            .account_type(Type::Asset)
            .created_at(Timestamp::now())
            .build();
        assert_eq!(acct.id(), &id);
        assert_eq!(acct.name(), "Savings");
        assert!(acct.commodities().is_empty());
        assert!(acct.is_active());
    }

    #[test]
    fn account_default_commodity_is_first() {
        let id1 = CommodityId::new();
        let id2 = CommodityId::new();
        let acct = Account::builder()
            .id(AccountId::new())
            .name("Brokerage")
            .account_type(Type::Asset)
            .commodities(vec![id1.clone(), id2])
            .created_at(jiff::Timestamp::now())
            .build();
        assert_eq!(acct.default_commodity(), Some(&id1));
    }

    #[test]
    fn set_name_rejects_empty() {
        let mut acct = Account::builder()
            .id(AccountId::new())
            .name("Checking")
            .account_type(Type::Asset)
            .created_at(jiff::Timestamp::now())
            .build();
        acct.set_name("New Name".to_owned())
            .expect("non-empty name should succeed");
        assert!(acct.set_name(String::new()).is_err());
    }

    #[test]
    fn account_archive() {
        use jiff::Timestamp;
        let mut acct = Account::builder()
            .id(AccountId::new())
            .name("Old Account")
            .account_type(Type::Asset)
            .created_at(Timestamp::now())
            .build();
        assert!(acct.is_active());
        acct.archive(Timestamp::now());
        assert!(!acct.is_active());
    }
}
