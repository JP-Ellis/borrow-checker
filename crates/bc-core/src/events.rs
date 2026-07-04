//! Append-only event log and event types.

use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::Amount;
use bc_models::BudgetId;
use bc_models::BudgetRevisionId;
use bc_models::DepreciationId;
use bc_models::EventId;
use bc_models::LoanId;
use bc_models::Period;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::RolloverPolicy;
use bc_models::SourceRefId;
use bc_models::TagId;
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
    /// A reversal transaction was created for an existing transaction.
    TransactionReversed {
        /// The original transaction's ID.
        original_id: TransactionId,
        /// The new reversal transaction's ID.
        reversal_id: TransactionId,
    },
    /// A transaction's payee was changed.
    TransactionPayeeChanged {
        /// The transaction's ID.
        id: TransactionId,
        /// Payee before the change, or `None` if previously unset.
        from: Option<String>,
        /// Payee after the change, or `None` if cleared.
        to: Option<String>,
    },
    /// A transaction's date was changed.
    TransactionDateChanged {
        /// The transaction's ID.
        id: TransactionId,
        /// Date before the change.
        from: jiff::civil::Date,
        /// Date after the change.
        to: jiff::civil::Date,
    },
    /// A transaction's description was changed.
    TransactionDescriptionChanged {
        /// The transaction's ID.
        id: TransactionId,
        /// Description before the change.
        from: String,
        /// Description after the change.
        to: String,
    },
    /// A transaction's note was changed.
    TransactionNoteChanged {
        /// The transaction's ID.
        id: TransactionId,
        /// Note before the change, or `None` if previously unset.
        from: Option<String>,
        /// Note after the change, or `None` if cleared.
        to: Option<String>,
    },
    /// A transaction's tag set was changed.
    TransactionTagsChanged {
        /// The transaction's ID.
        id: TransactionId,
        /// Tags added in this change.
        added: Vec<TagId>,
        /// Tags removed in this change.
        removed: Vec<TagId>,
    },
    /// A transaction's extra (labeled) dates were changed.
    TransactionExtraDatesChanged {
        /// The transaction's ID.
        id: TransactionId,
        /// The full extra-date set before the change.
        from: Vec<(String, jiff::civil::Date)>,
        /// The full extra-date set after the change.
        to: Vec<(String, jiff::civil::Date)>,
    },
    /// A transaction's reconciliation state was changed.
    TransactionReconciled {
        /// The transaction's ID.
        id: TransactionId,
        /// Reconciliation state before the change.
        from: Reconciliation,
        /// Reconciliation state after the change.
        to: Reconciliation,
    },
    /// A posting was moved to a different account (recategorised).
    PostingRecategorised {
        /// The owning transaction's ID.
        id: TransactionId,
        /// The posting that moved.
        posting_id: PostingId,
        /// Account before the change.
        from_account: AccountId,
        /// Account after the change.
        to_account: AccountId,
    },
    /// A posting's amount was changed.
    PostingAmountChanged {
        /// The owning transaction's ID.
        id: TransactionId,
        /// The posting whose amount changed.
        posting_id: PostingId,
        /// Amount before the change, or `None` if previously elided.
        from: Option<Amount>,
        /// Amount after the change, or `None` if now elided.
        to: Option<Amount>,
    },
    /// A posting's note was changed.
    PostingNoteChanged {
        /// The owning transaction's ID.
        id: TransactionId,
        /// The posting whose note changed.
        posting_id: PostingId,
        /// Note before the change, or `None` if previously unset.
        from: Option<String>,
        /// Note after the change, or `None` if cleared.
        to: Option<String>,
    },
    /// A posting's accrual spread window was changed.
    PostingSpreadChanged {
        /// The owning transaction's ID.
        id: TransactionId,
        /// The posting whose spread changed.
        posting_id: PostingId,
        /// Spread `(from, until)` before the change, or `None` if unset.
        from: Option<(jiff::civil::Date, jiff::civil::Date)>,
        /// Spread `(from, until)` after the change, or `None` if cleared.
        to: Option<(jiff::civil::Date, jiff::civil::Date)>,
    },
    /// A posting (leg) was added to a transaction (also covers splits).
    PostingAdded {
        /// The owning transaction's ID.
        id: TransactionId,
        /// The new posting's ID.
        posting_id: PostingId,
        /// The account the new posting hits.
        account: AccountId,
        /// The new posting's amount, or `None` if elided.
        amount: Option<Amount>,
    },
    /// A posting (leg) was removed from a transaction.
    PostingRemoved {
        /// The owning transaction's ID.
        id: TransactionId,
        /// The removed posting's ID.
        posting_id: PostingId,
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
    /// A budget anchor was created together with its initial revision.
    BudgetCreated {
        /// The new anchor's ID.
        budget_id: BudgetId,
        /// Account the budget is anchored to.
        account_id: AccountId,
        /// When the anchor was created.
        created_at: Timestamp,
        /// The initial revision's ID.
        revision_id: BudgetRevisionId,
        /// The initial revision's effective-from date.
        effective_from: jiff::civil::Date,
        /// Display name (initial revision).
        name: Option<String>,
        /// Target amount (initial revision).
        target: Option<Amount>,
        /// Period (initial revision).
        period: Period,
        /// Rollover policy (initial revision).
        rollover: RolloverPolicy,
        /// Tag filter (initial revision).
        tag_filter: Option<TagId>,
    },
    /// A revision was added or amended (upsert by `revision_id`).
    BudgetRevisionSet {
        /// Anchor this revision belongs to.
        budget_id: BudgetId,
        /// Revision being set.
        revision_id: BudgetRevisionId,
        /// Effective-from date.
        effective_from: jiff::civil::Date,
        /// Display name.
        name: Option<String>,
        /// Target amount.
        target: Option<Amount>,
        /// Period.
        period: Period,
        /// Rollover policy.
        rollover: RolloverPolicy,
        /// Tag filter.
        tag_filter: Option<TagId>,
    },
    /// A revision was removed.
    BudgetRevisionRemoved {
        /// Anchor this revision belonged to.
        budget_id: BudgetId,
        /// Revision removed.
        revision_id: BudgetRevisionId,
    },
    /// A budget was archived.
    BudgetArchived {
        /// The budget's ID.
        budget_id: BudgetId,
        /// When the budget was archived.
        archived_at: jiff::Timestamp,
    },
    /// A source reference (import provenance) was attached to a transaction.
    TransactionSourceAttached {
        /// The new source reference's ID.
        id: SourceRefId,
        /// The transaction this row produced.
        transaction_id: TransactionId,
        /// The account whose statement produced this row.
        account_id: AccountId,
        /// Value date of the row.
        date: jiff::civil::Date,
        /// Raw imported narration.
        narration: String,
        /// Statement amount.
        amount: Amount,
        /// Institution reference, if any.
        reference: Option<String>,
        /// Occurrence ordinal among identical fingerprints.
        occurrence: u32,
    },
    /// A source reference was detached from a transaction.
    TransactionSourceDetached {
        /// The detached source reference's ID.
        id: SourceRefId,
        /// The transaction it belonged to.
        transaction_id: TransactionId,
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
            Self::TransactionReversed { .. } => "TransactionReversed",
            Self::TransactionPayeeChanged { .. } => "TransactionPayeeChanged",
            Self::TransactionDateChanged { .. } => "TransactionDateChanged",
            Self::TransactionDescriptionChanged { .. } => "TransactionDescriptionChanged",
            Self::TransactionNoteChanged { .. } => "TransactionNoteChanged",
            Self::TransactionTagsChanged { .. } => "TransactionTagsChanged",
            Self::TransactionExtraDatesChanged { .. } => "TransactionExtraDatesChanged",
            Self::TransactionReconciled { .. } => "TransactionReconciled",
            Self::PostingRecategorised { .. } => "PostingRecategorised",
            Self::PostingAmountChanged { .. } => "PostingAmountChanged",
            Self::PostingNoteChanged { .. } => "PostingNoteChanged",
            Self::PostingSpreadChanged { .. } => "PostingSpreadChanged",
            Self::PostingAdded { .. } => "PostingAdded",
            Self::PostingRemoved { .. } => "PostingRemoved",
            Self::AssetValuationRecorded { .. } => "AssetValuationRecorded",
            Self::DepreciationCalculated { .. } => "DepreciationCalculated",
            Self::LoanTermsSet { .. } => "LoanTermsSet",
            Self::BudgetCreated { .. } => "BudgetCreated",
            Self::BudgetRevisionSet { .. } => "BudgetRevisionSet",
            Self::BudgetRevisionRemoved { .. } => "BudgetRevisionRemoved",
            Self::BudgetArchived { .. } => "BudgetArchived",
            Self::TransactionSourceAttached { .. } => "TransactionSourceAttached",
            Self::TransactionSourceDetached { .. } => "TransactionSourceDetached",
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
            | Self::TransactionVoided { id }
            | Self::TransactionPayeeChanged { id, .. }
            | Self::TransactionDateChanged { id, .. }
            | Self::TransactionDescriptionChanged { id, .. }
            | Self::TransactionNoteChanged { id, .. }
            | Self::TransactionTagsChanged { id, .. }
            | Self::TransactionExtraDatesChanged { id, .. }
            | Self::TransactionReconciled { id, .. }
            | Self::PostingRecategorised { id, .. }
            | Self::PostingAmountChanged { id, .. }
            | Self::PostingNoteChanged { id, .. }
            | Self::PostingSpreadChanged { id, .. }
            | Self::PostingAdded { id, .. }
            | Self::PostingRemoved { id, .. } => id.to_string(),
            Self::TransactionReversed { original_id, .. } => original_id.to_string(),
            // Asset/loan events belong to the account aggregate: `account_id` is the
            // aggregate root, so it is used as the aggregate ID rather than the entity's
            // own ID (`valuation_id`, `depreciation_id`, `loan_id`). This differs from
            // transaction events, which use their own `id` as the aggregate ID.
            Self::AssetValuationRecorded { account_id, .. }
            | Self::DepreciationCalculated { account_id, .. }
            | Self::LoanTermsSet { account_id, .. } => account_id.to_string(),
            Self::BudgetCreated { budget_id, .. }
            | Self::BudgetRevisionSet { budget_id, .. }
            | Self::BudgetRevisionRemoved { budget_id, .. }
            | Self::BudgetArchived { budget_id, .. } => budget_id.to_string(),
            Self::TransactionSourceAttached { transaction_id, .. }
            | Self::TransactionSourceDetached { transaction_id, .. } => transaction_id.to_string(),
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
    use bc_models::CommodityCode;
    use bc_models::SourceRefId;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;

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

    #[test]
    fn budget_created_round_trips() {
        let budget_id = BudgetId::new();
        let event = Event::BudgetCreated {
            budget_id: budget_id.clone(),
            account_id: bc_models::AccountId::new(),
            created_at: jiff::Timestamp::now(),
            revision_id: bc_models::BudgetRevisionId::new(),
            effective_from: jiff::civil::Date::constant(2026, 1, 1),
            name: Some("Groceries".to_owned()),
            target: None,
            period: bc_models::Period::Weekly,
            rollover: bc_models::RolloverPolicy::ResetToZero,
            tag_filter: None,
        };
        let json = serde_json::to_string(&event).expect("ser");
        let back: Event = serde_json::from_str(&json).expect("de");
        assert_eq!(back.kind(), "BudgetCreated");
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm required"
        )]
        match back {
            Event::BudgetCreated {
                name,
                effective_from,
                ..
            } => {
                assert_eq!(name, Some("Groceries".to_owned()));
                assert_eq!(effective_from, jiff::civil::Date::constant(2026, 1, 1));
            }
            other => panic!("expected BudgetCreated, got {other:?}"),
        }
    }

    #[test]
    fn budget_revision_set_round_trips() {
        let event = Event::BudgetRevisionSet {
            budget_id: BudgetId::new(),
            revision_id: bc_models::BudgetRevisionId::new(),
            effective_from: jiff::civil::Date::constant(2027, 1, 1),
            name: None,
            target: Some(bc_models::Amount::new(
                bc_models::Decimal::from(250_i32),
                bc_models::CommodityCode::new("AUD"),
            )),
            period: bc_models::Period::Weekly,
            rollover: bc_models::RolloverPolicy::CarryForward,
            tag_filter: None,
        };
        let json = serde_json::to_string(&event).expect("ser");
        let back: Event = serde_json::from_str(&json).expect("de");
        assert_eq!(back.kind(), "BudgetRevisionSet");
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm required"
        )]
        match back {
            Event::BudgetRevisionSet {
                effective_from,
                target,
                ..
            } => {
                assert_eq!(effective_from, jiff::civil::Date::constant(2027, 1, 1));
                assert_eq!(
                    target,
                    Some(bc_models::Amount::new(
                        bc_models::Decimal::from(250_i32),
                        bc_models::CommodityCode::new("AUD"),
                    ))
                );
            }
            other => panic!("expected BudgetRevisionSet, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn budget_archived_round_trips(pool: sqlx::SqlitePool) {
        let store = SqliteStore::new(pool.clone());
        let budget_id = BudgetId::new();
        let event = Event::BudgetArchived {
            budget_id: budget_id.clone(),
            archived_at: Timestamp::now(),
        };

        store.append(&event).await.expect("append");
        let records = store
            .replay_for(&budget_id.to_string())
            .await
            .expect("replay");
        assert_eq!(records.first().expect("one").kind, "BudgetArchived");
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

    #[test]
    fn posting_recategorised_round_trips() {
        use bc_models::PostingId;

        let event = Event::PostingRecategorised {
            id: TransactionId::new(),
            posting_id: PostingId::new(),
            from_account: AccountId::new(),
            to_account: AccountId::new(),
        };
        let json = serde_json::to_string(&event).expect("serialise");
        let back: Event = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(event.kind(), back.kind());
        assert_eq!(event.kind(), "PostingRecategorised");
    }

    #[test]
    fn transaction_reconciled_round_trips() {
        use bc_models::Reconciliation;

        let tx = TransactionId::new();
        let event = Event::TransactionReconciled {
            id: tx.clone(),
            from: Reconciliation::Unreconciled,
            to: Reconciliation::Reconciled,
        };
        let json = serde_json::to_string(&event).expect("serialise");
        let back: Event = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.kind(), "TransactionReconciled");
        assert_eq!(back.aggregate_id(), tx.to_string());
        #[expect(
            clippy::wildcard_enum_match_arm,
            reason = "Event is #[non_exhaustive]; wildcard arm required"
        )]
        match back {
            Event::TransactionReconciled { from, to, .. } => {
                assert_eq!(from, Reconciliation::Unreconciled);
                assert_eq!(to, Reconciliation::Reconciled);
            }
            other => panic!("expected TransactionReconciled, got {other:?}"),
        }
    }

    #[test]
    fn decomposed_events_aggregate_on_transaction_id() {
        let tx = TransactionId::new();
        let event = Event::TransactionPayeeChanged {
            id: tx.clone(),
            from: Some("Old".to_owned()),
            to: Some("New".to_owned()),
        };
        assert_eq!(event.aggregate_id(), tx.to_string());
        assert_eq!(event.kind(), "TransactionPayeeChanged");
    }

    #[test]
    fn source_attached_aggregates_on_transaction_id() {
        let tx = TransactionId::new();
        let event = Event::TransactionSourceAttached {
            id: SourceRefId::new(),
            transaction_id: tx.clone(),
            account_id: AccountId::new(),
            date: date(2025, 6, 27),
            narration: "ACME".to_owned(),
            amount: Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
            reference: None,
            occurrence: 0,
        };
        assert_eq!(event.kind(), "TransactionSourceAttached");
        assert_eq!(event.aggregate_id(), tx.to_string());
    }
}
