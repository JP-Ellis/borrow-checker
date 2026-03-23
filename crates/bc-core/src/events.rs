//! Append-only event log and event types.

use bc_models::{AccountId, EventId, ids::TransactionId};
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::error::BcResult;

/// All domain events produced by the core engine.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum Event {
    /// A new account was created.
    AccountCreated {
        /// The new account's ID.
        id: AccountId,
        /// Display name.
        name: String,
    },
    /// An account's metadata was updated.
    AccountUpdated {
        /// The account's ID.
        id: AccountId,
    },
    /// An account was archived.
    AccountArchived {
        /// The account's ID.
        id: AccountId,
    },
    /// A new transaction was recorded.
    TransactionCreated {
        /// The new transaction's ID.
        id: TransactionId,
    },
    /// A transaction was amended.
    TransactionAmended {
        /// The transaction's ID.
        id: TransactionId,
    },
    /// A transaction was voided.
    TransactionVoided {
        /// The transaction's ID.
        id: TransactionId,
    },
}

impl Event {
    /// Returns the string kind tag for this event (used as a DB discriminator).
    #[must_use]
    #[inline]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AccountCreated { .. } => "AccountCreated",
            Self::AccountUpdated { .. } => "AccountUpdated",
            Self::AccountArchived { .. } => "AccountArchived",
            Self::TransactionCreated { .. } => "TransactionCreated",
            Self::TransactionAmended { .. } => "TransactionAmended",
            Self::TransactionVoided { .. } => "TransactionVoided",
        }
    }

    /// Returns the aggregate ID this event belongs to.
    #[must_use]
    #[inline]
    pub fn aggregate_id(&self) -> String {
        match self {
            Self::AccountCreated { id, .. }
            | Self::AccountUpdated { id }
            | Self::AccountArchived { id } => id.to_string(),
            Self::TransactionCreated { id }
            | Self::TransactionAmended { id }
            | Self::TransactionVoided { id } => id.to_string(),
        }
    }
}

/// A raw event record as stored in the `events` table.
#[non_exhaustive]
#[derive(Debug, sqlx::FromRow)]
pub struct EventRecord {
    /// Event ID.
    pub id: String,
    /// Event kind tag.
    pub kind: String,
    /// ID of the affected aggregate.
    pub aggregate_id: String,
    /// JSON-encoded event payload.
    pub payload: String,
    /// When the event was appended (RFC 3339).
    pub created_at: String,
}

/// An append-only event store backed by SQLite.
#[derive(Debug, Clone)]
pub struct SqliteEventStore {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl SqliteEventStore {
    /// Creates a new event store using the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Appends an event to the log.
    ///
    /// # Errors
    ///
    /// Returns an error if serialisation or the database insert fails.
    #[inline]
    pub async fn append(&self, event: &Event) -> BcResult<()> {
        let event_id = EventId::new().to_string();
        let kind = event.kind();
        let aggregate_id = event.aggregate_id();
        let payload = serde_json::to_string(event)?;
        let created_at = Timestamp::now().to_string();

        sqlx::query(
            "INSERT INTO events (id, kind, aggregate_id, payload, created_at) VALUES (?, ?, ?, ?, ?)"
        )
        .bind(&event_id)
        .bind(kind)
        .bind(&aggregate_id)
        .bind(&payload)
        .bind(&created_at)
        .execute(&self.pool)
        .await?;

        tracing::debug!(%kind, %aggregate_id, "event appended");
        Ok(())
    }

    /// Returns all events for a given aggregate ID in insertion order.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    #[inline]
    pub async fn replay_for(&self, aggregate_id: &str) -> BcResult<Vec<EventRecord>> {
        let records = sqlx::query_as::<_, EventRecord>(
            "SELECT id, kind, aggregate_id, payload, created_at FROM events WHERE aggregate_id = ? ORDER BY rowid ASC"
        )
        .bind(aggregate_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use pretty_assertions::assert_eq;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn append_and_replay_account_created(pool: sqlx::SqlitePool) {
        let store = SqliteEventStore::new(pool.clone());
        let id = AccountId::new();
        let event = Event::AccountCreated {
            id: id.clone(),
            name: "Test".to_owned(),
        };

        store.append(&event).await.expect("append should succeed");

        let records = store
            .replay_for(&id.to_string())
            .await
            .expect("replay should succeed");
        assert_eq!(records.len(), 1);
        let first = records.first().expect("records should be non-empty");
        assert_eq!(first.kind, "AccountCreated");
    }
}
