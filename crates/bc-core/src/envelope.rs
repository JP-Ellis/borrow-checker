//! Envelope and allocation service.

use bc_models::AccountId;
use bc_models::Allocation;
use bc_models::AllocationId;
use bc_models::Amount;
use bc_models::CommodityCode;
use bc_models::Decimal;
use bc_models::Envelope;
use bc_models::EnvelopeId;
use bc_models::Period;
use bc_models::RolloverPolicy;
use bc_models::TagId;
use jiff::Timestamp;
use jiff::civil::Date;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;

/// Internal row type returned from the `envelopes` table.
///
/// `account_ids` and `tag_ids` are populated separately via join queries.
#[derive(sqlx::FromRow)]
struct EnvelopeRow {
    /// Raw envelope ID string.
    id: String,
    /// Display name.
    name: String,
    /// Raw parent envelope ID string, if nested.
    parent_id: Option<String>,
    /// Optional icon identifier.
    icon: Option<String>,
    /// Optional colour code.
    colour: Option<String>,
    /// Commodity code, or NULL if tracking across all commodities.
    commodity: Option<String>,
    /// Decimal string for the allocation target amount; NULL = tracking-only.
    allocation_target_amount: Option<String>,
    /// Commodity code for the allocation target; NULL when amount is NULL.
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
    /// Tag IDs — populated after the initial query.
    #[sqlx(skip)]
    tag_ids: Vec<TagId>,
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
                s.parse::<EnvelopeId>()
                    .map_err(|e| BcError::BadData(format!("invalid parent_id '{s}': {e}")))
            })
            .transpose()?;

        let commodity = row.commodity.as_deref().map(CommodityCode::new);

        let allocation_target = match (
            row.allocation_target_amount,
            row.allocation_target_commodity,
        ) {
            (Some(amt_str), Some(com_str)) => {
                let quantity = amt_str.parse::<Decimal>().map_err(|e| {
                    BcError::BadData(format!("invalid allocation_target_amount '{amt_str}': {e}"))
                })?;
                Some(Amount::new(quantity, CommodityCode::new(&com_str)))
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

        Ok(Self::builder()
            .id(id)
            .name(row.name)
            .maybe_parent_id(parent_id)
            .maybe_icon(row.icon)
            .maybe_colour(row.colour)
            .maybe_commodity(commodity)
            .maybe_allocation_target(allocation_target)
            .period(period)
            .rollover_policy(rollover_policy)
            .account_ids(row.account_ids)
            .tag_ids(row.tag_ids)
            .created_at(created_at)
            .build())
    }
}

/// Internal row type for allocation queries.
#[derive(sqlx::FromRow)]
struct AllocationRow {
    /// Raw allocation ID string.
    id: String,
    /// Raw envelope ID string.
    envelope_id: String,
    /// Decimal amount string.
    amount: String,
    /// Commodity code string.
    commodity: String,
    /// ISO 8601 creation timestamp.
    created_at: String,
}

/// Envelope CRUD and allocation service.
#[derive(Debug, Clone)]
pub struct Service {
    /// The SQLite connection pool.
    pool: SqlitePool,
}

