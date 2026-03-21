//! Global settings store (persisted in the `meta` table).

use bc_models::GlobalSettings;
use sqlx::SqlitePool;

use crate::error::BcResult;

/// The key used in the `meta` table to store global settings.
const SETTINGS_KEY: &str = "global_settings";

/// Persists and retrieves [`GlobalSettings`] in the `meta` key-value table.
#[expect(
    clippy::module_name_repetitions,
    reason = "SettingsStore is the canonical domain name regardless of module path"
)]
#[derive(Debug, Clone)]
pub struct SettingsStore {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl SettingsStore {
    /// Creates a [`SettingsStore`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Loads global settings, returning the default if none have been saved.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BcError`] on database or deserialisation failure.
    #[inline]
    pub async fn load(&self) -> BcResult<GlobalSettings> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM meta WHERE key = ?")
            .bind(SETTINGS_KEY)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some((json,)) => Ok(serde_json::from_str(&json)?),
            None => Ok(GlobalSettings::default()),
        }
    }

    /// Saves global settings (upserts the `meta` row).
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::BcError`] on serialisation or database failure.
    #[inline]
    pub async fn save(&self, settings: &GlobalSettings) -> BcResult<()> {
        let json = serde_json::to_string(settings)?;
        sqlx::query(
            "INSERT INTO meta (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(SETTINGS_KEY)
        .bind(&json)
        .execute(&self.pool)
        .await?;
        tracing::debug!("global settings saved");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bc_models::{GlobalSettings, money::CommodityCode};

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn settings_round_trip(pool: sqlx::SqlitePool) {
        let store = SettingsStore::new(pool.clone());
        let settings = GlobalSettings::new(7, 1, None, CommodityCode::new("AUD"));
        store.save(&settings).await.expect("save should succeed");
        let loaded = store.load().await.expect("load should succeed");
        assert_eq!(loaded.financial_year_start_month, 7);
        assert_eq!(loaded.display_commodity.to_string(), "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_returns_default_when_not_set(pool: sqlx::SqlitePool) {
        let store = SettingsStore::new(pool.clone());
        let loaded = store.load().await.expect("load should succeed");
        assert_eq!(
            loaded.financial_year_start_month,
            GlobalSettings::default().financial_year_start_month
        );
    }
}
