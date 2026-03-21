//! Account projection service.

use bc_models::{Account, AccountType, ids::AccountId, money::CommodityCode};
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::{
    error::{BcError, BcResult},
    events::{Event, SqliteEventStore},
};

/// Internal row type returned from the `accounts` table.
struct AccountRow {
    /// Raw account ID string.
    id: String,
    /// Account display name.
    name: String,
    /// Account type stored as `snake_case` string.
    account_type: String,
    /// Commodity code string.
    commodity: String,
    /// Optional description.
    description: Option<String>,
    /// ISO 8601 creation timestamp.
    created_at: String,
    /// ISO 8601 archive timestamp if archived.
    archived_at: Option<String>,
}

impl AccountRow {
    /// Converts this row into a domain [`Account`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    fn into_account(self) -> BcResult<Account> {
        let id = self
            .id
            .parse::<AccountId>()
            .map_err(|e| BcError::BadData(format!("invalid account id '{}': {e}", self.id)))?;

        // account_type is stored as snake_case JSON string value (without quotes)
        let account_type: AccountType = serde_json::from_str(&format!("\"{}\"", self.account_type))
            .map_err(|e| {
                BcError::BadData(format!("invalid account_type '{}': {e}", self.account_type))
            })?;

        let commodity = CommodityCode::new(self.commodity);

        let created_at = self.created_at.parse::<Timestamp>().map_err(|e| {
            BcError::BadData(format!("invalid created_at '{}': {e}", self.created_at))
        })?;

        let archived_at = self
            .archived_at
            .as_deref()
            .map(|s| {
                s.parse::<Timestamp>()
                    .map_err(|e| BcError::BadData(format!("invalid archived_at '{s}': {e}")))
            })
            .transpose()?;

        Ok(Account::new(
            id,
            self.name,
            account_type,
            commodity,
            self.description,
            created_at,
            archived_at,
        ))
    }
}

/// Service for creating and managing accounts.
#[expect(
    clippy::module_name_repetitions,
    reason = "AccountService is the canonical domain name regardless of module path"
)]
#[derive(Debug, Clone)]
pub struct AccountService {
    /// The SQLite connection pool.
    pool: SqlitePool,
    /// The event store for appending domain events.
    events: SqliteEventStore,
}

impl AccountService {
    /// Creates a new [`AccountService`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        let events = SqliteEventStore::new(pool.clone());
        Self { pool, events }
    }

    /// Creates a new account and returns its ID.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create(
        &self,
        name: &str,
        account_type: AccountType,
        commodity: CommodityCode,
        description: Option<&str>,
    ) -> BcResult<AccountId> {
        let id = AccountId::new();
        let created_at = Timestamp::now();

        self.events
            .append(&Event::AccountCreated {
                id: id.clone(),
                name: name.to_owned(),
            })
            .await?;

        // Serialize account_type as the snake_case string value (strip JSON quotes)
        let account_type_str =
            serde_json::to_string(&account_type).map(|s| s.trim_matches('"').to_owned())?;

        sqlx::query(
            "INSERT INTO accounts (id, name, account_type, commodity, description, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(id.to_string())
        .bind(name)
        .bind(&account_type_str)
        .bind(commodity.as_str())
        .bind(description)
        .bind(created_at.to_string())
        .execute(&self.pool)
        .await?;

        tracing::debug!(account_id = %id, %name, "account created");
        Ok(id)
    }

    /// Archives an account by setting its `archived_at` timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist or is already archived.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn archive(&self, id: &AccountId) -> BcResult<()> {
        self.events
            .append(&Event::AccountArchived { id: id.clone() })
            .await?;

        let archived_at = Timestamp::now().to_string();
        let rows_affected =
            sqlx::query("UPDATE accounts SET archived_at = ? WHERE id = ? AND archived_at IS NULL")
                .bind(&archived_at)
                .bind(id.to_string())
                .execute(&self.pool)
                .await?
                .rows_affected();

        if rows_affected == 0 {
            return Err(BcError::NotFound(id.to_string()));
        }

        tracing::debug!(account_id = %id, "account archived");
        Ok(())
    }

    /// Finds an account by ID.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no account with that ID exists.
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn find_by_id(&self, id: &AccountId) -> BcResult<Account> {
        let maybe_row = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, Option<String>)>(
            "SELECT id, name, account_type, commodity, description, created_at, archived_at FROM accounts WHERE id = ?"
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;

        let row = maybe_row.ok_or_else(|| BcError::NotFound(id.to_string()))?;

        AccountRow {
            id: row.0,
            name: row.1,
            account_type: row.2,
            commodity: row.3,
            description: row.4,
            created_at: row.5,
            archived_at: row.6,
        }
        .into_account()
    }

    /// Lists all active (non-archived) accounts, ordered by name.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_active(&self) -> BcResult<Vec<Account>> {
        let rows = sqlx::query_as::<_, (String, String, String, String, Option<String>, String, Option<String>)>(
            "SELECT id, name, account_type, commodity, description, created_at, archived_at FROM accounts WHERE archived_at IS NULL ORDER BY name ASC"
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|row| {
                AccountRow {
                    id: row.0,
                    name: row.1,
                    account_type: row.2,
                    commodity: row.3,
                    description: row.4,
                    created_at: row.5,
                    archived_at: row.6,
                }
                .into_account()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use bc_models::money::CommodityCode;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_persists_projection(pool: sqlx::SqlitePool) {
        let svc = AccountService::new(pool.clone());
        let id = svc
            .create(
                "Checking",
                bc_models::AccountType::Asset,
                CommodityCode::new("AUD"),
                None,
            )
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.name, "Checking");
        assert!(found.is_active());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_account_sets_archived_at(pool: sqlx::SqlitePool) {
        let svc = AccountService::new(pool.clone());
        let id = svc
            .create(
                "Old Account",
                bc_models::AccountType::Liability,
                CommodityCode::new("USD"),
                None,
            )
            .await
            .expect("create should succeed");

        svc.archive(&id).await.expect("archive should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert!(!found.is_active());
    }
}
