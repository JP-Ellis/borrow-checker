//! Account projection service.

use std::collections::HashMap;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::CommodityId;
use bc_models::EventId;
use bc_models::TagId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;

/// Internal row type returned from the `accounts` table plus join-loaded data.
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
    /// ISO 8601 creation timestamp.
    created_at: String,
    /// ISO 8601 archive timestamp if archived.
    archived_at: Option<String>,
    /// Allowed commodities; first = default; empty = unrestricted.
    commodities: Vec<CommodityId>,
    /// Tags attached to this account.
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

        Ok(Self::builder()
            .id(id)
            .name(row.name)
            .account_type(account_type)
            .kind(kind)
            .commodities(row.commodities)
            .tag_ids(row.tag_ids)
            .maybe_description(row.description)
            .maybe_archived_at(archived_at)
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
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create(
        &self,
        name: &str,
        account_type: AccountType,
        kind: AccountKind,
        description: Option<&str>,
    ) -> BcResult<AccountId> {
        let id = AccountId::new();
        let now = Timestamp::now();
        let event_id = EventId::new().to_string();
        let event = Event::AccountCreated {
            id: id.clone(),
            name: name.to_owned(),
        };
        let payload = serde_json::to_string(&event)?;

        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO events (id, kind, aggregate_id, payload, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&event_id)
        .bind(event.kind())
        .bind(id.to_string())
        .bind(&payload)
        .bind(now.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query(
            "INSERT INTO accounts (id, name, account_type, kind, description, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(id.to_string())
        .bind(name)
        .bind(to_db_str(account_type)?)
        .bind(to_db_str(kind)?)
        .bind(description)
        .bind(now.to_string())
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        tracing::info!(account_id = %id, %name, "account created");
        Ok(id)
    }

    /// Archives an account by setting its `archived_at` timestamp.
    ///
    /// The existence check happens before any write.  The event append and the
    /// projection update are wrapped in a single SQLite transaction so they
    /// succeed or fail atomically.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist or is already archived.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn archive(&self, id: &AccountId) -> BcResult<()> {
        let now = Timestamp::now();

        // Check existence first (before writing any event).
        let exists: Option<(String,)> =
            sqlx::query_as("SELECT id FROM accounts WHERE id = ? AND archived_at IS NULL")
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;

        if exists.is_none() {
            return Err(BcError::NotFound(id.to_string()));
        }

        let event_id = EventId::new().to_string();
        let event = Event::AccountArchived { id: id.clone() };
        let payload = serde_json::to_string(&event)?;

        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "INSERT INTO events (id, kind, aggregate_id, payload, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&event_id)
        .bind(event.kind())
        .bind(id.to_string())
        .bind(&payload)
        .bind(now.to_string())
        .execute(&mut *tx)
        .await?;

        sqlx::query("UPDATE accounts SET archived_at = ? WHERE id = ? AND archived_at IS NULL")
            .bind(now.to_string())
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;

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
        let row = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                Option<String>,
                String,
                Option<String>,
            ),
        >(
            "SELECT id, name, account_type, kind, description, created_at, archived_at \
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

        let commodities: Vec<CommodityId> = commodity_rows
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

        let tag_ids: Vec<TagId> = tag_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid tag_id '{s}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        Account::try_from(AccountRow {
            id: row.0,
            name: row.1,
            account_type: row.2,
            kind: row.3,
            description: row.4,
            created_at: row.5,
            archived_at: row.6,
            commodities,
            tag_ids,
        })
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
        let account_rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                String,
                String,
                Option<String>,
                String,
                Option<String>,
            ),
        >(
            "SELECT id, name, account_type, kind, description, created_at, archived_at \
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

        account_rows
            .into_iter()
            .map(|row| {
                let commodities = commodities_map.remove(&row.0).unwrap_or_default();
                let tag_ids = tags_map.remove(&row.0).unwrap_or_default();
                Account::try_from(AccountRow {
                    id: row.0,
                    name: row.1,
                    account_type: row.2,
                    kind: row.3,
                    description: row.4,
                    created_at: row.5,
                    archived_at: row.6,
                    commodities,
                    tag_ids,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

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
            .create(
                "Checking",
                bc_models::AccountType::Asset,
                bc_models::AccountKind::DepositAccount,
                None,
            )
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
            .create(
                "Old Account",
                bc_models::AccountType::Liability,
                bc_models::AccountKind::DepositAccount,
                None,
            )
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
            .create(
                "House",
                bc_models::AccountType::Asset,
                AccountKind::ManualAsset,
                None,
            )
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.account_type(), bc_models::AccountType::Asset);
        assert_eq!(found.kind(), AccountKind::ManualAsset);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_active_excludes_archived(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let _id1 = svc
            .create(
                "Active",
                bc_models::AccountType::Asset,
                bc_models::AccountKind::DepositAccount,
                None,
            )
            .await
            .expect("create should succeed");
        let id2 = svc
            .create(
                "Archived",
                bc_models::AccountType::Expense,
                bc_models::AccountKind::DepositAccount,
                None,
            )
            .await
            .expect("create should succeed");
        svc.archive(&id2).await.expect("archive should succeed");

        let active = svc.list_active().await.expect("list should succeed");
        assert_eq!(active.len(), 1);
        let first = active.first().expect("one active account should exist");
        assert_eq!(first.name(), "Active");
    }
}
