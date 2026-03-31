//! Append-only event log and event types.

use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::AllocationId;
use bc_models::Amount;
use bc_models::DepreciationId;
use bc_models::EnvelopeGroupId;
use bc_models::EnvelopeId;
use bc_models::EventId;
use bc_models::LoanId;
use bc_models::Period;
use bc_models::RolloverPolicy;
use bc_models::TransactionId;
use bc_models::ValuationId;
use bc_models::ValuationSource;
use jiff::Timestamp;
use jiff::civil::Date;
use rust_decimal::Decimal;
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
    /// A point-in-time market value was recorded for a [`ManualAsset`] account.
    ///
    /// [`ManualAsset`]: bc_models::AccountKind::ManualAsset
    AssetValuationRecorded {
        /// Unique identifier for this valuation record.
        id: ValuationId,
        /// The account whose value was assessed.
        account_id: AccountId,
        /// Assessed market value (positive).
        market_value: Decimal,
        /// Commodity of the valuation (e.g. `"AUD"`).
        commodity: String,
        /// Source / authority for this valuation.
        source: ValuationSource,
        /// Business date of the assessment (not the insertion timestamp).
        recorded_at: Date,
    },
    /// A depreciation amount was calculated for a [`ManualAsset`] account.
    ///
    /// [`ManualAsset`]: bc_models::AccountKind::ManualAsset
    DepreciationCalculated {
        /// Unique identifier for this depreciation record.
        id: DepreciationId,
        /// The account being depreciated.
        account_id: AccountId,
        /// Depreciation amount (positive = asset value reduced by this amount).
        amount: Decimal,
        /// Commodity (e.g. `"AUD"`).
        commodity: String,
        /// Start of the depreciation period (inclusive).
        period_start: Date,
        /// End of the depreciation period (inclusive).
        period_end: Date,
    },
    /// Loan terms were set or updated for a [`Receivable`] account.
    ///
    /// **Note:** `compounding_frequency` and `offset_account_ids` are stored only in the
    /// `loan_terms` and `loan_offset_accounts` projection tables, not in this event.
    /// Event replay alone cannot recover these fields; the projection DB is canonical.
    ///
    /// [`Receivable`]: bc_models::AccountKind::Receivable
    LoanTermsSet {
        /// Unique identifier for this loan terms record.
        id: LoanId,
        /// The account these terms apply to.
        account_id: AccountId,
        /// Original principal amount.
        principal: Decimal,
        /// Annual interest rate as a fraction (e.g. `0.065` = 6.5 %).
        annual_rate: Decimal,
        /// Date the loan commenced.
        start_date: Date,
        /// Total term in months.
        term_months: u32,
        /// Repayment frequency.
        repayment_frequency: Period,
        /// Commodity of the loan (e.g. `"AUD"`).
        commodity: String,
    },
    /// A new envelope group was created.
    EnvelopeGroupCreated {
        /// The new group's ID.
        id: EnvelopeGroupId,
        /// Display name of the group.
        name: String,
        /// Parent group ID, if nested.
        group_id: Option<EnvelopeGroupId>,
    },
    /// A new budget envelope was created.
    EnvelopeCreated {
        /// The new envelope's ID.
        id: EnvelopeId,
        /// Display name.
        name: String,
        /// Group this envelope belongs to, if any.
        group_id: Option<EnvelopeGroupId>,
        /// Recurrence period.
        period: Period,
        /// Rollover policy.
        rollover_policy: RolloverPolicy,
        /// Budget target per period; `None` = category tracking mode.
        allocation_target: Option<Amount>,
    },
    /// Funds were allocated to an envelope for a period.
    EnvelopeAllocated {
        /// Allocation record ID (aggregate for replay).
        id: AllocationId,
        /// The envelope receiving the allocation.
        envelope_id: EnvelopeId,
        /// Canonical period start date.
        period_start: jiff::civil::Date,
        /// Amount allocated.
        amount: Amount,
    },
    /// An envelope was archived.
    EnvelopeArchived {
        /// The envelope's ID.
        id: EnvelopeId,
    },
    /// An envelope was moved to a different group (or to the root).
    EnvelopeMoved {
        /// The envelope's ID.
        id: EnvelopeId,
        /// New group ID, or `None` to place at the root.
        group_id: Option<EnvelopeGroupId>,
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
            Self::AssetValuationRecorded { .. } => "AssetValuationRecorded",
            Self::DepreciationCalculated { .. } => "DepreciationCalculated",
            Self::LoanTermsSet { .. } => "LoanTermsSet",
            Self::EnvelopeGroupCreated { .. } => "EnvelopeGroupCreated",
            Self::EnvelopeCreated { .. } => "EnvelopeCreated",
            Self::EnvelopeAllocated { .. } => "EnvelopeAllocated",
            Self::EnvelopeArchived { .. } => "EnvelopeArchived",
            Self::EnvelopeMoved { .. } => "EnvelopeMoved",
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
            // Asset/loan events belong to the account aggregate: `account_id` is the
            // aggregate root, so it is used as the aggregate ID rather than the entity's
            // own ID (`valuation_id`, `depreciation_id`, `loan_id`). This differs from
            // transaction events, which use their own `id` as the aggregate ID.
            Self::AssetValuationRecorded { account_id, .. }
            | Self::DepreciationCalculated { account_id, .. }
            | Self::LoanTermsSet { account_id, .. } => account_id.to_string(),
            Self::EnvelopeGroupCreated { id, .. } => id.to_string(),
            Self::EnvelopeCreated { id, .. }
            | Self::EnvelopeArchived { id }
            | Self::EnvelopeMoved { id, .. } => id.to_string(),
            Self::EnvelopeAllocated { id, .. } => id.to_string(),
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
    async fn asset_valuation_recorded_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::ValuationId;
        use bc_models::ValuationSource;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let store = SqliteStore::new(pool.clone());
        let account_id = AccountId::new();
        let event = Event::AssetValuationRecorded {
            id: ValuationId::new(),
            account_id: account_id.clone(),
            market_value: dec!(650_000),
            commodity: "AUD".to_owned(),
            source: ValuationSource::ProfessionalAppraisal,
            recorded_at: date(2026, 3, 31),
        };

        store.append(&event).await.expect("append should succeed");
        let records = store
            .replay_for(&account_id.to_string())
            .await
            .expect("replay");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records.first().expect("one record").kind,
            "AssetValuationRecorded"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn depreciation_calculated_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::DepreciationId;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let store = SqliteStore::new(pool.clone());
        let account_id = AccountId::new();
        let event = Event::DepreciationCalculated {
            id: DepreciationId::new(),
            account_id: account_id.clone(),
            amount: dec!(16_250),
            commodity: "AUD".to_owned(),
            period_start: date(2026, 1, 1),
            period_end: date(2026, 3, 31),
        };

        store.append(&event).await.expect("append");
        let records = store
            .replay_for(&account_id.to_string())
            .await
            .expect("replay");
        assert_eq!(records.first().expect("one").kind, "DepreciationCalculated");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn loan_terms_set_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::LoanId;
        use bc_models::Period;
        use jiff::civil::date;
        use rust_decimal_macros::dec;

        let store = SqliteStore::new(pool.clone());
        let account_id = AccountId::new();
        let event = Event::LoanTermsSet {
            id: LoanId::new(),
            account_id: account_id.clone(),
            principal: dec!(100_000),
            annual_rate: dec!(0.065),
            start_date: date(2026, 1, 1),
            term_months: 360,
            repayment_frequency: Period::Monthly,
            commodity: "AUD".to_owned(),
        };

        store.append(&event).await.expect("append");
        let records = store
            .replay_for(&account_id.to_string())
            .await
            .expect("replay");
        assert_eq!(records.first().expect("one").kind, "LoanTermsSet");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn envelope_created_payload_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::EnvelopeId;
        use bc_models::Period;
        use bc_models::RolloverPolicy;

        let store = SqliteStore::new(pool.clone());
        let id = EnvelopeId::new();
        let event = Event::EnvelopeCreated {
            id: id.clone(),
            name: "Groceries".to_owned(),
            group_id: None,
            period: Period::Monthly,
            rollover_policy: RolloverPolicy::CarryForward,
            allocation_target: None,
        };

        store.append(&event).await.expect("append should succeed");

        let records = store
            .replay_for(&id.to_string())
            .await
            .expect("replay should succeed");
        assert_eq!(records.len(), 1);
        assert_eq!(
            records.first().expect("record should exist").kind,
            "EnvelopeCreated"
        );

        let replayed: Event = serde_json::from_str(&records.first().expect("record").payload)
            .expect("payload should deserialise");

        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm is required for exhaustive match in tests"
        )]
        match replayed {
            Event::EnvelopeCreated {
                id: replayed_id,
                name,
                group_id,
                period,
                rollover_policy,
                allocation_target,
            } => {
                assert_eq!(replayed_id, id);
                assert_eq!(name, "Groceries");
                assert_eq!(group_id, None);
                assert_eq!(period, Period::Monthly);
                assert_eq!(rollover_policy, RolloverPolicy::CarryForward);
                assert_eq!(allocation_target, None);
            }
            other => panic!("expected EnvelopeCreated, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn envelope_allocated_payload_round_trips(pool: sqlx::SqlitePool) {
        use bc_models::AllocationId;
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use bc_models::EnvelopeId;
        use jiff::civil::Date;

        let store = SqliteStore::new(pool.clone());
        let alloc_id = AllocationId::new();
        let env_id = EnvelopeId::new();
        let event = Event::EnvelopeAllocated {
            id: alloc_id.clone(),
            envelope_id: env_id.clone(),
            period_start: Date::constant(2026, 3, 1),
            amount: Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD")),
        };

        store.append(&event).await.expect("append should succeed");

        let records = store
            .replay_for(&alloc_id.to_string())
            .await
            .expect("replay should succeed");
        assert_eq!(records.first().expect("record").kind, "EnvelopeAllocated");

        let replayed: Event = serde_json::from_str(&records.first().expect("record").payload)
            .expect("payload should deserialise");

        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm is required for exhaustive match in tests"
        )]
        match replayed {
            Event::EnvelopeAllocated {
                id: replayed_id,
                envelope_id: replayed_env_id,
                period_start,
                amount: replayed_amount,
            } => {
                assert_eq!(replayed_id, alloc_id);
                assert_eq!(replayed_env_id, env_id);
                assert_eq!(period_start, Date::constant(2026, 3, 1));
                assert_eq!(
                    replayed_amount,
                    Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD"))
                );
            }
            other => panic!("expected EnvelopeAllocated, got {other:?}"),
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
