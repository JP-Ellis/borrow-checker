//! Envelope and allocation service.

use std::collections::HashMap;

use bc_models::AccountId;
use bc_models::Allocation;
use bc_models::AllocationId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::Envelope;
use bc_models::EnvelopeGroup;
use bc_models::EnvelopeGroupId;
use bc_models::EnvelopeId;
use bc_models::Period;
use bc_models::RolloverPolicy;
use jiff::Timestamp;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;

/// Internal row type returned from the `envelopes` table, mapped by `sqlx::FromRow`.
///
/// The `account_ids` field is populated separately via a join query after the initial fetch.
#[derive(sqlx::FromRow)]
struct EnvelopeRow {
    /// Raw envelope ID string.
    id: String,
    /// Display name.
    name: String,
    /// Raw parent group ID string, if this envelope is in a group.
    parent_id: Option<String>,
    /// Optional icon identifier.
    icon: Option<String>,
    /// Optional colour code.
    colour: Option<String>,
    /// Commodity code this envelope is denominated in.
    commodity: String,
    /// Decimal string for the allocation target amount; `None` = tracking-only mode.
    allocation_target_amount: Option<String>,
    /// Commodity code for the allocation target; `None` when amount is `None`.
    allocation_target_commodity: Option<String>,
    /// JSON-serialised [`Period`].
    period: String,
    /// Snake-case rollover policy string.
    rollover_policy: String,
    /// ISO 8601 creation timestamp.
    created_at: String,
    /// Linked account IDs — populated after the initial query.
    #[sqlx(skip)]
    account_ids: Vec<AccountId>,
}

impl TryFrom<EnvelopeRow> for Envelope {
    type Error = BcError;

    /// Converts a raw database row into a domain [`Envelope`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    #[inline]
    fn try_from(row: EnvelopeRow) -> BcResult<Self> {
        let id = row
            .id
            .parse::<EnvelopeId>()
            .map_err(|e| BcError::BadData(format!("invalid envelope id '{}': {e}", row.id)))?;

        let parent_id = row
            .parent_id
            .as_deref()
            .map(|s| {
                s.parse::<EnvelopeGroupId>()
                    .map_err(|e| BcError::BadData(format!("invalid parent_id '{s}': {e}")))
            })
            .transpose()?;

        let allocation_target = match (
            row.allocation_target_amount,
            row.allocation_target_commodity,
        ) {
            (Some(amt_str), Some(com_str)) => {
                let quantity = amt_str.parse::<Decimal>().map_err(|e| {
                    BcError::BadData(format!("invalid allocation_target_amount '{amt_str}': {e}"))
                })?;
                let commodity = CommodityCode::new(&com_str);
                Some(Amount::new(quantity, commodity))
            }
            (None, None) => None,
            _ => {
                return Err(BcError::BadData(
                    "allocation_target_amount and allocation_target_commodity \
                     must both be set or both NULL"
                        .to_owned(),
                ));
            }
        };

        let period: Period = serde_json::from_str(&row.period)
            .map_err(|e| BcError::BadData(format!("invalid period '{}': {e}", row.period)))?;

        let rollover_policy = from_db_str::<RolloverPolicy>(&row.rollover_policy)?;

        let created_at = row.created_at.parse::<Timestamp>().map_err(|e| {
            BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;

        let commodity = CommodityCode::new(&row.commodity);

        Ok(Self::builder()
            .id(id)
            .name(row.name)
            .maybe_parent_id(parent_id)
            .maybe_icon(row.icon)
            .maybe_colour(row.colour)
            .commodity(commodity)
            .maybe_allocation_target(allocation_target)
            .period(period)
            .rollover_policy(rollover_policy)
            .account_ids(row.account_ids)
            .created_at(created_at)
            .build())
    }
}

/// Parameters for creating a new envelope.
///
/// Passed to [`Service::create`].
#[non_exhaustive]
pub struct CreateParams {
    /// Display name for the envelope.
    pub name: String,
    /// Group this envelope belongs to, if any.
    pub group_id: Option<EnvelopeGroupId>,
    /// Optional display icon (emoji or icon name).
    pub icon: Option<String>,
    /// Optional display colour (e.g. `"#4CAF50"`).
    pub colour: Option<String>,
    /// Commodity (currency) this envelope is denominated in.
    ///
    /// When [`CreateParams::allocation_target`] is set, its commodity **must**
    /// match this field or [`Service::create`] returns [`BcError::InvalidInput`].
    pub commodity: CommodityCode,
    /// Budget target per period. `None` = category tracking mode.
    pub allocation_target: Option<Amount>,
    /// Recurrence period.
    pub period: Period,
    /// How unspent funds roll between periods.
    pub rollover_policy: RolloverPolicy,
    /// Accounts linked for UI hints.
    pub account_ids: Vec<AccountId>,
}

impl CreateParams {
    /// Creates a new [`CreateParams`] with all required and optional fields.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name for the envelope.
    /// * `group_id` - Group this envelope belongs to, if any.
    /// * `icon` - Optional display icon (emoji or icon name).
    /// * `colour` - Optional display colour (e.g. `"#4CAF50"`).
    /// * `commodity` - Commodity (currency) code for this envelope.
    /// * `allocation_target` - Budget target per period. `None` = category tracking mode.
    /// * `period` - Recurrence period.
    /// * `rollover_policy` - How unspent funds roll between periods.
    /// * `account_ids` - Accounts linked for UI hints.
    #[inline]
    #[must_use]
    #[expect(
        clippy::too_many_arguments,
        reason = "all fields are distinct configuration values"
    )]
    pub fn new(
        name: String,
        group_id: Option<EnvelopeGroupId>,
        icon: Option<String>,
        colour: Option<String>,
        commodity: CommodityCode,
        allocation_target: Option<Amount>,
        period: Period,
        rollover_policy: RolloverPolicy,
        account_ids: Vec<AccountId>,
    ) -> Self {
        Self {
            name,
            group_id,
            icon,
            colour,
            commodity,
            allocation_target,
            period,
            rollover_policy,
            account_ids,
        }
    }
}

