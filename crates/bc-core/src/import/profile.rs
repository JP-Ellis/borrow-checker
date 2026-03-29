//! Import profile storage service.
//!
//! An [`ImportProfile`] captures the association between a financial account,
//! a named importer (e.g. `"csv"`, `"ofx"`), and the opaque JSON configuration
//! blob that drives that importer.  The [`Service`] provides CRUD operations
//! backed by the `import_profiles` SQLite table.

use bc_models::AccountId;
use bc_models::ProfileId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use super::Config;
use crate::BcError;
use crate::BcResult;

/// A persisted import profile linking an account to an importer and its config.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub struct ImportProfile {
    /// Unique identifier for this profile.
    pub id: ProfileId,
    /// Human-readable name for the profile.
    pub name: String,
    /// Stable identifier of the importer plugin (e.g. `"csv"`, `"ofx"`).
    pub importer: String,
    /// The account this profile feeds transactions into.
    pub account_id: AccountId,
    /// Opaque JSON configuration passed to the importer.
    pub config: Config,
    /// The timestamp when this profile was created.
    pub created_at: Timestamp,
}

/// Service for creating and managing import profiles.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    ///
    /// # Arguments
    ///
    /// * `pool` - A SQLite connection pool connected to the BorrowChecker database.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Creates a new import profile and returns its ID.
    ///
    /// # Arguments
    ///
    /// * `name` - Human-readable name for the profile.
    /// * `importer` - Stable identifier of the importer plugin.
    /// * `account_id` - The account this profile will feed transactions into.
    /// * `config` - Opaque JSON configuration for the importer.
    ///
    /// # Returns
    ///
    /// The [`ProfileId`] of the newly created profile.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Serialisation`] if the config cannot be serialised.
    /// Returns [`BcError::Database`] on database insert failure.
    #[inline]
    pub async fn create(
        &self,
        name: &str,
        importer: &str,
        account_id: &AccountId,
        config: Config,
    ) -> BcResult<ProfileId> {
        let id = ProfileId::new();
        let created_at = Timestamp::now();
        let config_json =
            serde_json::to_string(config.as_value()).map_err(BcError::Serialisation)?;

        sqlx::query(
            "INSERT INTO import_profiles \
             (id, name, importer, account_id, config, created_at) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(importer)
        .bind(account_id.to_string())
        .bind(&config_json)
        .bind(created_at.to_string())
        .execute(&self.pool)
        .await?;

        tracing::info!(profile_id = %id, %name, %importer, "import profile created");
        Ok(id)
    }

    /// Updates an existing import profile's mutable fields.
    ///
    /// The `account_id` and `created_at` fields are immutable after creation.
    ///
    /// # Arguments
    ///
    /// * `id` - The [`ProfileId`] of the profile to update.
    /// * `name` - New human-readable name.
    /// * `importer` - New stable importer identifier.
    /// * `config` - New opaque JSON configuration.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no profile with that ID exists.
    /// Returns [`BcError::Serialisation`] if the config cannot be serialised.
    /// Returns [`BcError::Database`] on database update failure.
    #[inline]
    pub async fn update(
        &self,
        id: &ProfileId,
        name: &str,
        importer: &str,
        config: Config,
    ) -> BcResult<()> {
        let config_json =
            serde_json::to_string(config.as_value()).map_err(BcError::Serialisation)?;

        let result = sqlx::query(
            "UPDATE import_profiles \
             SET name = ?, importer = ?, config = ? \
             WHERE id = ?",
        )
        .bind(name)
        .bind(importer)
        .bind(&config_json)
        .bind(id.to_string())
        .execute(&self.pool)
        .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(format!("import profile {id}")));
        }

        tracing::info!(profile_id = %id, %name, %importer, "import profile updated");
        Ok(())
    }

    /// Finds an import profile by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The [`ProfileId`] to look up.
    ///
    /// # Returns
    ///
    /// The [`ImportProfile`] if found.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no profile with that ID exists.
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    /// Returns [`BcError::Database`] on database query failure.
    #[inline]
    pub async fn find_by_id(&self, id: &ProfileId) -> BcResult<ImportProfile> {
        let row: (String, String, String, String, String, String) = sqlx::query_as(
            "SELECT id, name, importer, account_id, config, created_at \
             FROM import_profiles WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(format!("import profile {id}")))?;

        parse_row(&row.0, row.1, row.2, &row.3, &row.4, &row.5)
    }

    /// Lists all import profiles for a given account, ordered by creation time.
    ///
    /// # Arguments
    ///
    /// * `account_id` - The account whose profiles to list.
    ///
    /// # Returns
    ///
    /// A [`Vec`] of [`ImportProfile`] values ordered by `created_at` ascending.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    /// Returns [`BcError::Database`] on database query failure.
    #[inline]
    pub async fn list_for_account(&self, account_id: &AccountId) -> BcResult<Vec<ImportProfile>> {
        let rows: Vec<(String, String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, name, importer, account_id, config, created_at \
             FROM import_profiles WHERE account_id = ? ORDER BY created_at ASC",
        )
        .bind(account_id.to_string())
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(
                |(raw_id, name, importer, raw_account_id, raw_config, raw_created_at)| {
                    parse_row(
                        &raw_id,
                        name,
                        importer,
                        &raw_account_id,
                        &raw_config,
                        &raw_created_at,
                    )
                },
            )
            .collect()
    }

    /// Deletes an import profile by its ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The [`ProfileId`] of the profile to delete.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no profile with that ID exists.
    /// Returns [`BcError::Database`] on database delete failure.
    #[inline]
    pub async fn delete(&self, id: &ProfileId) -> BcResult<()> {
        let result = sqlx::query("DELETE FROM import_profiles WHERE id = ?")
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(format!("import profile {id}")));
        }

        tracing::info!(profile_id = %id, "import profile deleted");
        Ok(())
    }
}

