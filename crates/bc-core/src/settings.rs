//! Settings persistence for application-wide configuration.
//!
//! [`Settings`] is defined in `bc-config`; this module provides the `Store`
//! that persists and retrieves settings from the `meta` key-value table.

pub use bc_config::Settings;
use sqlx::SqlitePool;

use crate::BcResult;

/// The key used in the `meta` table to store application settings.
const SETTINGS_KEY: &str = "global_settings";

/// Persists and retrieves [`Settings`] in the `meta` key-value table.
#[derive(Debug, Clone)]
pub struct Store {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl Store {
    /// Creates a [`Store`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Loads settings, returning the default if none have been saved.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on database or deserialisation failure.
    #[inline]
    pub async fn load(&self) -> BcResult<Settings> {
        let row: Option<(String,)> = sqlx::query_as("SELECT value FROM meta WHERE key = ?")
            .bind(SETTINGS_KEY)
            .fetch_optional(&self.pool)
            .await?;
        match row {
            Some((json,)) => Ok(serde_json::from_str(&json)?),
            None => Ok(Settings::default()),
        }
    }

    /// Saves settings (upserts the `meta` row).
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on serialisation or database failure.
    #[inline]
    pub async fn save(&self, settings: &Settings) -> BcResult<()> {
        let json = serde_json::to_string(settings)?;
        sqlx::query(
            "INSERT INTO meta (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        )
        .bind(SETTINGS_KEY)
        .bind(&json)
        .execute(&self.pool)
        .await?;
        tracing::debug!("settings saved");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bc_models::CommodityCode;
    use pretty_assertions::assert_eq;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn settings_round_trip(pool: sqlx::SqlitePool) {
        let store = Store::new(pool.clone());
        let settings = Settings::default();
        store.save(&settings).await.expect("save should succeed");
        let loaded = store.load().await.expect("load should succeed");
        assert_eq!(
            loaded.financial_year_start_month(),
            settings.financial_year_start_month()
        );
        assert_eq!(
            loaded.display_commodity().as_str(),
            settings.display_commodity().as_str()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn load_returns_default_when_not_set(pool: sqlx::SqlitePool) {
        let store = Store::new(pool.clone());
        let loaded = store.load().await.expect("load should succeed");
        assert_eq!(
            loaded.financial_year_start_month(),
            Settings::default().financial_year_start_month()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn save_twice_updates_not_duplicates(pool: sqlx::SqlitePool) {
        let store = Store::new(pool.clone());

        let json1 = r#"{"financial_year_start_month":7,"financial_year_start_day":1,"fortnightly_anchor":null,"display_commodity":"USD"}"#;
        let settings1: Settings =
            serde_json::from_str(json1).expect("parse settings1 should succeed");
        store
            .save(&settings1)
            .await
            .expect("first save should succeed");

        let json2 = r#"{"financial_year_start_month":7,"financial_year_start_day":1,"fortnightly_anchor":null,"display_commodity":"EUR"}"#;
        let settings2: Settings =
            serde_json::from_str(json2).expect("parse settings2 should succeed");
        store
            .save(&settings2)
            .await
            .expect("second save should succeed");

        let loaded = store.load().await.expect("load should succeed");
        assert_eq!(
            loaded.display_commodity().as_str(),
            settings2.display_commodity().as_str()
        );

        // Confirm there is only one row
        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM meta")
            .fetch_one(&pool)
            .await
            .expect("count query should succeed");
        assert_eq!(count.0, 1, "upsert should not create duplicate rows");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn custom_display_commodity_round_trips(pool: sqlx::SqlitePool) {
        use bc_config::Settings;
        let store = Store::new(pool.clone());
        // Build a non-default settings using serde_json for simplicity
        let json = r#"{"financial_year_start_month":1,"financial_year_start_day":1,"fortnightly_anchor":null,"display_commodity":"USD"}"#;
        let settings: Settings = serde_json::from_str(json).expect("parse should succeed");
        store.save(&settings).await.expect("save should succeed");
        let loaded = store.load().await.expect("load should succeed");
        assert_eq!(loaded.display_commodity(), &CommodityCode::new("USD"));
    }
}