/// Envelope group + envelope CRUD service.
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

    // ── Group management ──────────────────────────────────────────────────

    /// Creates a new envelope group.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create_group(
        &self,
        name: &str,
        parent_id: Option<&EnvelopeGroupId>,
    ) -> BcResult<EnvelopeGroup> {
        let id = EnvelopeGroupId::new();
        let now = Timestamp::now();
        let event = Event::EnvelopeGroupCreated {
            id: id.clone(),
            name: name.to_owned(),
            parent_id: parent_id.cloned(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO envelope_groups (id, name, parent_id, created_at) VALUES (?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(name)
        .bind(parent_id.map(ToString::to_string))
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(group_id = %id, %name, "envelope group created");

        Ok(EnvelopeGroup::builder()
            .id(id)
            .name(name)
            .maybe_parent_id(parent_id.cloned())
            .created_at(now)
            .build())
    }

    /// Lists all active (non-archived) envelope groups, ordered by name.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_groups(&self) -> BcResult<Vec<EnvelopeGroup>> {
        let rows: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
            "SELECT id, name, parent_id, created_at \
             FROM envelope_groups \
             WHERE archived_at IS NULL \
             ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        rows.into_iter()
            .map(|(id_str, name, parent_str, created_str)| {
                let id = id_str
                    .parse::<EnvelopeGroupId>()
                    .map_err(|e| BcError::BadData(format!("invalid group id '{id_str}': {e}")))?;
                let parent_id = parent_str
                    .as_deref()
                    .map(|s| {
                        s.parse::<EnvelopeGroupId>()
                            .map_err(|e| BcError::BadData(format!("invalid parent_id '{s}': {e}")))
                    })
                    .transpose()?;
                let created_at = created_str.parse::<Timestamp>().map_err(|e| {
                    BcError::BadData(format!("invalid created_at '{created_str}': {e}"))
                })?;
                Ok(EnvelopeGroup::builder()
                    .id(id)
                    .name(name)
                    .maybe_parent_id(parent_id)
                    .created_at(created_at)
                    .build())
            })
            .collect()
    }

    // ── Envelope management ───────────────────────────────────────────────

    /// Creates a new budget envelope.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database insert failure.
    #[inline]
    pub async fn create(&self, params: CreateParams) -> BcResult<Envelope> {
        if params.rollover_policy == RolloverPolicy::CapAtTarget
            && params.allocation_target.is_none()
        {
            return Err(BcError::InvalidInput(
                "CapAtTarget rollover policy requires an allocation target".to_owned(),
            ));
        }

        if let Some(target) = &params.allocation_target {
            if target.commodity() != &params.commodity {
                return Err(BcError::InvalidInput(format!(
                    "allocation_target commodity '{}' does not match envelope commodity '{}'",
                    target.commodity(),
                    params.commodity,
                )));
            }
        }

        let id = EnvelopeId::new();
        let now = Timestamp::now();

        let event = Event::EnvelopeCreated {
            id: id.clone(),
            name: params.name.clone(),
            group_id: params.group_id.clone(),
            period: params.period.clone(),
            rollover_policy: params.rollover_policy,
            allocation_target: params.allocation_target.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        let period_json = serde_json::to_string(&params.period)?;
        let rollover_db = to_db_str(params.rollover_policy)?;
        let (target_amount, target_commodity) =
            params.allocation_target.as_ref().map_or((None, None), |a| {
                (Some(a.value().to_string()), Some(a.commodity().to_string()))
            });

        sqlx::query(
            "INSERT INTO envelopes \
             (id, name, parent_id, icon, colour, commodity, \
              allocation_target_amount, allocation_target_commodity, \
              period, rollover_policy, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(&params.name)
        .bind(params.group_id.as_ref().map(ToString::to_string))
        .bind(&params.icon)
        .bind(&params.colour)
        .bind(params.commodity.as_str())
        .bind(&target_amount)
        .bind(&target_commodity)
        .bind(&period_json)
        .bind(&rollover_db)
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        for account_id in &params.account_ids {
            sqlx::query(
                "INSERT INTO envelope_account_links (envelope_id, account_id) VALUES (?, ?)",
            )
            .bind(id.to_string())
            .bind(account_id.to_string())
            .execute(&mut *db_tx)
            .await?;
        }

        db_tx.commit().await?;
        tracing::info!(envelope_id = %id, name = %params.name, "envelope created");

        Ok(Envelope::builder()
            .id(id)
            .name(params.name)
            .maybe_parent_id(params.group_id)
            .maybe_icon(params.icon)
            .maybe_colour(params.colour)
            .commodity(params.commodity)
            .maybe_allocation_target(params.allocation_target)
            .period(params.period)
            .rollover_policy(params.rollover_policy)
            .account_ids(params.account_ids)
            .created_at(now)
            .build())
    }

    /// Lists all active (non-archived) envelopes, ordered by name.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list(&self) -> BcResult<Vec<Envelope>> {
        let mut rows = sqlx::query_as::<_, EnvelopeRow>(
            "SELECT id, name, parent_id, icon, colour, commodity, \
              allocation_target_amount, allocation_target_commodity, \
              period, rollover_policy, created_at \
             FROM envelopes \
             WHERE archived_at IS NULL \
             ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        if !rows.is_empty() {
            let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();

            let mut builder = sqlx::QueryBuilder::new(
                "SELECT envelope_id, account_id \
                 FROM envelope_account_links \
                 WHERE envelope_id IN ",
            );
            builder.push_tuples(ids.iter(), |mut b, id| {
                b.push_bind(id);
            });
            let link_rows: Vec<(String, String)> =
                builder.build_query_as().fetch_all(&self.pool).await?;

            let mut links_map: HashMap<String, Vec<AccountId>> = HashMap::new();
            for (envelope_id, account_id_str) in link_rows {
                let account_id = account_id_str.parse::<AccountId>().map_err(|e| {
                    BcError::BadData(format!("invalid account_id '{account_id_str}': {e}"))
                })?;
                links_map.entry(envelope_id).or_default().push(account_id);
            }

            for row in &mut rows {
                row.account_ids = links_map.remove(&row.id).unwrap_or_default();
            }
        }

        rows.into_iter().map(Envelope::try_from).collect()
    }

    /// Finds an envelope by ID.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no envelope with that ID exists.
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn get(&self, id: &EnvelopeId) -> BcResult<Envelope> {
        let mut row = sqlx::query_as::<_, EnvelopeRow>(
            "SELECT id, name, parent_id, icon, colour, commodity, \
              allocation_target_amount, allocation_target_commodity, \
              period, rollover_policy, created_at \
             FROM envelopes \
             WHERE id = ? AND archived_at IS NULL",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(id.to_string()))?;

        row.account_ids = self.load_account_ids(id).await?;

        Envelope::try_from(row)
    }

    /// Moves an envelope to a different group, or removes it from all groups.
    ///
    /// # Arguments
    ///
    /// * `envelope_id` - The ID of the envelope to move.
    /// * `group_id` - The target group ID, or `None` to place the envelope at the root.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no active envelope with that ID exists.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn move_to_group(
        &self,
        envelope_id: &EnvelopeId,
        group_id: Option<&EnvelopeGroupId>,
    ) -> BcResult<Envelope> {
        let event = Event::EnvelopeMoved {
            id: envelope_id.clone(),
            group_id: group_id.cloned(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        let result =
            sqlx::query("UPDATE envelopes SET parent_id = ? WHERE id = ? AND archived_at IS NULL")
                .bind(group_id.map(ToString::to_string))
                .bind(envelope_id.to_string())
                .execute(&mut *db_tx)
                .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(envelope_id.to_string()));
        }

        db_tx.commit().await?;
        tracing::info!(envelope_id = %envelope_id, ?group_id, "envelope moved to group");

        self.get(envelope_id).await
    }

    /// Archives an envelope by ID.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no active envelope with that ID exists.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn archive(&self, id: &EnvelopeId) -> BcResult<()> {
        let now = Timestamp::now();
        let event = Event::EnvelopeArchived { id: id.clone() };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        let result = sqlx::query(
            "UPDATE envelopes SET archived_at = ? WHERE id = ? AND archived_at IS NULL",
        )
        .bind(now.to_string())
        .bind(id.to_string())
        .execute(&mut *db_tx)
        .await?;

        if result.rows_affected() == 0 {
            // rows_affected == 0 means the UPDATE found no matching row.
            // Returning here drops `db_tx` without committing — sqlx rolls it
            // back implicitly, discarding the event insert above.
            return Err(BcError::NotFound(id.to_string()));
        }

        db_tx.commit().await?;
        tracing::info!(envelope_id = %id, "envelope archived");
        Ok(())
    }

    // ── Allocation management ─────────────────────────────────────────────

    /// Allocates funds to an envelope for the period starting on `period_start`.
    ///
    /// If an allocation already exists for that period, it is replaced (upsert).
    /// Each call appends a new [`Event::EnvelopeAllocated`] regardless of whether
    /// an existing allocation was replaced; the event log will therefore contain one
    /// event per call rather than a tombstone for the superseded allocation.  This is
    /// intentional: the projection table (`envelope_allocations`) is the authoritative
    /// read model, and the event log is used for audit purposes only.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database failure.
    #[inline]
    pub async fn allocate(
        &self,
        envelope_id: &EnvelopeId,
        period_start: Date,
        amount: Amount,
    ) -> BcResult<Allocation> {
        // Validate the envelope exists and is not archived.
        let envelope = self.get(envelope_id).await?;

        // Validate commodity consistency: the allocation must match the envelope's commodity.
        if amount.commodity() != envelope.commodity() {
            return Err(BcError::InvalidInput(format!(
                "envelope '{}' uses commodity '{}' but the allocation amount is in '{}'",
                envelope_id,
                envelope.commodity(),
                amount.commodity(),
            )));
        }

        let id = AllocationId::new();
        let now = Timestamp::now();
        let event = Event::EnvelopeAllocated {
            id: id.clone(),
            envelope_id: envelope_id.clone(),
            period_start,
            amount: amount.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        sqlx::query(
            "INSERT INTO envelope_allocations (id, envelope_id, period_start, amount, commodity, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (envelope_id, period_start) \
             DO UPDATE SET id = excluded.id, amount = excluded.amount, commodity = excluded.commodity, created_at = excluded.created_at",
        )
        .bind(id.to_string())
        .bind(envelope_id.to_string())
        .bind(period_start.to_string())
        .bind(amount.value().to_string())
        .bind(amount.commodity().as_str())
        .bind(now.to_string())
        .execute(&mut *db_tx)
        .await?;

        db_tx.commit().await?;
        tracing::info!(envelope_id = %envelope_id, %period_start, "envelope allocated");

        Ok(Allocation::builder()
            .id(id)
            .envelope_id(envelope_id.clone())
            .period_start(period_start)
            .amount(amount)
            .created_at(now)
            .build())
    }

    /// Retrieves the allocation for an envelope in a specific period, if one exists.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn get_allocation(
        &self,
        envelope_id: &EnvelopeId,
        period_start: Date,
    ) -> BcResult<Option<Allocation>> {
        let row: Option<(String, String, String, String, String)> = sqlx::query_as(
            "SELECT id, envelope_id, amount, commodity, created_at \
             FROM envelope_allocations \
             WHERE envelope_id = ? AND period_start = ?",
        )
        .bind(envelope_id.to_string())
        .bind(period_start.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|(id_str, env_id_str, amt_str, com_str, created_str)| {
            let id = id_str
                .parse::<AllocationId>()
                .map_err(|e| BcError::BadData(format!("invalid allocation id '{id_str}': {e}")))?;
            let env_id = env_id_str.parse::<EnvelopeId>().map_err(|e| {
                BcError::BadData(format!("invalid envelope_id '{env_id_str}': {e}"))
            })?;
            let value = amt_str
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid amount '{amt_str}': {e}")))?;
            let created_at = created_str.parse::<Timestamp>().map_err(|e| {
                BcError::BadData(format!("invalid created_at '{created_str}': {e}"))
            })?;
            Ok(Allocation::builder()
                .id(id)
                .envelope_id(env_id)
                .period_start(period_start)
                .amount(Amount::new(value, CommodityCode::new(com_str)))
                .created_at(created_at)
                .build())
        })
        .transpose()
    }

    /// Loads the account IDs linked to an envelope.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    async fn load_account_ids(&self, id: &EnvelopeId) -> BcResult<Vec<AccountId>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT account_id FROM envelope_account_links WHERE envelope_id = ?")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;

        rows.into_iter()
            .map(|(s,)| {
                s.parse::<AccountId>()
                    .map_err(|e| BcError::BadData(format!("invalid account_id '{s}': {e}")))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::CommodityCode;
    use bc_models::EnvelopeId;
    use bc_models::Period;
    use bc_models::RolloverPolicy;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::account;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_and_list_groups(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let group = svc
            .create_group("Food", None)
            .await
            .expect("create should succeed");
        assert_eq!(group.name(), "Food");
        assert!(group.parent_id().is_none());

        let groups = svc.list_groups().await.expect("list should succeed");
        assert_eq!(groups.len(), 1);
        assert_eq!(
            groups.first().expect("one group should exist").name(),
            "Food"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_nested_group(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc.create_group("Food", None).await.expect("parent");
        let child = svc
            .create_group("Restaurants", Some(parent.id()))
            .await
            .expect("child");
        assert_eq!(child.parent_id(), Some(parent.id()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_and_list_envelopes(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let params = CreateParams {
            name: "Groceries".to_owned(),
            group_id: None,
            icon: None,
            colour: None,
            commodity: CommodityCode::new("AUD"),
            allocation_target: None,
            period: Period::Monthly,
            rollover_policy: RolloverPolicy::CarryForward,
            account_ids: vec![],
        };
        let env = svc.create(params).await.expect("create should succeed");
        assert_eq!(env.name(), "Groceries");
        assert!(env.is_tracking_only());

        let list = svc.list().await.expect("list should succeed");
        assert_eq!(list.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_envelopes_returns_account_ids(pool: sqlx::SqlitePool) {
        let account_svc = account::Service::new(pool.clone());
        let account_id = account_svc
            .create(
                "Checking",
                AccountType::Asset,
                AccountKind::DepositAccount,
                None,
                None,
                &[],
                &[],
            )
            .await
            .expect("account create should succeed");

        let svc = Service::new(pool);
        let params = CreateParams {
            name: "Groceries".to_owned(),
            group_id: None,
            icon: None,
            colour: None,
            commodity: CommodityCode::new("AUD"),
            allocation_target: None,
            period: Period::Monthly,
            rollover_policy: RolloverPolicy::CarryForward,
            account_ids: vec![account_id.clone()],
        };
        svc.create(params)
            .await
            .expect("envelope create should succeed");

        let list = svc.list().await.expect("list should succeed");
        assert_eq!(list.len(), 1);
        let envelope = list.first().expect("one envelope should exist");
        assert_eq!(envelope.account_ids(), &[account_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_envelopes_empty_account_ids(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let params = CreateParams {
            name: "Savings".to_owned(),
            group_id: None,
            icon: None,
            colour: None,
            commodity: CommodityCode::new("AUD"),
            allocation_target: None,
            period: Period::Monthly,
            rollover_policy: RolloverPolicy::CarryForward,
            account_ids: vec![],
        };
        svc.create(params).await.expect("create should succeed");

        let list = svc.list().await.expect("list should succeed");
        assert_eq!(list.len(), 1);
        let envelope = list.first().expect("one envelope should exist");
        assert!(
            envelope.account_ids().is_empty(),
            "envelope with no linked accounts must return empty account_ids"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_envelope(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let params = CreateParams {
            name: "Old Envelope".to_owned(),
            group_id: None,
            icon: None,
            colour: None,
            commodity: CommodityCode::new("AUD"),
            allocation_target: None,
            period: Period::Monthly,
            rollover_policy: RolloverPolicy::ResetToZero,
            account_ids: vec![],
        };
        let env = svc.create(params).await.expect("create");
        svc.archive(env.id()).await.expect("archive");

        let list = svc.list().await.expect("list");
        assert!(
            list.is_empty(),
            "archived envelopes must not appear in list()"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_nonexistent_envelope_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let result = svc.archive(&EnvelopeId::new()).await;
        assert!(matches!(result, Err(crate::BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn allocate_and_retrieve(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use bc_models::Period;
        use bc_models::RolloverPolicy;
        use jiff::civil::Date;

        let svc = Service::new(pool);
        let env = svc
            .create(CreateParams {
                name: "Groceries".to_owned(),
                group_id: None,
                icon: None,
                colour: None,
                commodity: CommodityCode::new("AUD"),
                allocation_target: Some(Amount::new(
                    Decimal::from(500_i32),
                    CommodityCode::new("AUD"),
                )),
                period: Period::Monthly,
                rollover_policy: RolloverPolicy::CarryForward,
                account_ids: vec![],
            })
            .await
            .expect("create");

        let period_start = Date::constant(2026, 3, 1);
        let amount = Amount::new(Decimal::from(500_i32), CommodityCode::new("AUD"));
        let alloc = svc
            .allocate(env.id(), period_start, amount.clone())
            .await
            .expect("allocate");

        assert_eq!(alloc.envelope_id(), env.id());
        assert_eq!(alloc.period_start(), period_start);
        assert_eq!(alloc.amount().value(), amount.value());

        let fetched = svc
            .get_allocation(env.id(), period_start)
            .await
            .expect("get")
            .expect("should exist");
        assert_eq!(fetched.id(), alloc.id());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reallocate_replaces_existing(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use bc_models::Period;
        use bc_models::RolloverPolicy;
        use jiff::civil::Date;

        let svc = Service::new(pool);
        let env = svc
            .create(CreateParams {
                name: "Fuel".to_owned(),
                group_id: None,
                icon: None,
                colour: None,
                commodity: CommodityCode::new("AUD"),
                allocation_target: None,
                period: Period::Monthly,
                rollover_policy: RolloverPolicy::ResetToZero,
                account_ids: vec![],
            })
            .await
            .expect("create");

        let period_start = Date::constant(2026, 3, 1);
        svc.allocate(
            env.id(),
            period_start,
            Amount::new(Decimal::from(200_i32), CommodityCode::new("AUD")),
        )
        .await
        .expect("first allocation");
        svc.allocate(
            env.id(),
            period_start,
            Amount::new(Decimal::from(300_i32), CommodityCode::new("AUD")),
        )
        .await
        .expect("second allocation should replace");

        let fetched = svc
            .get_allocation(env.id(), period_start)
            .await
            .expect("get")
            .expect("should exist");
        assert_eq!(fetched.amount().value(), Decimal::from(300_i32));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn allocate_to_nonexistent_envelope_returns_not_found(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use bc_models::EnvelopeId;
        use jiff::civil::Date;

        let svc = Service::new(pool);
        let bogus_id = EnvelopeId::new();
        let result = svc
            .allocate(
                &bogus_id,
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
            )
            .await;
        assert!(
            matches!(result, Err(crate::BcError::NotFound(_))),
            "allocating to a nonexistent envelope should return NotFound, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn allocate_to_archived_envelope_returns_not_found(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use bc_models::Period;
        use bc_models::RolloverPolicy;
        use jiff::civil::Date;

        let svc = Service::new(pool);
        let env = svc
            .create(CreateParams {
                name: "Archived Envelope".to_owned(),
                group_id: None,
                icon: None,
                colour: None,
                commodity: CommodityCode::new("AUD"),
                allocation_target: None,
                period: Period::Monthly,
                rollover_policy: RolloverPolicy::ResetToZero,
                account_ids: vec![],
            })
            .await
            .expect("create");
        svc.archive(env.id()).await.expect("archive");

        let result = svc
            .allocate(
                env.id(),
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(100_i32), CommodityCode::new("AUD")),
            )
            .await;
        assert!(
            matches!(result, Err(crate::BcError::NotFound(_))),
            "allocating to an archived envelope should return NotFound, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn allocate_with_mismatched_commodity_returns_invalid_input(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;
        use bc_models::Period;
        use bc_models::RolloverPolicy;
        use jiff::civil::Date;

        let svc = Service::new(pool);
        let env = svc
            .create(CreateParams {
                name: "AUD Envelope".to_owned(),
                group_id: None,
                icon: None,
                colour: None,
                commodity: CommodityCode::new("AUD"),
                allocation_target: None,
                period: Period::Monthly,
                rollover_policy: RolloverPolicy::ResetToZero,
                account_ids: vec![],
            })
            .await
            .expect("create");

        let result = svc
            .allocate(
                env.id(),
                Date::constant(2026, 3, 1),
                Amount::new(Decimal::from(100_i32), CommodityCode::new("USD")),
            )
            .await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "allocating with a mismatched commodity should return InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_cap_at_target_without_allocation_target_is_invalid(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let params = CreateParams {
            name: "Savings".to_owned(),
            group_id: None,
            icon: None,
            colour: None,
            commodity: CommodityCode::new("AUD"),
            allocation_target: None,
            period: Period::Monthly,
            rollover_policy: RolloverPolicy::CapAtTarget,
            account_ids: vec![],
        };
        let result = svc.create(params).await;
        assert!(
            result.is_err(),
            "CapAtTarget without allocation_target should fail"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn move_to_group_changes_parent_id(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);

        // Create a group to move the envelope into.
        let group = svc
            .create_group("Transport", None)
            .await
            .expect("create group should succeed");

        // Create an envelope with no group initially.
        let env = svc
            .create(CreateParams {
                name: "Fuel".to_owned(),
                group_id: None,
                icon: None,
                colour: None,
                commodity: CommodityCode::new("AUD"),
                allocation_target: None,
                period: Period::Monthly,
                rollover_policy: RolloverPolicy::ResetToZero,
                account_ids: vec![],
            })
            .await
            .expect("create envelope should succeed");

        assert!(
            env.parent_id().is_none(),
            "envelope should start with no group"
        );

        // Move the envelope into the group.
        let moved = svc
            .move_to_group(env.id(), Some(group.id()))
            .await
            .expect("move_to_group should succeed");

        assert_eq!(
            moved.parent_id(),
            Some(group.id()),
            "envelope should now belong to the group"
        );

        // Verify persistence by re-fetching.
        let fetched = svc.get(env.id()).await.expect("get should succeed");
        assert_eq!(
            fetched.parent_id(),
            Some(group.id()),
            "parent_id should persist to DB"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn move_to_group_with_none_removes_group(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);

        let group = svc.create_group("Food", None).await.expect("create group");

        let env = svc
            .create(CreateParams {
                name: "Groceries".to_owned(),
                group_id: Some(group.id().clone()),
                icon: None,
                colour: None,
                commodity: CommodityCode::new("AUD"),
                allocation_target: None,
                period: Period::Monthly,
                rollover_policy: RolloverPolicy::CarryForward,
                account_ids: vec![],
            })
            .await
            .expect("create envelope should succeed");

        assert_eq!(
            env.parent_id(),
            Some(group.id()),
            "envelope should start in the group"
        );

        // Move to root (no group).
        let moved = svc
            .move_to_group(env.id(), None)
            .await
            .expect("move_to_group should succeed");

        assert!(
            moved.parent_id().is_none(),
            "envelope should now have no group"
        );

        let fetched = svc.get(env.id()).await.expect("get should succeed");
        assert!(
            fetched.parent_id().is_none(),
            "parent_id NULL should persist to DB"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn move_to_group_nonexistent_envelope_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let result = svc.move_to_group(&EnvelopeId::new(), None).await;
        assert!(
            matches!(result, Err(crate::BcError::NotFound(_))),
            "unknown envelope should return NotFound"
        );
    }
}
