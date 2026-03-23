//! Account projection service.

use bc_models::{Account, AccountId, AccountType, EventId, money::CommodityCode};
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::{
    error::{BcError, BcResult},
    events::Event,
};

/// Converts an [`AccountType`] to its canonical database string.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `at` is an unrecognised variant (future-proofing for
/// `#[non_exhaustive]` additions).
fn account_type_to_str(at: AccountType) -> BcResult<&'static str> {
    match at {
        AccountType::Asset => Ok("asset"),
        AccountType::Liability => Ok("liability"),
        AccountType::Equity => Ok("equity"),
        AccountType::Income => Ok("income"),
        AccountType::Expense => Ok("expense"),
        _ => Err(BcError::BadData(format!("unknown account type: {at:?}"))),
    }
}

/// Parses an [`AccountType`] from its canonical database string.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `s` is not a recognised account-type string.
fn account_type_from_str(s: &str) -> BcResult<AccountType> {
    match s {
        "asset" => Ok(AccountType::Asset),
        "liability" => Ok(AccountType::Liability),
        "equity" => Ok(AccountType::Equity),
        "income" => Ok(AccountType::Income),
        "expense" => Ok(AccountType::Expense),
        other => Err(BcError::BadData(format!("unknown account type: {other}"))),
    }
}

/// Internal row type returned from the `accounts` table.
struct AccountRow {
    /// Raw account ID string.
    id: String,
    /// Account display name.
    name: String,
    /// Account type stored as `snake_case` string.
    account_type: String,
    /// Commodity code string (retained for DB compatibility; not yet mapped to
    /// `CommodityId` — will be wired up in Task 11).
    #[expect(
        dead_code,
        reason = "retained for DB schema compatibility; full mapping in Task 11"
    )]
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

        let account_type = account_type_from_str(&self.account_type)?;

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

        Ok(Account::builder()
            .id(id)
            .name(self.name)
            .account_type(account_type)
            .maybe_description(self.description)
            .maybe_archived_at(archived_at)
            .created_at(created_at)
            .build())
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
}

impl AccountService {
    /// Creates a new [`AccountService`] with the given connection pool.
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
        commodity: CommodityCode,
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
            "INSERT INTO accounts (id, name, account_type, commodity, description, created_at) VALUES (?, ?, ?, ?, ?, ?)"
        )
        .bind(id.to_string())
        .bind(name)
        .bind(account_type_to_str(account_type)?)
        .bind(commodity.as_str())
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
    use pretty_assertions::assert_eq;

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
        assert_eq!(found.name(), "Checking");
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
