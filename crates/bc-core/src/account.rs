//! Account projection service.

use std::collections::HashMap;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::CommodityId;
use bc_models::TagId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;

/// Internal row type returned from the `accounts` table, mapped by `sqlx::FromRow`.
///
/// The `commodities` and `tag_ids` fields are populated separately via join queries
/// after the initial row fetch.
#[derive(sqlx::FromRow)]
struct AccountRow {
    /// Raw account ID string.
    id: String,
    /// Account display name.
    name: String,
    /// Account type stored as `snake_case` string.
    account_type: String,
    /// Account maintenance kind stored as `snake_case` string.
    kind: String,
    /// Optional description.
    description: Option<String>,
    /// Raw parent account ID string, if this account has a parent.
    parent_id: Option<String>,
    /// ISO 8601 creation timestamp.
    created_at: String,
    /// ISO 8601 archive timestamp if archived.
    archived_at: Option<String>,
    /// Acquisition date for `ManualAsset` accounts (YYYY-MM-DD), if recorded.
    acquisition_date: Option<String>,
    /// Acquisition cost as a decimal string, if recorded.
    acquisition_cost: Option<String>,
    /// JSON-encoded `DepreciationPolicy`, if set.
    depreciation_policy: Option<String>,
    /// Allowed commodities; first = default; empty = unrestricted.
    #[sqlx(skip)]
    commodities: Vec<CommodityId>,
    /// Tags attached to this account.
    #[sqlx(skip)]
    tag_ids: Vec<TagId>,
}

impl TryFrom<AccountRow> for Account {
    type Error = BcError;