/// Parses six raw database column strings into an [`ImportProfile`].
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any ID or timestamp string is malformed.
/// Returns [`BcError::Serialisation`] if the config JSON is malformed.
#[inline]
fn parse_row(
    raw_id: &str,
    name: String,
    importer: String,
    raw_account_id: &str,
    raw_config: &str,
    raw_created_at: &str,
) -> BcResult<ImportProfile> {
    let id = raw_id
        .parse::<ProfileId>()
        .map_err(|e: bc_models::IdParseError| BcError::BadData(e.to_string()))?;

    let account_id = raw_account_id
        .parse::<AccountId>()
        .map_err(|e: bc_models::IdParseError| BcError::BadData(e.to_string()))?;

    let created_at = raw_created_at
        .parse::<Timestamp>()
        .map_err(|e| BcError::BadData(e.to_string()))?;

    let config_value =
        serde_json::from_str::<serde_json::Value>(raw_config).map_err(BcError::Serialisation)?;
    let config = Config::from_value(config_value);

    Ok(ImportProfile {
        id,
        name,
        importer,
        account_id,
        config,
        created_at,
    })
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use pretty_assertions::assert_eq;
    use sqlx::SqlitePool;

    use super::*;

    /// Creates a minimal account and returns its ID, for use in profile tests.
    async fn make_account(pool: &SqlitePool) -> AccountId {
        crate::AccountService::new(pool.clone())
            .create(
                "Savings",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("create account")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_and_find_by_id_returns_matching_data(pool: SqlitePool) {
        let account_id = make_account(&pool).await;
        let svc = Service::new(pool.clone());

        let config = Config::default();
        let id = svc
            .create("My Profile", "csv", &account_id, config)
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.id, id);
        assert_eq!(found.name, "My Profile");
        assert_eq!(found.importer, "csv");
        assert_eq!(found.account_id, account_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_changes_mutable_fields(pool: SqlitePool) {
        let account_id = make_account(&pool).await;
        let svc = Service::new(pool.clone());

        let id = svc
            .create("Old Name", "csv", &account_id, Config::default())
            .await
            .expect("create should succeed");

        svc.update(&id, "New Name", "ofx", Config::default())
            .await
            .expect("update should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.name, "New Name");
        assert_eq!(found.importer, "ofx");
        // account_id must not change
        assert_eq!(found.account_id, account_id);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn update_nonexistent_returns_not_found(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = ProfileId::new();
        let result = svc.update(&fake_id, "Name", "csv", Config::default()).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_for_account_returns_correct_count(pool: SqlitePool) {
        let account_id = make_account(&pool).await;
        let svc = Service::new(pool.clone());

        svc.create("Profile A", "csv", &account_id, Config::default())
            .await
            .expect("create first should succeed");
        svc.create("Profile B", "ofx", &account_id, Config::default())
            .await
            .expect("create second should succeed");

        let profiles = svc
            .list_for_account(&account_id)
            .await
            .expect("list should succeed");
        assert_eq!(profiles.len(), 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_removes_the_profile(pool: SqlitePool) {
        let account_id = make_account(&pool).await;
        let svc = Service::new(pool.clone());

        let id = svc
            .create("To Delete", "csv", &account_id, Config::default())
            .await
            .expect("create should succeed");

        svc.delete(&id).await.expect("delete should succeed");

        let result = svc.find_by_id(&id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_nonexistent_returns_not_found(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = ProfileId::new();
        let result = svc.delete(&fake_id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }
}
