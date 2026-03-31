//! Append-only event log and event types.

use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::EventId;
use bc_models::TransactionId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcResult;

/// All domain events produced by the core engine.
#[non_exhaustive]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum Event {
    /// A new account was created.
    AccountCreated {
        /// The new account's ID.
        id: AccountId,
        /// Display name of the new account.
        name: String,
        /// Classification in the chart of accounts.
        account_type: AccountType,
        /// Account maintenance kind.
        kind: AccountKind,
        /// Optional free-text description.
        description: Option<String>,
    },
    /// An account's metadata was updated.
    // TODO(M1): AccountUpdated must include the full new account state
    // (all mutable fields) before Service::update() is implemented.
    // AccountCreated correctly demonstrates the full-state pattern.
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
    ///
    /// Records the updated metadata fields (date, description, payee).
    /// Posting and tag mutations are applied directly to the projection tables
    /// and are not captured in this event payload.
    TransactionAmended {
        /// The transaction's ID.
        id: TransactionId,
        /// The new transaction date after amendment.
        date: jiff::civil::Date,
        /// The new description after amendment.
        description: String,
        /// The new payee after amendment, or `None` if the payee was removed.
        payee: Option<String>,
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
            | Self::TransactionAmended { id, .. }
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
pub struct SqliteStore {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

impl SqliteStore {
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

/// Inserts an event record within an existing database transaction.
///
/// Used by services that need to append an event atomically alongside
/// their own projection writes, sharing a single [`sqlx::SqliteConnection`].
///
/// # Arguments
///
/// * `event` - The event to insert.
/// * `conn` - An open, in-progress database transaction connection.
///
/// # Errors
///
/// Returns an error if serialisation or the database insert fails.
#[inline]
pub(crate) async fn insert_event(event: &Event, conn: &mut sqlx::SqliteConnection) -> BcResult<()> {
    let event_id = EventId::new().to_string();
    let kind = event.kind();
    let aggregate_id = event.aggregate_id();
    let payload = serde_json::to_string(event)?;
    let created_at = Timestamp::now().to_string();

    sqlx::query(
        "INSERT INTO events (id, kind, aggregate_id, payload, created_at) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&event_id)
    .bind(kind)
    .bind(&aggregate_id)
    .bind(&payload)
    .bind(&created_at)
    .execute(conn)
    .await?;

    tracing::debug!(%kind, %aggregate_id, "event appended");
    Ok(())
}

#[cfg(test)]
mod tests {
    use bc_models::AccountId;
    use pretty_assertions::assert_eq;

    use super::*;

    #[sqlx::test(migrations = "./migrations")]
    async fn append_and_replay_account_created(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        let store = SqliteStore::new(pool.clone());
        let id = AccountId::new();
        let event = Event::AccountCreated {
            id: id.clone(),
            name: "Test".to_owned(),
            account_type: AccountType::Asset,
            kind: AccountKind::DepositAccount,
            description: None,
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

    #[sqlx::test(migrations = "./migrations")]
    async fn replay_for_returns_events_in_insertion_order(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        let store = SqliteStore::new(pool.clone());
        let id = AccountId::new();

        store
            .append(&Event::AccountCreated {
                id: id.clone(),
                name: "Created".to_owned(),
                account_type: AccountType::Asset,
                kind: AccountKind::DepositAccount,
                description: None,
            })
            .await
            .expect("first append should succeed");
        store
            .append(&Event::AccountUpdated { id: id.clone() })
            .await
            .expect("second append should succeed");
        store
            .append(&Event::AccountArchived { id: id.clone() })
            .await
            .expect("third append should succeed");

        let records = store
            .replay_for(&id.to_string())
            .await
            .expect("replay should succeed");

        assert_eq!(records.len(), 3);
        assert_eq!(
            records.first().expect("first record should exist").kind,
            "AccountCreated"
        );
        assert_eq!(
            records.get(1).expect("second record should exist").kind,
            "AccountUpdated"
        );
        assert_eq!(
            records.get(2).expect("third record should exist").kind,
            "AccountArchived"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn replay_for_returns_empty_for_unknown_aggregate(pool: sqlx::SqlitePool) {
        let store = SqliteStore::new(pool.clone());
        let records = store
            .replay_for("account_nonexistent_id")
            .await
            .expect("replay should succeed");
        assert_eq!(records.len(), 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_amended_payload_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::TransactionId;
        use jiff::civil::Date;

        let store = SqliteStore::new(pool.clone());
        let id = TransactionId::new();
        let event = Event::TransactionAmended {
            id: id.clone(),
            date: Date::constant(2026, 3, 15),
            description: "Amended description".to_owned(),
            payee: Some("Woolworths".to_owned()),
        };

        store.append(&event).await.expect("append should succeed");

        let records = store
            .replay_for(&id.to_string())
            .await
            .expect("replay should succeed");
        let record = records.first().expect("one record should exist");
        assert_eq!(record.kind, "TransactionAmended");

        let replayed: Event =
            serde_json::from_str(&record.payload).expect("payload should deserialise");

        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm is required for exhaustive match in tests"
        )]
        match replayed {
            Event::TransactionAmended {
                id: replayed_id,
                date,
                description,
                payee,
            } => {
                assert_eq!(replayed_id, id);
                assert_eq!(date, Date::constant(2026, 3, 15));
                assert_eq!(description, "Amended description");
                assert_eq!(payee, Some("Woolworths".to_owned()));
            }
            other => panic!("expected TransactionAmended, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_amended_payload_round_trips_without_payee(pool: sqlx::SqlitePool) {
        use bc_models::TransactionId;
        use jiff::civil::Date;

        let store = SqliteStore::new(pool.clone());
        let id = TransactionId::new();
        let event = Event::TransactionAmended {
            id: id.clone(),
            date: Date::constant(2026, 1, 1),
            description: "No payee".to_owned(),
            payee: None,
        };

        store.append(&event).await.expect("append should succeed");

        let records = store
            .replay_for(&id.to_string())
            .await
            .expect("replay should succeed");
        let record = records.first().expect("one record should exist");

        let replayed: Event =
            serde_json::from_str(&record.payload).expect("payload should deserialise");

        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm is required for exhaustive match in tests"
        )]
        match replayed {
            Event::TransactionAmended { payee, .. } => {
                assert_eq!(payee, None);
            }
            other => panic!("expected TransactionAmended, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn account_created_payload_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        use bc_models::AccountType;

        let store = SqliteStore::new(pool.clone());
        let id = AccountId::new();
        let original = Event::AccountCreated {
            id: id.clone(),
            name: "Round-Trip Account".to_owned(),
            account_type: AccountType::Asset,
            kind: AccountKind::DepositAccount,
            description: Some("A test description".to_owned()),
        };

        store
            .append(&original)
            .await
            .expect("append should succeed");

        let records = store
            .replay_for(&id.to_string())
            .await
            .expect("replay should succeed");
        let record = records.first().expect("one record should exist");

        let replayed: Event =
            serde_json::from_str(&record.payload).expect("payload should deserialise");

        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm is required for exhaustive match in tests"
        )]
        match replayed {
            Event::AccountCreated {
                id: replayed_id,
                name,
                account_type,
                kind,
                description,
            } => {
                assert_eq!(replayed_id, id);
                assert_eq!(name, "Round-Trip Account");
                assert_eq!(account_type, AccountType::Asset);
                assert_eq!(kind, AccountKind::DepositAccount);
                assert_eq!(description, Some("A test description".to_owned()));
            }
            other => panic!("expected AccountCreated, got {other:?}"),
        }
    }
}