#[bon::bon]
impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // MARK: Envelope management

    /// Creates a new budget envelope.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::InvalidInput`] if `rollover_policy` is `CapAtTarget`
    /// but `allocation_target` is `None`, if `allocation_target` is set but
    /// `commodity` is `None`, or if the allocation target's commodity does not
    /// match `commodity` when both are provided.
    /// Returns [`BcError`] on event append or database insert failure.
    #[builder]
    #[inline]
    pub async fn create(
        &self,
        #[builder(into)] name: String,
        parent_id: Option<EnvelopeId>,
        #[builder(into)] icon: Option<String>,
        #[builder(into)] colour: Option<String>,
        commodity: Option<CommodityCode>,
        allocation_target: Option<Amount>,
        period: Period,
        rollover_policy: RolloverPolicy,
        #[builder(default)] account_ids: Vec<AccountId>,
        #[builder(default)] tag_ids: Vec<TagId>,
    ) -> BcResult<Envelope> {
        if rollover_policy == RolloverPolicy::CapAtTarget && allocation_target.is_none() {
            return Err(BcError::InvalidInput(
                "CapAtTarget rollover policy requires an allocation target".to_owned(),
            ));
        }

        if allocation_target.is_some() && commodity.is_none() {
            return Err(BcError::InvalidInput(
                "allocation_target requires a commodity to be set on the envelope".to_owned(),
            ));
        }

        if let (Some(target), Some(env_commodity)) = (&allocation_target, &commodity) {
            if target.commodity() != env_commodity {
                return Err(BcError::InvalidInput(format!(
                    "allocation_target commodity '{}' does not match envelope commodity '{}'",
                    target.commodity(),
                    env_commodity,
                )));
            }
        }

        let id = EnvelopeId::new();
        let now = Timestamp::now();

        let event = Event::EnvelopeCreated {
            id: id.clone(),
            name: name.clone(),
            parent_id: parent_id.clone(),
            period: period.clone(),
            rollover_policy,
            allocation_target: allocation_target.clone(),
            commodity: commodity.clone(),
            icon: icon.clone(),
            colour: colour.clone(),
            account_ids: account_ids.clone(),
            tag_ids: tag_ids.clone(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        let period_json = serde_json::to_string(&period)?;
        let rollover_db = to_db_str(rollover_policy)?;
        let (target_amount, target_commodity) =
            allocation_target.as_ref().map_or((None, None), |a| {
                (Some(a.value().to_string()), Some(a.commodity().to_string()))
            });

        sqlx::query(
            "INSERT INTO envelopes \
             (id, name, parent_id, icon, colour, commodity, \
              allocation_target_amount, allocation_target_commodity, \
              period, rollover_policy, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id.to_string()) //  1. id
        .bind(&name) //  2. name
        .bind(parent_id.as_ref().map(ToString::to_string)) //  3. parent_id
        .bind(&icon) //  4. icon
        .bind(&colour) //  5. colour
        .bind(commodity.as_ref().map(CommodityCode::as_str)) //  6. commodity
        .bind(&target_amount) //  7. allocation_target_amount
        .bind(&target_commodity) //  8. allocation_target_commodity
        .bind(&period_json) //  9. period
        .bind(&rollover_db) // 10. rollover_policy
        .bind(now.to_string()) // 11. created_at
        .execute(&mut *db_tx)
        .await?;

        for account_id in &account_ids {
            sqlx::query(
                "INSERT INTO envelope_account_links (envelope_id, account_id) VALUES (?, ?)",
            )
            .bind(id.to_string())
            .bind(account_id.to_string())
            .execute(&mut *db_tx)
            .await?;
        }

        for tag_id in &tag_ids {
            sqlx::query("INSERT INTO envelope_tags (envelope_id, tag_id) VALUES (?, ?)")
                .bind(id.to_string())
                .bind(tag_id.to_string())
                .execute(&mut *db_tx)
                .await?;
        }

        db_tx.commit().await?;
        tracing::info!(envelope_id = %id, %name, "envelope created");

        Ok(Envelope::builder()
            .id(id)
            .name(name)
            .maybe_parent_id(parent_id)
            .maybe_icon(icon)
            .maybe_colour(colour)
            .maybe_commodity(commodity)
            .maybe_allocation_target(allocation_target)
            .period(period)
            .rollover_policy(rollover_policy)
            .account_ids(account_ids)
            .tag_ids(tag_ids)
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
            self.populate_account_ids(&mut rows).await?;
            self.populate_tag_ids(&mut rows).await?;
        }

        rows.into_iter().map(Envelope::try_from).collect()
    }

    /// Finds an active envelope by ID.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no active envelope with that ID exists.
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
        row.tag_ids = self.load_tag_ids(id).await?;

        Envelope::try_from(row)
    }

    /// Moves an envelope to a different parent, or removes it from all parents.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::InvalidInput`] if `parent_id` equals `envelope_id`,
    /// if the proposed parent does not exist or is archived, or if the move
    /// would create a cycle in the envelope hierarchy.
    /// Returns [`BcError::NotFound`] if no active envelope with that ID exists.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn set_parent(
        &self,
        envelope_id: &EnvelopeId,
        parent_id: Option<&EnvelopeId>,
    ) -> BcResult<Envelope> {
        // Guard 1 — self-reference check.
        if parent_id == Some(envelope_id) {
            return Err(BcError::InvalidInput(format!(
                "envelope '{envelope_id}' cannot be its own parent"
            )));
        }

        // Guard 2 — parent existence check.
        if let Some(pid) = parent_id {
            match self.get(pid).await {
                Ok(_) => {}
                Err(BcError::NotFound(_)) => {
                    return Err(BcError::InvalidInput(format!(
                        "parent envelope '{pid}' does not exist or is archived"
                    )));
                }
                Err(e) => return Err(e),
            }
        }

        // Guard 3 — cycle detection via recursive CTE.
        if let Some(pid) = parent_id {
            let is_cycle: Option<(i64,)> = sqlx::query_as(
                "WITH RECURSIVE ancestors(id) AS (
                     SELECT ? AS id
                     UNION ALL
                     SELECT e.parent_id FROM envelopes e
                     INNER JOIN ancestors a ON e.id = a.id
                     WHERE e.parent_id IS NOT NULL
                 )
                 SELECT 1 FROM ancestors WHERE id = ? LIMIT 1",
            )
            .bind(pid.to_string())
            .bind(envelope_id.to_string())
            .fetch_optional(&self.pool)
            .await?;

            if is_cycle.is_some() {
                return Err(BcError::InvalidInput(format!(
                    "setting parent of '{envelope_id}' to '{pid}' would create a cycle"
                )));
            }
        }

        let event = Event::EnvelopeMoved {
            id: envelope_id.clone(),
            parent_id: parent_id.cloned(),
        };

        let mut db_tx = self.pool.begin().await?;
        insert_event(&event, &mut db_tx).await?;

        let result =
            sqlx::query("UPDATE envelopes SET parent_id = ? WHERE id = ? AND archived_at IS NULL")
                .bind(parent_id.map(ToString::to_string))
                .bind(envelope_id.to_string())
                .execute(&mut *db_tx)
                .await?;

        if result.rows_affected() == 0 {
            return Err(BcError::NotFound(envelope_id.to_string()));
        }

        db_tx.commit().await?;
        tracing::info!(envelope_id = %envelope_id, ?parent_id, "envelope parent updated");

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
            return Err(BcError::NotFound(id.to_string()));
        }

        db_tx.commit().await?;
        tracing::info!(envelope_id = %id, "envelope archived");
        Ok(())
    }

    // MARK: Allocation management

    /// Allocates funds to an envelope for the period starting on `period_start`.
    ///
    /// If an allocation already exists for that period, it is replaced (upsert).
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the envelope does not exist or is archived.
    /// Returns [`BcError::InvalidInput`] if the allocation commodity does not match
    /// the envelope's commodity (when the envelope has one set).
    /// Returns [`BcError`] on event append or database failure.
    #[inline]
    pub async fn allocate(
        &self,
        envelope_id: &EnvelopeId,
        period_start: Date,
        amount: Amount,
    ) -> BcResult<Allocation> {
        let envelope = self.get(envelope_id).await?;

        if let Some(env_commodity) = envelope.commodity() {
            if amount.commodity() != env_commodity {
                return Err(BcError::InvalidInput(format!(
                    "envelope '{}' uses commodity '{}' but the allocation amount is in '{}'",
                    envelope_id,
                    env_commodity,
                    amount.commodity(),
                )));
            }
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
            "INSERT INTO envelope_allocations \
             (id, envelope_id, period_start, amount, commodity, created_at) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT (envelope_id, period_start) \
             DO UPDATE SET id = excluded.id, amount = excluded.amount, \
                           commodity = excluded.commodity, created_at = excluded.created_at",
        )
        .bind(id.to_string()) // 1. id
        .bind(envelope_id.to_string()) // 2. envelope_id
        .bind(period_start.to_string()) // 3. period_start
        .bind(amount.value().to_string()) // 4. amount
        .bind(amount.commodity().as_str()) // 5. commodity
        .bind(now.to_string()) // 6. created_at
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
        let row: Option<AllocationRow> = sqlx::query_as(
            "SELECT id, envelope_id, amount, commodity, created_at \
             FROM envelope_allocations \
             WHERE envelope_id = ? AND period_start = ?",
        )
        .bind(envelope_id.to_string())
        .bind(period_start.to_string())
        .fetch_optional(&self.pool)
        .await?;

        row.map(|r| {
            let id = r
                .id
                .parse::<AllocationId>()
                .map_err(|e| BcError::BadData(format!("invalid allocation id '{}': {e}", r.id)))?;
            let env_id = r.envelope_id.parse::<EnvelopeId>().map_err(|e| {
                BcError::BadData(format!("invalid envelope_id '{}': {e}", r.envelope_id))
            })?;
            let value = r
                .amount
                .parse::<Decimal>()
                .map_err(|e| BcError::BadData(format!("invalid amount '{}': {e}", r.amount)))?;
            let created_at = r.created_at.parse::<Timestamp>().map_err(|e| {
                BcError::BadData(format!("invalid created_at '{}': {e}", r.created_at))
            })?;
            Ok(Allocation::builder()
                .id(id)
                .envelope_id(env_id)
                .period_start(period_start)
                .amount(Amount::new(value, CommodityCode::new(r.commodity)))
                .created_at(created_at)
                .build())
        })
        .transpose()
    }

    // MARK: Private helpers

    /// Loads the account IDs linked to an envelope.
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

    /// Loads the tag IDs linked to an envelope.
    #[inline]
    async fn load_tag_ids(&self, id: &EnvelopeId) -> BcResult<Vec<TagId>> {
        let rows: Vec<(String,)> =
            sqlx::query_as("SELECT tag_id FROM envelope_tags WHERE envelope_id = ?")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|(s,)| {
                s.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid tag_id '{s}': {e}")))
            })
            .collect()
    }

    /// Bulk-populates `account_ids` on a slice of [`EnvelopeRow`]s via a single query.
    #[inline]
    async fn populate_account_ids(&self, rows: &mut [EnvelopeRow]) -> BcResult<()> {
        use std::collections::HashMap;
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT envelope_id, account_id FROM envelope_account_links WHERE envelope_id IN ",
        );
        builder.push_tuples(ids.iter(), |mut b, id| {
            b.push_bind(id);
        });
        let link_rows: Vec<(String, String)> =
            builder.build_query_as().fetch_all(&self.pool).await?;

        let mut map: HashMap<String, Vec<AccountId>> = HashMap::new();
        for (env_id, acct_id_str) in link_rows {
            let acct_id = acct_id_str.parse::<AccountId>().map_err(|e| {
                BcError::BadData(format!("invalid account_id '{acct_id_str}': {e}"))
            })?;
            map.entry(env_id).or_default().push(acct_id);
        }
        for row in rows.iter_mut() {
            row.account_ids = map.remove(&row.id).unwrap_or_default();
        }
        Ok(())
    }

    /// Bulk-populates `tag_ids` on a slice of [`EnvelopeRow`]s via a single query.
    #[inline]
    async fn populate_tag_ids(&self, rows: &mut [EnvelopeRow]) -> BcResult<()> {
        use std::collections::HashMap;
        let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
        let mut builder = sqlx::QueryBuilder::new(
            "SELECT envelope_id, tag_id FROM envelope_tags WHERE envelope_id IN ",
        );
        builder.push_tuples(ids.iter(), |mut b, id| {
            b.push_bind(id);
        });
        let tag_rows: Vec<(String, String)> =
            builder.build_query_as().fetch_all(&self.pool).await?;

        let mut map: HashMap<String, Vec<TagId>> = HashMap::new();
        for (env_id, tag_id_str) in tag_rows {
            let tid = tag_id_str
                .parse::<TagId>()
                .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id_str}': {e}")))?;
            map.entry(env_id).or_default().push(tid);
        }
        for row in rows.iter_mut() {
            row.tag_ids = map.remove(&row.id).unwrap_or_default();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::EnvelopeId;
    use bc_models::Period;
    use bc_models::RolloverPolicy;
    use pretty_assertions::assert_eq;

    use super::*;
    use crate::account;

    #[sqlx::test(migrations = "./migrations")]
    async fn create_and_list_envelopes(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let env = svc
            .create()
            .name("Groceries")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create should succeed");
        assert_eq!(env.name(), "Groceries");
        assert!(env.is_tracking_only());

        let list = svc.list().await.expect("list should succeed");
        assert_eq!(list.len(), 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_envelopes_returns_account_ids(pool: sqlx::SqlitePool) {
        let account_svc = account::Service::new(pool.clone());
        let account_id = account_svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("account create should succeed");

        let svc = Service::new(pool);
        svc.create()
            .name("Groceries")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .account_ids(vec![account_id.clone()])
            .call()
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
        svc.create()
            .name("Savings")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create should succeed");

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
        let env = svc
            .create()
            .name("Old Envelope")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");
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
            .create()
            .name("Groceries")
            .commodity(CommodityCode::new("AUD"))
            .allocation_target(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .call()
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
            .create()
            .name("Fuel")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
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
            .create()
            .name("Archived Envelope")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
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
            .create()
            .name("AUD Envelope")
            .commodity(CommodityCode::new("AUD"))
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
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
        let result = svc
            .create()
            .name("Savings")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CapAtTarget)
            .call()
            .await;
        assert!(
            result.is_err(),
            "CapAtTarget without allocation_target should fail"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_changes_parent_id(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);

        // Create a parent envelope.
        let parent = svc
            .create()
            .name("Transport")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create parent should succeed");

        // Create an envelope with no parent initially.
        let env = svc
            .create()
            .name("Fuel")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create envelope should succeed");

        assert!(
            env.parent_id().is_none(),
            "envelope should start with no parent"
        );

        // Move the envelope under the parent.
        let moved = svc
            .set_parent(env.id(), Some(parent.id()))
            .await
            .expect("set_parent should succeed");

        assert_eq!(
            moved.parent_id(),
            Some(parent.id()),
            "envelope should now have the parent"
        );

        // Verify persistence by re-fetching.
        let fetched = svc.get(env.id()).await.expect("get should succeed");
        assert_eq!(
            fetched.parent_id(),
            Some(parent.id()),
            "parent_id should persist to DB"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_with_none_removes_parent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);

        let parent = svc
            .create()
            .name("Food")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create parent");

        let env = svc
            .create()
            .name("Groceries")
            .parent_id(parent.id().clone())
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::CarryForward)
            .call()
            .await
            .expect("create envelope should succeed");

        assert_eq!(
            env.parent_id(),
            Some(parent.id()),
            "envelope should start with the parent"
        );

        // Move to root (no parent).
        let moved = svc
            .set_parent(env.id(), None)
            .await
            .expect("set_parent should succeed");

        assert!(
            moved.parent_id().is_none(),
            "envelope should now have no parent"
        );

        let fetched = svc.get(env.id()).await.expect("get should succeed");
        assert!(
            fetched.parent_id().is_none(),
            "parent_id NULL should persist to DB"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_nonexistent_envelope_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let result = svc.set_parent(&EnvelopeId::new(), None).await;
        assert!(
            matches!(result, Err(crate::BcError::NotFound(_))),
            "unknown envelope should return NotFound"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_envelope_with_bon_builder(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let env = svc
            .create()
            .name("Groceries")
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create should succeed");
        assert_eq!(env.name(), "Groceries");
        assert!(env.commodity().is_none());
        assert!(env.parent_id().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_nested_envelope(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc
            .create()
            .name("Health")
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create parent");
        let child = svc
            .create()
            .name("Gym")
            .parent_id(parent.id().clone())
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create child");
        assert_eq!(child.parent_id(), Some(parent.id()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_moves_envelope(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc
            .create()
            .name("Health")
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create parent");
        let child = svc
            .create()
            .name("Gym")
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create child");
        let moved = svc
            .set_parent(child.id(), Some(parent.id()))
            .await
            .expect("set_parent");
        assert_eq!(moved.parent_id(), Some(parent.id()));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_with_allocation_target_but_no_commodity_is_invalid(pool: sqlx::SqlitePool) {
        use bc_models::Amount;
        use bc_models::CommodityCode;
        use bc_models::Decimal;

        let svc = Service::new(pool);
        let result = svc
            .create()
            .name("Test")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .allocation_target(Amount::new(
                Decimal::from(500_i32),
                CommodityCode::new("AUD"),
            ))
            .call()
            .await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "allocation_target without commodity must be InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_envelope_with_tag_ids(pool: sqlx::SqlitePool) {
        use bc_models::TagId;
        use jiff::Timestamp;
        let tag_id = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, 'person:me', ?)")
            .bind(tag_id.to_string())
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("insert tag");

        let svc = Service::new(pool);
        let env = svc
            .create()
            .name("Gym")
            .period(bc_models::Period::Monthly)
            .rollover_policy(bc_models::RolloverPolicy::ResetToZero)
            .tag_ids(vec![tag_id.clone()])
            .call()
            .await
            .expect("create");
        assert_eq!(env.tag_ids(), core::slice::from_ref(&tag_id));

        // Verify persistence.
        let fetched = svc.get(env.id()).await.expect("get");
        assert_eq!(fetched.tag_ids(), &[tag_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_rejects_self_reference(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let env = svc
            .create()
            .name("Self")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");

        let result = svc.set_parent(env.id(), Some(env.id())).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "self-reference should return InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_rejects_nonexistent_parent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let env = svc
            .create()
            .name("Child")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create");

        let bogus_id = EnvelopeId::new();
        let result = svc.set_parent(env.id(), Some(&bogus_id)).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "nonexistent parent should return InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_rejects_archived_parent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc
            .create()
            .name("Soon Archived")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create parent");
        svc.archive(parent.id()).await.expect("archive parent");

        let child = svc
            .create()
            .name("Child")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create child");

        let result = svc.set_parent(child.id(), Some(parent.id())).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "archived parent should return InvalidInput, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_parent_rejects_cycle(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let a = svc
            .create()
            .name("A")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create A");
        let b = svc
            .create()
            .name("B")
            .period(Period::Monthly)
            .rollover_policy(RolloverPolicy::ResetToZero)
            .call()
            .await
            .expect("create B");

        // B is a child of A — valid.
        svc.set_parent(b.id(), Some(a.id()))
            .await
            .expect("set B's parent to A should succeed");

        // Now try to make A a child of B — this would create a cycle.
        let result = svc.set_parent(a.id(), Some(b.id())).await;
        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "cycle-creating move should return InvalidInput, got: {result:?}"
        );
    }
}