    /// Converts a raw database row into a domain [`Account`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    #[inline]
    fn try_from(row: AccountRow) -> BcResult<Self> {
        let id = row
            .id
            .parse::<AccountId>()
            .map_err(|e| BcError::BadData(format!("invalid account id '{}': {e}", row.id)))?;

        let account_type = from_db_str::<AccountType>(&row.account_type)?;

        let kind = from_db_str::<AccountKind>(&row.kind)?;

        let created_at = row.created_at.parse::<Timestamp>().map_err(|e| {
            BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;

        let archived_at = row
            .archived_at
            .as_deref()
            .map(|s| {
                s.parse::<Timestamp>()
                    .map_err(|e| BcError::BadData(format!("invalid archived_at '{s}': {e}")))
            })
            .transpose()?;

        let parent_id = row
            .parent_id
            .as_deref()
            .map(|s| {
                s.parse::<AccountId>()
                    .map_err(|e| BcError::BadData(format!("invalid parent_id '{s}': {e}")))
            })
            .transpose()?;

        let acquisition_date = row
            .acquisition_date
            .as_deref()
            .map(|s| {
                s.parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid acquisition_date '{s}': {e}")))
            })
            .transpose()?;

        let acquisition_cost = row
            .acquisition_cost
            .as_deref()
            .map(|s| {
                s.parse::<rust_decimal::Decimal>()
                    .map_err(|e| BcError::BadData(format!("invalid acquisition_cost '{s}': {e}")))
            })
            .transpose()?;

        let depreciation_policy = row
            .depreciation_policy
            .as_deref()
            .map(|s| {
                serde_json::from_str::<bc_models::DepreciationPolicy>(s)
                    .map_err(|e| BcError::BadData(format!("invalid depreciation_policy: {e}")))
            })
            .transpose()?;

        Ok(Self::builder()
            .id(id)
            .name(row.name)
            .account_type(account_type)
            .kind(kind)
            .commodities(row.commodities)
            .tag_ids(row.tag_ids)
            .maybe_description(row.description)
            .maybe_parent_id(parent_id)
            .maybe_archived_at(archived_at)
            .maybe_acquisition_date(acquisition_date)
            .maybe_acquisition_cost(acquisition_cost)
            .maybe_depreciation_policy(depreciation_policy)
            .created_at(created_at)
            .build())
    }
}

/// Parses a slice of `(account_id, commodity_id)` rows into a `HashMap`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any commodity ID string is malformed.
fn build_commodities_map(
    rows: Vec<(String, String)>,
) -> BcResult<HashMap<String, Vec<CommodityId>>> {
    let mut map: HashMap<String, Vec<CommodityId>> = HashMap::new();
    for (account_id, commodity_id) in rows {
        let cid = commodity_id
            .parse::<CommodityId>()
            .map_err(|e| BcError::BadData(format!("invalid commodity_id '{commodity_id}': {e}")))?;
        map.entry(account_id).or_default().push(cid);
    }
    Ok(map)
}

/// Parses a slice of `(account_id, tag_id)` rows into a `HashMap`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any tag ID string is malformed.
fn build_tags_map(rows: Vec<(String, String)>) -> BcResult<HashMap<String, Vec<TagId>>> {
    let mut map: HashMap<String, Vec<TagId>> = HashMap::new();
    for (account_id, tag_id) in rows {
        let tid = tag_id
            .parse::<TagId>()
            .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id}': {e}")))?;
        map.entry(account_id).or_default().push(tid);
    }
    Ok(map)
}

/// Service for creating and managing accounts.
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

#[bon::bon]
impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Creates a new account and returns its ID.
    ///
    /// Both the event append and the projection insert are wrapped in a single
    /// SQLite transaction so they succeed or fail atomically.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name for the new account.
    /// * `account_type` - Classification in the chart of accounts.
    /// * `kind` - Account maintenance kind.
    /// * `description` - Optional free-text description.
    /// * `parent_id` - Optional parent account ID for sub-accounts.
    /// * `commodity_ids` - Ordered list of allowed commodity IDs; first entry is the default.
    /// * `tag_ids` - Tags to attach to the account.
    /// * `acquisition_date` - Date the asset was acquired (only for [`AccountKind::ManualAsset`]).
    /// * `acquisition_cost` - Cost of acquisition (only for [`AccountKind::ManualAsset`]).
    /// * `depreciation_policy` - Depreciation method (only for [`AccountKind::ManualAsset`]).
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database insert failure.
    #[builder]
    #[inline]
    pub async fn create(
        &self,
        name: &str,
        account_type: AccountType,
        kind: AccountKind,
        description: Option<&str>,
        parent_id: Option<&AccountId>,
        #[builder(default)] commodity_ids: &[CommodityId],
        #[builder(default)] tag_ids: &[TagId],
        acquisition_date: Option<jiff::civil::Date>,
        acquisition_cost: Option<rust_decimal::Decimal>,
        depreciation_policy: Option<&bc_models::DepreciationPolicy>,
    ) -> BcResult<AccountId> {
        let id = AccountId::new();
        let now = Timestamp::now();
        let event = Event::AccountCreated {
            id: id.clone(),
            name: name.to_owned(),
            account_type,
            kind,
            description: description.map(str::to_owned),
        };

        let mut tx = self.pool.begin().await?;

        insert_event(&event, &mut tx).await?;

        sqlx::query(
            "INSERT INTO accounts (id, name, account_type, kind, description, parent_id, created_at, \
             acquisition_date, acquisition_cost, depreciation_policy) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        )
        .bind(id.to_string())
        .bind(name)
        .bind(to_db_str(account_type)?)
        .bind(to_db_str(kind)?)
        .bind(description)
        .bind(parent_id.map(AccountId::to_string))
        .bind(now.to_string())
        .bind(acquisition_date.map(|d| d.to_string()))
        .bind(acquisition_cost.map(|c| c.to_string()))
        .bind(
            depreciation_policy
                .map(serde_json::to_string)
                .transpose()
                .map_err(BcError::Serialisation)?,
        )
        .execute(&mut *tx)
        .await?;

        for (position, commodity_id) in commodity_ids.iter().enumerate() {
            sqlx::query(
                "INSERT INTO account_commodities (account_id, commodity_id, position) VALUES (?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(commodity_id.to_string())
            .bind(
                i64::try_from(position)
                    .map_err(|e| BcError::BadData(format!("commodity position overflow: {e}")))?,
            )
            .execute(&mut *tx)
            .await?;
        }

        for tag_id in tag_ids {
            sqlx::query("INSERT INTO account_tags (account_id, tag_id) VALUES (?, ?)")
                .bind(id.to_string())
                .bind(tag_id.to_string())
                .execute(&mut *tx)
                .await?;
        }

        tx.commit().await?;
        tracing::info!(account_id = %id, %name, "account created");
        Ok(id)
    }

    /// Archives an account by setting its `archived_at` timestamp.
    ///
    /// The event append and the projection UPDATE are wrapped in a single SQLite
    /// transaction so they succeed or fail atomically.  `rows_affected()` is used
    /// to detect a missing or already-archived account without a separate pre-check,
    /// eliminating a TOCTOU race.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::AlreadyArchived`] if the account exists but is already archived.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn archive(&self, id: &AccountId) -> BcResult<()> {
        let now = Timestamp::now();
        let event = Event::AccountArchived { id: id.clone() };

        let mut tx = self.pool.begin().await?;

        insert_event(&event, &mut tx).await?;

        let result =
            sqlx::query("UPDATE accounts SET archived_at = ? WHERE id = ? AND archived_at IS NULL")
                .bind(now.to_string())
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;

        if result.rows_affected() == 0 {
            // rows_affected == 0 means the UPDATE found no matching row.
            // Returning here drops `tx` without committing — sqlx rolls it
            // back implicitly, discarding the event insert above.
            //
            // Perform a follow-up SELECT to distinguish "not found" from
            // "already archived" so callers get a semantic error.
            let exists: bool = sqlx::query_scalar("SELECT count(*) > 0 FROM accounts WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(&self.pool)
                .await?;

            return if exists {
                Err(BcError::AlreadyArchived(id.clone()))
            } else {
                Err(BcError::NotFound(id.to_string()))
            };
        }

        tx.commit().await?;
        tracing::info!(account_id = %id, "account archived");
        Ok(())
    }

    /// Finds an account by ID, including its commodity and tag associations.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no account with that ID exists.
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn find_by_id(&self, id: &AccountId) -> BcResult<Account> {
        let mut row = sqlx::query_as::<_, AccountRow>(
            "SELECT id, name, account_type, kind, description, parent_id, created_at, archived_at, \
             acquisition_date, acquisition_cost, depreciation_policy \
             FROM accounts WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(id.to_string()))?;

        let commodity_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT commodity_id FROM account_commodities WHERE account_id = ? ORDER BY position",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        row.commodities = commodity_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<CommodityId>()
                    .map_err(|e| BcError::BadData(format!("invalid commodity_id '{s}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        let tag_rows: Vec<(String,)> =
            sqlx::query_as("SELECT tag_id FROM account_tags WHERE account_id = ?")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;

        row.tag_ids = tag_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid tag_id '{s}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        Account::try_from(row)
    }

    /// Lists all active (non-archived) accounts, ordered by name.
    ///
    /// Commodity and tag associations are loaded in bulk (two additional queries)
    /// to avoid N+1 database round-trips.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_active(&self) -> BcResult<Vec<Account>> {
        let mut account_rows = sqlx::query_as::<_, AccountRow>(
            "SELECT id, name, account_type, kind, description, parent_id, created_at, archived_at, \
             acquisition_date, acquisition_cost, depreciation_policy \
             FROM accounts WHERE archived_at IS NULL ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        if account_rows.is_empty() {
            return Ok(vec![]);
        }

        // Load all commodity associations for active accounts in one query.
        let commodity_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT ac.account_id, ac.commodity_id \
             FROM account_commodities ac \
             JOIN accounts a ON ac.account_id = a.id \
             WHERE a.archived_at IS NULL \
             ORDER BY ac.account_id, ac.position",
        )
        .fetch_all(&self.pool)
        .await?;

        // Load all tag associations for active accounts in one query.
        let tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT at.account_id, at.tag_id \
             FROM account_tags at \
             JOIN accounts a ON at.account_id = a.id \
             WHERE a.archived_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut commodities_map = build_commodities_map(commodity_rows)?;
        let mut tags_map = build_tags_map(tag_rows)?;

        for row in &mut account_rows {
            row.commodities = commodities_map.remove(&row.id).unwrap_or_default();
            row.tag_ids = tags_map.remove(&row.id).unwrap_or_default();
        }

        account_rows.into_iter().map(Account::try_from).collect()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_via_builder_api(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create via builder");
        assert!(id.to_string().starts_with("account_"));
    }

    #[test]
    fn account_kind_round_trips() {
        use bc_models::AccountKind;
        for (kind, expected) in [
            (AccountKind::DepositAccount, "deposit_account"),
            (AccountKind::ManualAsset, "manual_asset"),
            (AccountKind::Receivable, "receivable"),
            (AccountKind::VirtualAllocation, "virtual_allocation"),
        ] {
            let s = to_db_str(kind).expect("known variant should serialise");
            assert_eq!(s, expected);
            let back = from_db_str::<AccountKind>(&s).expect("known string should deserialise");
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn account_kind_from_str_rejects_unknown() {
        from_db_str::<AccountKind>("bogus").expect_err("unknown string should fail");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_persists_projection(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.name(), "Checking");
        assert!(found.is_active());
        assert!(found.commodities().is_empty());
        assert!(found.tag_ids().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_account_sets_archived_at(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Old Account")
            .account_type(bc_models::AccountType::Liability)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");

        svc.archive(&id).await.expect("archive should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert!(!found.is_active());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_with_kind_persists(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("House")
            .account_type(bc_models::AccountType::Asset)
            .kind(AccountKind::ManualAsset)
            .call()
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.account_type(), bc_models::AccountType::Asset);
        assert_eq!(found.kind(), AccountKind::ManualAsset);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_nonexistent_account_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::AccountId::new();
        let result = svc.archive(&fake_id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
        // Verify the failed archive did not leave any orphaned events.
        let store = crate::events::SqliteStore::new(pool.clone());
        let events = store
            .replay_for(&fake_id.to_string())
            .await
            .expect("replay should succeed");
        assert!(
            events.is_empty(),
            "failed archive must not leave events in the log"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_already_archived_returns_already_archived(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Savings")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        svc.archive(&id)
            .await
            .expect("first archive should succeed");
        let result = svc.archive(&id).await;
        assert!(matches!(result, Err(BcError::AlreadyArchived(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_by_id_nonexistent_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::AccountId::new();
        let result = svc.find_by_id(&fake_id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_with_commodities_and_tags(pool: sqlx::SqlitePool) {
        // Insert a commodity row directly since there is no CommodityService yet.
        let commodity_id = bc_models::CommodityId::new();
        sqlx::query("INSERT INTO commodities (id, code) VALUES (?, ?)")
            .bind(commodity_id.to_string())
            .bind("USD")
            .execute(&pool)
            .await
            .expect("commodity insert should succeed");

        // Insert a tag row directly since there is no TagService yet.
        let tag_id = bc_models::TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
            .bind(tag_id.to_string())
            .bind("savings")
            .bind(jiff::Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("tag insert should succeed");

        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .commodity_ids(core::slice::from_ref(&commodity_id))
            .tag_ids(core::slice::from_ref(&tag_id))
            .call()
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.commodities(), &[commodity_id]);
        assert_eq!(found.tag_ids(), &[tag_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_active_excludes_archived(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let _id1 = svc
            .create()
            .name("Active")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        let id2 = svc
            .create()
            .name("Archived")
            .account_type(bc_models::AccountType::Expense)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        svc.archive(&id2).await.expect("archive should succeed");

        let active = svc.list_active().await.expect("list should succeed");
        assert_eq!(active.len(), 1);
        let first = active.first().expect("one active account should exist");
        assert_eq!(first.name(), "Active");
    }
}
