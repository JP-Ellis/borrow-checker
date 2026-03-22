//! Account domain types.

use jiff::Timestamp;

use crate::{TagPath, ids::AccountId, money::CommodityCode};

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
    /// Parent account ID, if this is a sub-account.
    ///
    /// `None` indicates a root account whose [`AccountType`] is authoritative.
    /// `Some(id)` indicates a child whose type must match its root ancestor
    /// (enforced in `bc-core`, not here).
    pub parent_id: Option<AccountId>,
    /// Cross-cutting hierarchical tags for grouping and filtering.
    ///
    /// Tags complement the primary [`parent_id`] hierarchy by providing
    /// additional axes of organisation (e.g. `institution:commbank`,
    /// `owner:mine`). An empty list means no tags.
    ///
    /// [`parent_id`]: Account::parent_id
    pub tags: Vec<TagPath>,
    /// When the account was created.
    pub created_at: Timestamp,
    /// When the account was archived, if ever.
    pub archived_at: Option<Timestamp>,
}

impl Account {
    /// Creates a new [`Account`] with all fields.
    ///
    /// This constructor is required because the struct is `#[non_exhaustive]`.
    #[expect(
        clippy::too_many_arguments,
        reason = "all fields are required for a complete Account; a builder is not warranted yet"
    )]
    #[inline]
    #[must_use]
    pub fn new(
        id: AccountId,
        name: String,
        account_type: AccountType,
        commodity: CommodityCode,
        description: Option<String>,
        parent_id: Option<AccountId>,
        tags: Vec<TagPath>,
        created_at: Timestamp,
        archived_at: Option<Timestamp>,
    ) -> Self {
        Self {
            id,
            name,
            account_type,
            commodity,
            description,
            parent_id,
            tags,
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
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::{TagPath, ids::AccountId};

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
            parent_id: None,
            tags: vec![],
            created_at: Timestamp::now(),
            archived_at: None,
        };
        assert!(acct.is_active());
    }

    #[test]
    fn account_with_parent_records_parent_id() {
        let parent_id = AccountId::new();
        let acct = Account::new(
            AccountId::new(),
            "Savings".to_owned(),
            AccountType::Asset,
            crate::money::CommodityCode::new("AUD"),
            None,
            Some(parent_id.clone()),
            vec![],
            Timestamp::now(),
            None,
        );
        assert_eq!(acct.parent_id, Some(parent_id));
    }

    #[test]
    fn account_root_has_no_parent() {
        let acct = Account::new(
            AccountId::new(),
            "Assets".to_owned(),
            AccountType::Asset,
            crate::money::CommodityCode::new("AUD"),
            None,
            None,
            vec![],
            Timestamp::now(),
            None,
        );
        assert!(acct.parent_id.is_none());
    }

    #[test]
    fn account_stores_tags() {
        let tag = TagPath::new(["institution", "commbank"]).expect("valid tag");
        let acct = Account::new(
            AccountId::new(),
            "Savings".to_owned(),
            AccountType::Asset,
            crate::money::CommodityCode::new("AUD"),
            None,
            None,
            vec![tag.clone()],
            Timestamp::now(),
            None,
        );
        assert_eq!(acct.tags, vec![tag]);
    }

    #[test]
    fn account_without_tags_has_empty_tag_list() {
        let acct = Account::new(
            AccountId::new(),
            "Checking".to_owned(),
            AccountType::Asset,
            crate::money::CommodityCode::new("AUD"),
            None,
            None,
            vec![],
            Timestamp::now(),
            None,
        );
        assert!(acct.tags.is_empty());
    }
}
