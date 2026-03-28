//! Account domain types.

use jiff::Timestamp;

use crate::CommodityId;
use crate::TagId;

crate::define_id!(AccountId, "account");

/// The maintenance kind of an account — governs how its balance is updated.
///
/// Re-exported from the crate root as [`crate::AccountKind`].
///
/// # Example
///
/// ```
/// use bc_models::AccountKind;
///
/// let kind = AccountKind::DepositAccount;
/// assert_eq!(kind, AccountKind::DepositAccount);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Reconciles against a bank/card/brokerage statement.
    /// May have an import profile. This is the default kind.
    ///
    /// Examples: checking account, savings, credit card, investment portfolio.
    DepositAccount,
    /// Manually-maintained real asset with no bank statement.
    /// Balance is driven by periodic valuation events.
    ///
    /// Examples: real property, vehicle, private equity stake.
    ManualAsset,
    /// Money owed to you by a third party.
    /// Tracked via ordinary transactions; may carry optional loan terms.
    ///
    /// Examples: personal loan to a friend, loan to a family trust.
    Receivable,
    /// Virtual allocation with no independent existence.
    /// Subdivides a parent account's balance.
    ///
    /// Examples: earmarked sub-accounts within an offset account.
    VirtualAllocation,
}

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
///
/// let account = Account::builder()
///     .name("Checking")
///     .account_type(AccountType::Asset)
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
    /// Stable, opaque identifier for this account (a prefixed `UUIDv7`).
    /// Defaults to a freshly generated value; only supply this when
    /// re-hydrating a record from storage.
    #[builder(default)]
    id: AccountId,

    /// Display name shown in reports and the user interface. Must not be empty
    /// — [`Account::set_name`] enforces this on mutation, but the builder does
    /// not validate at construction time.
    #[builder(into)]
    name: String,

    /// Classification in the chart of accounts. Determines how the account
    /// contributes to the balance sheet and P&L statements.
    ///
    /// A sub-account (one with a `parent_id`) must share the root ancestor's
    /// type; this invariant is enforced by `bc-core`, not here.
    #[expect(
        clippy::struct_field_names,
        reason = "account_type is the idiomatic name; renaming to avoid the lint would obscure intent"
    )]
    account_type: Type,

    /// Account maintenance kind — governs how this account's balance is updated.
    ///
    /// Defaults to [`Kind::DepositAccount`], which is backwards-compatible with all
    /// existing accounts. Only `DepositAccount` accounts may have an import profile
    /// attached; this invariant is enforced by `bc-core` at creation time.
    /// Deferred to Milestone 2 (import profiles not yet implemented in M1).
    #[builder(default = Kind::DepositAccount)]
    kind: Kind,

    /// Commodities this account may hold. An empty list means unrestricted —
    /// the account can hold any commodity. When non-empty, the *first* entry is
    /// used as the default commodity for display purposes. Defaults to empty.
    #[builder(default)]
    commodities: Vec<CommodityId>,

    /// Optional free-text description providing context about this account.
    /// Defaults to `None`; supply a value to surface it in reports and exports.
    #[builder(into)]
    description: Option<String>,

    /// Parent account ID, if this is a sub-account.
    ///
    /// `None` indicates a root account whose [`Type`] is authoritative.
    /// `Some(id)` indicates a child whose type must match its root ancestor
    /// (enforced in `bc-core`, not here, as the model can only inspect the id,
    /// without access to the underlying account).
    parent_id: Option<AccountId>,

    /// Tags for cross-cutting labels (e.g. reporting categories). Applied at the
    /// account level; individual transactions and postings carry their own tags.
    /// Defaults to empty; managed in `bc-core`.
    #[builder(default)]
    tag_ids: Vec<TagId>,

    /// Timestamp recorded when this account was first persisted. Defaults to
    /// [`jiff::Timestamp::now()`].
    #[builder(default = jiff::Timestamp::now())]
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
    #[expect(
        clippy::shadow_reuse,
        reason = "shadowing `name` with its owned form is the idiomatic conversion pattern"
    )]
    pub fn set_name(&mut self, name: impl Into<String>) -> Result<(), ValidationError> {
        let name = name.into();
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

    /// Returns the account kind.
    #[inline]
    #[must_use]
    pub fn kind(&self) -> Kind {
        self.kind
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
    ///
    /// This is a no-op if the account is already archived; the original
    /// archive timestamp is preserved.
    #[inline]
    pub fn archive(&mut self, at: Timestamp) {
        if self.archived_at.is_none() {
            self.archived_at = Some(at);
        }
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::CommodityId;

    #[test]
    fn account_id_has_correct_prefix() {
        assert!(AccountId::new().to_string().starts_with("account_"));
    }

    #[test]
    fn account_is_active_when_not_archived() {
        let acct = Account::builder()
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
            .name("Checking")
            .account_type(Type::Asset)
            .created_at(jiff::Timestamp::now())
            .build();
        acct.set_name("New Name".to_owned())
            .expect("non-empty name should succeed");
        assert!(acct.set_name(String::new()).is_err());
    }

    #[test]
    fn account_kind_defaults_to_deposit_account() {
        let acct = Account::builder()
            .name("Checking")
            .account_type(Type::Asset)
            .build();
        assert_eq!(acct.kind(), Kind::DepositAccount);
    }

    #[test]
    fn account_kind_can_be_set_explicitly() {
        let acct = Account::builder()
            .name("House")
            .account_type(Type::Asset)
            .kind(Kind::ManualAsset)
            .build();
        assert_eq!(acct.kind(), Kind::ManualAsset);
    }

    #[test]
    fn account_kind_variants_exist() {
        _ = (
            Kind::DepositAccount,
            Kind::ManualAsset,
            Kind::Receivable,
            Kind::VirtualAllocation,
        );
    }

    #[test]
    #[expect(
        clippy::no_effect_underscore_binding,
        reason = "intentional: verifies Kind is Copy by using the same value twice"
    )]
    fn account_kind_is_copy() {
        let k = Kind::DepositAccount;
        let _a = k;
        let _b = k; // only compiles if Kind is Copy
    }

    #[test]
    fn account_type_variants_exist() {
        _ = (
            Type::Asset,
            Type::Liability,
            Type::Equity,
            Type::Income,
            Type::Expense,
        );
    }

    #[test]
    fn account_created_at_defaults_to_current_time() {
        let before = jiff::Timestamp::now();
        let acct = Account::builder()
            .name("Defaults Test")
            .account_type(Type::Asset)
            .build();
        let after = jiff::Timestamp::now();
        assert!(acct.created_at() >= &before);
        assert!(acct.created_at() <= &after);
    }

    #[test]
    fn account_archive() {
        use jiff::Timestamp;
        let mut acct = Account::builder()
            .name("Old Account")
            .account_type(Type::Asset)
            .created_at(Timestamp::now())
            .build();
        assert!(acct.is_active());
        acct.archive(Timestamp::now());
        assert!(!acct.is_active());
    }

    #[test]
    fn account_archive_is_noop_when_already_archived() {
        use jiff::Timestamp;
        let first_ts = Timestamp::now();
        let mut acct = Account::builder()
            .name("Old Account")
            .account_type(Type::Asset)
            .created_at(Timestamp::now())
            .archived_at(first_ts)
            .build();
        assert!(!acct.is_active());
        let original_archived_at = acct.archived_at().copied();
        // Archiving again with a different timestamp must not overwrite the original
        acct.archive(Timestamp::now());
        assert_eq!(acct.archived_at(), original_archived_at.as_ref());
    }

    #[test]
    fn set_name_accepts_str_reference() {
        let mut acct = Account::builder()
            .name("Checking")
            .account_type(Type::Asset)
            .build();
        acct.set_name("Savings").expect("&str should be accepted");
        assert_eq!(acct.name(), "Savings");
    }
}
