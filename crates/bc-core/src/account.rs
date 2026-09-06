//! Account projection service.

use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::Account;
use bc_models::AccountId;
use bc_models::AccountKind;
use bc_models::AccountType;
use bc_models::CommodityId;
use bc_models::TagId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::AccountPath;
use crate::BcError;
use crate::BcResult;
use crate::db::from_db_str;
use crate::db::to_db_str;
use crate::events::Event;
use crate::events::insert_event;

/// Internal row type returned from the `accounts` table, mapped by `sqlx::FromRow`.
///
/// The `commodities` and `tag_ids` fields are populated separately via join queries
/// after the initial row fetch.
#[derive(sqlx::FromRow)]
struct AccountRow {
    /// Raw account ID string.
    id: String,
    /// Account display name.
    name: String,
    /// Account type stored as `snake_case` string.
    account_type: String,
    /// Account maintenance kind stored as `snake_case` string.
    kind: String,
    /// Optional description.
    description: Option<String>,
    /// Raw parent account ID string, if this account has a parent.
    parent_id: Option<String>,
    /// ISO 8601 creation timestamp.
    created_at: String,
    /// ISO 8601 archive timestamp if archived.
    archived_at: Option<String>,
    /// Acquisition date for `ManualAsset` accounts (YYYY-MM-DD), if recorded.
    acquisition_date: Option<String>,
    /// Acquisition cost as a decimal string, if recorded.
    acquisition_cost: Option<String>,
    /// JSON-encoded `DepreciationPolicy`, if set.
    depreciation_policy: Option<String>,
    /// Business date the account opened (YYYY-MM-DD), if declared.
    opened_on: Option<String>,
    /// Business date the account closed (YYYY-MM-DD), if closed.
    closed_on: Option<String>,
    /// Allowed commodities; first = default; empty = unrestricted.
    #[sqlx(skip)]
    commodities: Vec<CommodityId>,
    /// Tags attached to this account.
    #[sqlx(skip)]
    tag_ids: Vec<TagId>,
}

impl TryFrom<AccountRow> for Account {
    type Error = BcError;

    /// Converts a raw database row into a domain [`Account`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if any stored value cannot be parsed.
    #[inline]
    fn try_from(row: AccountRow) -> BcResult<Self> {
        let id = row
            .id
            .parse::<AccountId>()
            .map_err(|e| BcError::BadData(format!("invalid account id '{}': {e}", row.id)))?;

        let account_type = from_db_str::<AccountType>(&row.account_type)?;

        let kind = from_db_str::<AccountKind>(&row.kind)?;

        let created_at = row.created_at.parse::<Timestamp>().map_err(|e| {
            BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;

        let archived_at = row
            .archived_at
            .as_deref()
            .map(|s| {
                s.parse::<Timestamp>()
                    .map_err(|e| BcError::BadData(format!("invalid archived_at '{s}': {e}")))
            })
            .transpose()?;

        let parent_id = row
            .parent_id
            .as_deref()
            .map(|s| {
                s.parse::<AccountId>()
                    .map_err(|e| BcError::BadData(format!("invalid parent_id '{s}': {e}")))
            })
            .transpose()?;

        let acquisition_date = row
            .acquisition_date
            .as_deref()
            .map(|s| {
                s.parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid acquisition_date '{s}': {e}")))
            })
            .transpose()?;

        let acquisition_cost = row
            .acquisition_cost
            .as_deref()
            .map(|s| {
                s.parse::<rust_decimal::Decimal>()
                    .map_err(|e| BcError::BadData(format!("invalid acquisition_cost '{s}': {e}")))
            })
            .transpose()?;

        let depreciation_policy = row
            .depreciation_policy
            .as_deref()
            .map(|s| {
                serde_json::from_str::<bc_models::DepreciationPolicy>(s)
                    .map_err(|e| BcError::BadData(format!("invalid depreciation_policy: {e}")))
            })
            .transpose()?;

        let opened_on = row
            .opened_on
            .as_deref()
            .map(|s| {
                s.parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid opened_on '{s}': {e}")))
            })
            .transpose()?;

        let closed_on = row
            .closed_on
            .as_deref()
            .map(|s| {
                s.parse::<jiff::civil::Date>()
                    .map_err(|e| BcError::BadData(format!("invalid closed_on '{s}': {e}")))
            })
            .transpose()?;

        Ok(Self::builder()
            .id(id)
            .name(row.name)
            .account_type(account_type)
            .kind(kind)
            .commodities(row.commodities)
            .tag_ids(row.tag_ids)
            .maybe_description(row.description)
            .maybe_parent_id(parent_id)
            .maybe_archived_at(archived_at)
            .maybe_acquisition_date(acquisition_date)
            .maybe_acquisition_cost(acquisition_cost)
            .maybe_depreciation_policy(depreciation_policy)
            .maybe_opened_on(opened_on)
            .maybe_closed_on(closed_on)
            .created_at(created_at)
            .build())
    }
}

/// Parses a slice of `(account_id, commodity_id)` rows into a `HashMap`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any commodity ID string is malformed.
fn build_commodities_map(
    rows: Vec<(String, String)>,
) -> BcResult<HashMap<String, Vec<CommodityId>>> {
    let mut map: HashMap<String, Vec<CommodityId>> = HashMap::new();
    for (account_id, commodity_id) in rows {
        let cid = commodity_id
            .parse::<CommodityId>()
            .map_err(|e| BcError::BadData(format!("invalid commodity_id '{commodity_id}': {e}")))?;
        map.entry(account_id).or_default().push(cid);
    }
    Ok(map)
}

/// Parses a slice of `(account_id, tag_id)` rows into a `HashMap`.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if any tag ID string is malformed.
fn build_tags_map(rows: Vec<(String, String)>) -> BcResult<HashMap<String, Vec<TagId>>> {
    let mut map: HashMap<String, Vec<TagId>> = HashMap::new();
    for (account_id, tag_id) in rows {
        let tid = tag_id
            .parse::<TagId>()
            .map_err(|e| BcError::BadData(format!("invalid tag_id '{tag_id}': {e}")))?;
        map.entry(account_id).or_default().push(tid);
    }
    Ok(map)
}

/// What [`Service::archive`] and [`Service::close`] do about descendants that
/// block the write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Cascade {
    /// Reject the write, naming the descendants that block it.
    Reject,
    /// Apply the same write to every blocking descendant, in one transaction.
    Into,
}

impl Cascade {
    /// Whether this cascades into blocking descendants rather than rejecting.
    #[must_use]
    #[inline]
    pub const fn is_into(self) -> bool {
        matches!(self, Self::Into)
    }
}

/// Rejects a closing date that precedes the stored opening date it is paired
/// with.
///
/// An account whose `closed_on` precedes its `opened_on` has no date inside its
/// declared life, so `warning::check_postings` warns on every posting into it,
/// often twice. `label` names the account in the message.
///
/// # Arguments
///
/// * `on` - The closing date being applied.
/// * `opened_on` - The account's stored opening date, unparsed, if any.
/// * `label` - How to name the account in the error message.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `on` precedes `opened_on`, or if `opened_on`
/// does not parse as a date.
fn reject_close_before_open(
    on: jiff::civil::Date,
    opened_on: Option<&str>,
    label: &str,
) -> BcResult<()> {
    let Some(raw) = opened_on else {
        return Ok(());
    };
    let opening = raw
        .parse::<jiff::civil::Date>()
        .map_err(|e| BcError::BadData(format!("invalid opened_on '{raw}': {e}")))?;
    if on < opening {
        return Err(BcError::BadData(format!(
            "cannot close {label} on {on}, before it opened on {opening}"
        )));
    }
    Ok(())
}

/// Rejects a write when `blocking` descendants exist and `cascade` is
/// [`Cascade::Reject`].
///
/// Shared by [`Service::archive`] and [`Service::close`], whose descendant
/// checks differ only in which descendants block the write (active vs. open)
/// and in wording. `action` names the write in the message (e.g. `"archive"`)
/// and `state` names why a descendant blocks it (e.g. `"active"`).
///
/// # Arguments
///
/// * `blocking` - The descendants that block the write.
/// * `cascade` - Whether the caller intends to cascade into `blocking`, which lifts the block.
/// * `action` - The write being attempted, for the error message (e.g. `"archive"`, `"close"`).
/// * `state` - Why a descendant blocks the write, for the error message (e.g. `"active"`, `"open"`).
///
/// # Errors
///
/// Returns [`BcError::BadData`] naming every blocking descendant if `blocking`
/// is non-empty and `cascade` is [`Cascade::Reject`].
fn reject_blocking_descendants(
    blocking: &[&Descendant],
    cascade: Cascade,
    action: &str,
    state: &str,
) -> BcResult<()> {
    if blocking.is_empty() || cascade.is_into() {
        return Ok(());
    }
    let names: Vec<&str> = blocking.iter().map(|d| d.name.as_str()).collect();
    Err(BcError::BadData(format!(
        "cannot {action} an account while {} descendant(s) remain {state}: {}; \
         {action} them first or pass cascade",
        blocking.len(),
        names.join(", ")
    )))
}

/// One descendant account, as [`descendants_of`] reads it.
///
/// Named fields rather than a tuple: `opened_on`, `closed_on` and `archived_at`
/// are all nullable dates, and `archive` and `close` filter on different ones.
#[derive(Debug, sqlx::FromRow)]
struct Descendant {
    /// The account's id, unparsed.
    id: String,
    /// The account's name, for naming it in a blocking-descendant error.
    name: String,
    /// The account's declared opening date, if any.
    opened_on: Option<String>,
    /// The account's declared closing date, if any.
    closed_on: Option<String>,
    /// When the account was archived, if it was.
    archived_at: Option<String>,
}

impl Descendant {
    /// Whether the account is still open, i.e. declares no closing date.
    fn is_open(&self) -> bool {
        self.closed_on.is_none()
    }

    /// Whether the account is still active, i.e. has not been archived.
    fn is_active(&self) -> bool {
        self.archived_at.is_none()
    }
}

/// Returns every descendant of `id`, deepest last, using a recursive CTE.
///
/// Excludes `id` itself.
///
/// # Arguments
///
/// * `conn` - An open connection to read through.
/// * `id` - The subtree root.
///
/// # Errors
///
/// Returns [`BcError`] on database read failure.
async fn descendants_of(
    conn: &mut sqlx::SqliteConnection,
    id: &AccountId,
) -> BcResult<Vec<Descendant>> {
    sqlx::query_as(
        "WITH RECURSIVE subtree(id, name, opened_on, closed_on, archived_at) AS ( \
             SELECT id, name, opened_on, closed_on, archived_at FROM accounts WHERE parent_id = ? \
             UNION ALL \
             SELECT a.id, a.name, a.opened_on, a.closed_on, a.archived_at \
             FROM accounts a JOIN subtree s ON a.parent_id = s.id \
         ) SELECT id, name, opened_on, closed_on, archived_at FROM subtree",
    )
    .bind(id.to_string())
    .fetch_all(&mut *conn)
    .await
    .map_err(BcError::from)
}

/// Service for creating and managing accounts.
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

    /// Creates a new account and returns its ID.
    ///
    /// Both the event append and the projection insert are wrapped in a single
    /// SQLite transaction so they succeed or fail atomically.
    ///
    /// # Arguments
    ///
    /// * `name` - Display name for the new account.
    /// * `account_type` - Classification in the chart of accounts.
    /// * `kind` - Account maintenance kind.
    /// * `description` - Optional free-text description.
    /// * `parent_id` - Optional parent account ID for sub-accounts.
    /// * `commodity_ids` - Ordered list of allowed commodity IDs; first entry is the default.
    /// * `tag_ids` - Tags to attach to the account.
    /// * `acquisition_date` - Date the asset was acquired (only for [`AccountKind::ManualAsset`]).
    /// * `acquisition_cost` - Cost of acquisition (only for [`AccountKind::ManualAsset`]).
    /// * `depreciation_policy` - Depreciation method (only for [`AccountKind::ManualAsset`]).
    /// * `opened_on` - Business date the account opened; `None` leaves it undeclared.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on event append or database insert failure.
    /// Returns [`BcError::BadData`] if `acquisition_date`, `acquisition_cost`, or
    /// `depreciation_policy` is `Some` and `kind` is not [`bc_models::AccountKind::ManualAsset`].
    /// Returns [`BcError::BadData`] if `parent_id` names an account whose type
    /// differs from `account_type`.
    /// Returns [`BcError::NotFound`] if `parent_id` names no account.
    #[builder]
    #[inline]
    pub async fn create(
        &self,
        name: &str,
        account_type: AccountType,
        kind: AccountKind,
        description: Option<&str>,
        parent_id: Option<&AccountId>,
        #[builder(default)] commodity_ids: &[CommodityId],
        #[builder(default)] tag_ids: &[TagId],
        acquisition_date: Option<jiff::civil::Date>,
        acquisition_cost: Option<rust_decimal::Decimal>,
        depreciation_policy: Option<&bc_models::DepreciationPolicy>,
        opened_on: Option<jiff::civil::Date>,
    ) -> BcResult<AccountId> {
        let mut tx = self.pool.begin().await?;
        let id = create_in_tx(
            &mut tx,
            name,
            account_type,
            kind,
            description,
            parent_id,
            commodity_ids,
            tag_ids,
            acquisition_date,
            acquisition_cost,
            depreciation_policy,
            opened_on,
        )
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Archives an account by setting its `archived_at` timestamp.
    ///
    /// An archived account may not have an active descendant. With `cascade`
    /// false this rejects when any descendant is still active, naming them;
    /// with `cascade` true every active descendant is archived alongside `id`
    /// in the same transaction. `id` being already archived does not stop the
    /// cascade — an already-archived parent with active descendants is exactly
    /// the state `cascade` repairs.
    ///
    /// The event append and the projection UPDATE are wrapped in a single SQLite
    /// transaction so they succeed or fail atomically.  `rows_affected()` is used
    /// to detect a missing or already-archived target without a separate pre-check,
    /// eliminating a TOCTOU race.
    ///
    /// # Arguments
    ///
    /// * `id` - The account to archive.
    /// * `cascade` - Whether to archive active descendants too, or reject on them.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::BadData`] if `cascade` is [`Cascade::Reject`] and any
    /// descendant is still active. This check runs first, so an already-archived
    /// account with active descendants reports the descendants rather than
    /// [`BcError::AlreadyArchived`] — the blocking subtree is the more useful
    /// thing to name.
    /// Returns [`BcError::AlreadyArchived`] if the account exists, is already
    /// archived, and nothing blocked the call.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn archive(&self, id: &AccountId, cascade: Cascade) -> BcResult<()> {
        let now = Timestamp::now();

        let mut tx = self.pool.begin().await?;

        let descendants = descendants_of(&mut tx, id).await?;
        let active: Vec<&Descendant> = descendants.iter().filter(|d| d.is_active()).collect();

        reject_blocking_descendants(&active, cascade, "archive", "active")?;

        let result =
            sqlx::query("UPDATE accounts SET archived_at = ? WHERE id = ? AND archived_at IS NULL")
                .bind(now.to_string())
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;

        let target_archived = result.rows_affected() > 0;

        if target_archived {
            insert_event(&Event::AccountArchived { id: id.clone() }, &mut tx).await?;
        } else {
            // rows_affected == 0 means the UPDATE found no matching row.
            // Perform a follow-up SELECT to distinguish "not found" from
            // "already archived" so callers get a semantic error.
            let exists: bool = sqlx::query_scalar("SELECT count(*) > 0 FROM accounts WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;

            if !exists {
                return Err(BcError::NotFound(id.to_string()));
            }
            if !cascade.is_into() {
                return Err(BcError::AlreadyArchived(id.clone()));
            }
            // `cascade` is `Into` and the target is merely already archived —
            // that is not a reason to leave its active descendants alone, so
            // fall through to the cascade below without an event for `id`
            // (nothing about it actually changed).
        }

        let mut archived = usize::from(target_archived);
        if cascade.is_into() {
            for descendant in &active {
                let descendant_id = descendant.id.parse::<AccountId>().map_err(|e| {
                    BcError::BadData(format!("invalid account id '{}': {e}", descendant.id))
                })?;
                insert_event(
                    &Event::AccountArchived {
                        id: descendant_id.clone(),
                    },
                    &mut tx,
                )
                .await?;
                sqlx::query(
                    "UPDATE accounts SET archived_at = ? WHERE id = ? AND archived_at IS NULL",
                )
                .bind(now.to_string())
                .bind(descendant_id.to_string())
                .execute(&mut *tx)
                .await?;
                archived = archived.saturating_add(1);
            }
        }

        tx.commit().await?;
        tracing::info!(account_id = %id, archived, "account archived");
        Ok(())
    }

    /// Closes an account on a business date.
    ///
    /// A closed account may not have an open descendant. With `cascade` false
    /// this rejects when any descendant is still open, naming them; with
    /// `cascade` true the same `closed_on` is stamped on every open descendant
    /// in the same transaction.
    ///
    /// Closing does not archive. `archived_at` controls visibility in active
    /// lists; an account closed years ago is still wanted in reports covering
    /// the years it was open.
    ///
    /// # Arguments
    ///
    /// `on` may not precede the account's own `opened_on`, nor that of any
    /// descendant a cascade would stamp: an account that closed before it
    /// opened has no date inside its declared life, so every posting into it
    /// would warn.
    ///
    /// # Arguments
    ///
    /// * `id` - The account to close.
    /// * `on` - The business date it closed.
    /// * `cascade` - Whether to close open descendants too, or reject on them.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::AlreadyClosed`] if the account exists but is already closed.
    /// Returns [`BcError::BadData`] if `cascade` is [`Cascade::Reject`] and any
    /// descendant is still open, or if `on` precedes the opening date of the
    /// account or of any descendant being closed.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn close(
        &self,
        id: &AccountId,
        on: jiff::civil::Date,
        cascade: Cascade,
    ) -> BcResult<()> {
        let mut tx = self.pool.begin().await?;

        let descendants = descendants_of(&mut tx, id).await?;
        let open: Vec<&Descendant> = descendants.iter().filter(|d| d.is_open()).collect();

        reject_blocking_descendants(&open, cascade, "close", "open")?;

        let target: Option<(String, Option<String>)> =
            sqlx::query_as("SELECT name, opened_on FROM accounts WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        if let Some((name, opened_on)) = target {
            reject_close_before_open(on, opened_on.as_deref(), &name)?;
        }
        if cascade.is_into() {
            for descendant in &open {
                reject_close_before_open(on, descendant.opened_on.as_deref(), &descendant.name)?;
            }
        }

        let result =
            sqlx::query("UPDATE accounts SET closed_on = ? WHERE id = ? AND closed_on IS NULL")
                .bind(on.to_string())
                .bind(id.to_string())
                .execute(&mut *tx)
                .await?;

        let target_closed = result.rows_affected() > 0;

        if target_closed {
            insert_event(
                &Event::AccountClosed {
                    id: id.clone(),
                    closed_on: on,
                },
                &mut tx,
            )
            .await?;
        } else {
            // rows_affected == 0 means the UPDATE found no matching row.
            // Perform a follow-up SELECT to distinguish "not found" from
            // "already closed" so callers get a semantic error.
            let exists: bool = sqlx::query_scalar("SELECT count(*) > 0 FROM accounts WHERE id = ?")
                .bind(id.to_string())
                .fetch_one(&mut *tx)
                .await?;

            if !exists {
                return Err(BcError::NotFound(id.to_string()));
            }
            if !cascade.is_into() {
                return Err(BcError::AlreadyClosed(id.clone()));
            }
            // `cascade` is `Into` and the target is merely already closed —
            // that is not a reason to leave its open descendants alone, so
            // fall through to the cascade below without an event for `id`
            // (nothing about it actually changed).
        }

        let mut closed = usize::from(target_closed);
        if cascade.is_into() {
            for descendant in &open {
                let descendant_id = descendant.id.parse::<AccountId>().map_err(|e| {
                    BcError::BadData(format!("invalid account id '{}': {e}", descendant.id))
                })?;
                insert_event(
                    &Event::AccountClosed {
                        id: descendant_id.clone(),
                        closed_on: on,
                    },
                    &mut tx,
                )
                .await?;
                sqlx::query("UPDATE accounts SET closed_on = ? WHERE id = ? AND closed_on IS NULL")
                    .bind(on.to_string())
                    .bind(descendant_id.to_string())
                    .execute(&mut *tx)
                    .await?;
                closed = closed.saturating_add(1);
            }
        }

        tx.commit().await?;
        tracing::info!(account_id = %id, closed_on = %on, closed, "account closed");
        Ok(())
    }

    /// Clears an account's `closed_on`, reopening it.
    ///
    /// Rejects when the parent is closed, holding the "no open descendant of a
    /// closed account" invariant from the other direction.
    ///
    /// # Arguments
    ///
    /// * `id` - The account to reopen.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::NotClosed`] if the account is not currently closed.
    /// Returns [`BcError::BadData`] if the account's parent is closed.
    /// Returns [`BcError`] on event append or database update failure.
    #[inline]
    pub async fn reopen(&self, id: &AccountId) -> BcResult<()> {
        let mut tx = self.pool.begin().await?;

        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT parent_id, closed_on FROM accounts WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let Some((parent_id, closed_on)) = row else {
            return Err(BcError::NotFound(id.to_string()));
        };
        if closed_on.is_none() {
            return Err(BcError::NotClosed(id.clone()));
        }

        if let Some(parent) = parent_id {
            let parent_closed: Option<String> =
                sqlx::query_scalar("SELECT closed_on FROM accounts WHERE id = ?")
                    .bind(&parent)
                    .fetch_optional(&mut *tx)
                    .await?
                    .flatten();
            if let Some(parent_closed_on) = parent_closed {
                return Err(BcError::BadData(format!(
                    "cannot reopen an account whose parent closed on {parent_closed_on}; \
                     reopen the parent first"
                )));
            }
        }

        insert_event(&Event::AccountReopened { id: id.clone() }, &mut tx).await?;
        sqlx::query("UPDATE accounts SET closed_on = NULL WHERE id = ?")
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        tracing::info!(account_id = %id, "account reopened");
        Ok(())
    }

    /// Sets an account's declared opening date.
    ///
    /// Unconstrained by the parent's dates: an account can be moved between
    /// parents, so a child that opened before its current parent is an ordinary
    /// fact rather than an error. It is constrained by the account's own
    /// `closed_on`, which must not precede it — see [`Self::close`].
    ///
    /// # Arguments
    ///
    /// * `id` - The account to update.
    /// * `opened_on` - The new opening date, or `None` to clear it.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the account does not exist.
    /// Returns [`BcError::BadData`] if `opened_on` falls after the account's
    /// declared `closed_on`.
    /// Returns [`BcError`] on database update failure.
    #[inline]
    pub async fn set_opened_on(
        &self,
        id: &AccountId,
        opened_on: Option<jiff::civil::Date>,
    ) -> BcResult<()> {
        let mut tx = self.pool.begin().await?;

        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT closed_on FROM accounts WHERE id = ?")
                .bind(id.to_string())
                .fetch_optional(&mut *tx)
                .await?;
        let Some((closed_on,)) = row else {
            return Err(BcError::NotFound(id.to_string()));
        };

        if let (Some(opening), Some(raw)) = (opened_on, closed_on.as_deref()) {
            let closing = raw
                .parse::<jiff::civil::Date>()
                .map_err(|e| BcError::BadData(format!("invalid closed_on '{raw}': {e}")))?;
            if opening > closing {
                return Err(BcError::BadData(format!(
                    "cannot set an opening date of {opening} on an account that closed on \
                     {closing}; reopen it or correct the closing date first"
                )));
            }
        }

        sqlx::query("UPDATE accounts SET opened_on = ? WHERE id = ?")
            .bind(opened_on.map(|d| d.to_string()))
            .bind(id.to_string())
            .execute(&mut *tx)
            .await?;

        tx.commit().await?;
        tracing::info!(account_id = %id, "account opened_on set");
        Ok(())
    }

    /// Finds an account by ID, including its commodity and tag associations.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no account with that ID exists.
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn find_by_id(&self, id: &AccountId) -> BcResult<Account> {
        let mut row = sqlx::query_as::<_, AccountRow>(
            "SELECT id, name, account_type, kind, description, parent_id, created_at, archived_at, \
             acquisition_date, acquisition_cost, depreciation_policy, \
             opened_on, closed_on \
             FROM accounts WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?
        .ok_or_else(|| BcError::NotFound(id.to_string()))?;

        let commodity_rows: Vec<(String,)> = sqlx::query_as(
            "SELECT commodity_id FROM account_commodities WHERE account_id = ? ORDER BY position",
        )
        .bind(id.to_string())
        .fetch_all(&self.pool)
        .await?;

        row.commodities = commodity_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<CommodityId>()
                    .map_err(|e| BcError::BadData(format!("invalid commodity_id '{s}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        let tag_rows: Vec<(String,)> =
            sqlx::query_as("SELECT tag_id FROM account_tags WHERE account_id = ?")
                .bind(id.to_string())
                .fetch_all(&self.pool)
                .await?;

        row.tag_ids = tag_rows
            .into_iter()
            .map(|(s,)| {
                s.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid tag_id '{s}': {e}")))
            })
            .collect::<BcResult<_>>()?;

        Account::try_from(row)
    }

    /// Lists all active (non-archived) accounts, ordered by name.
    ///
    /// Commodity and tag associations are loaded in bulk (two additional queries)
    /// to avoid N+1 database round-trips.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_active(&self) -> BcResult<Vec<Account>> {
        let mut account_rows = sqlx::query_as::<_, AccountRow>(
            "SELECT id, name, account_type, kind, description, parent_id, created_at, archived_at, \
             acquisition_date, acquisition_cost, depreciation_policy, \
             opened_on, closed_on \
             FROM accounts WHERE archived_at IS NULL ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        if account_rows.is_empty() {
            return Ok(vec![]);
        }

        // Load all commodity associations for active accounts in one query.
        let commodity_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT ac.account_id, ac.commodity_id \
             FROM account_commodities ac \
             JOIN accounts a ON ac.account_id = a.id \
             WHERE a.archived_at IS NULL \
             ORDER BY ac.account_id, ac.position",
        )
        .fetch_all(&self.pool)
        .await?;

        // Load all tag associations for active accounts in one query.
        let tag_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT at.account_id, at.tag_id \
             FROM account_tags at \
             JOIN accounts a ON at.account_id = a.id \
             WHERE a.archived_at IS NULL",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut commodities_map = build_commodities_map(commodity_rows)?;
        let mut tags_map = build_tags_map(tag_rows)?;

        for row in &mut account_rows {
            row.commodities = commodities_map.remove(&row.id).unwrap_or_default();
            row.tag_ids = tags_map.remove(&row.id).unwrap_or_default();
        }

        account_rows.into_iter().map(Account::try_from).collect()
    }

    /// Lists every account, including archived ones, ordered by name.
    ///
    /// Import path resolution needs archived accounts in the map: resolving a
    /// path to an archived account is allowed (with a warning) rather than
    /// treated as missing, since the account genuinely exists.
    ///
    /// Commodity and tag associations are loaded in bulk (two additional
    /// queries) to avoid N+1 database round-trips.
    ///
    /// # Returns
    ///
    /// Every [`Account`] in the database, ordered by name ascending.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data parse failure.
    #[inline]
    pub async fn list_all(&self) -> BcResult<Vec<Account>> {
        let mut account_rows = sqlx::query_as::<_, AccountRow>(
            "SELECT id, name, account_type, kind, description, parent_id, created_at, archived_at, \
             acquisition_date, acquisition_cost, depreciation_policy, \
             opened_on, closed_on \
             FROM accounts ORDER BY name ASC",
        )
        .fetch_all(&self.pool)
        .await?;

        if account_rows.is_empty() {
            return Ok(vec![]);
        }

        // Load every commodity association in one query.
        let commodity_rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT account_id, commodity_id \
             FROM account_commodities \
             ORDER BY account_id, position",
        )
        .fetch_all(&self.pool)
        .await?;

        // Load every tag association in one query.
        let tag_rows: Vec<(String, String)> =
            sqlx::query_as("SELECT account_id, tag_id FROM account_tags")
                .fetch_all(&self.pool)
                .await?;

        let mut commodities_map = build_commodities_map(commodity_rows)?;
        let mut tags_map = build_tags_map(tag_rows)?;

        for row in &mut account_rows {
            row.commodities = commodities_map.remove(&row.id).unwrap_or_default();
            row.tag_ids = tags_map.remove(&row.id).unwrap_or_default();
        }

        account_rows.into_iter().map(Account::try_from).collect()
    }

    /// Materialises every path in `specs`, creating only what is missing.
    ///
    /// Every path is resolved and created in one pass over a single snapshot, so
    /// a batch of thousands costs one `SELECT` rather than one per path. Paths
    /// are processed in the order given, and each insert updates the snapshot, so
    /// two paths sharing an ancestor create it once. The whole batch is one
    /// transaction: a failure on any path rolls back every path.
    ///
    /// Missing ancestors are minted as [`AccountKind::Group`] with no other
    /// attributes; only the leaf takes the attributes from its [`PathSpec`].
    /// Every path takes a single account type, resolved in order: the existing
    /// root's type, then the spec's explicit type, then the root segment name.
    ///
    /// An existing leaf is reused rather than re-created, provided nothing the
    /// caller explicitly requested contradicts it. Segment matching is
    /// case-sensitive, matching [`crate::AccountPath`] resolution. The reuse
    /// decision is made against a snapshot taken before the first insert, so two
    /// concurrent calls can still race; `idx_accounts_sibling_unique` rejects the
    /// loser.
    ///
    /// # Arguments
    ///
    /// * `specs` - The paths to materialise, with their leaf attributes.
    ///
    /// # Returns
    ///
    /// A [`Created`] mapping every requested path to its leaf ID, alongside every
    /// path this call brought into existence — auto-created ancestors included.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::InvalidInput`] if a path's root is unrecognised and no
    /// explicit type was given, if an explicit type contradicts an existing root,
    /// or if an existing leaf contradicts an explicitly-requested attribute;
    /// [`BcError::Database`] on query or insert failure; [`BcError::BadData`] if a
    /// stored row cannot be parsed.
    #[inline]
    #[expect(
        clippy::too_many_lines,
        reason = "one pass over segments; splitting would obscure the snapshot invariant"
    )]
    pub async fn create_paths(&self, specs: &[PathSpec]) -> BcResult<Created> {
        if specs.is_empty() {
            return Ok(Created::default());
        }

        let mut known = self.list_all().await?;
        let mut out = Created::default();
        let mut db_tx = self.pool.begin().await?;

        for spec in specs {
            let rendered = spec.path().to_string();
            let segments = spec.path().segments();
            let Some(root) = segments.first() else {
                return Err(BcError::BadData("account path had no segments".to_owned()));
            };

            // Resolution order: an existing root's type, then the explicit type,
            // then the root segment name.
            let existing_root = known
                .iter()
                .find(|a| a.name() == root.as_str() && a.parent_id().is_none())
                .map(Account::account_type);
            let account_type = match (existing_root, spec.account_type()) {
                (Some(stored), Some(requested)) if stored != requested => {
                    return Err(BcError::InvalidInput(format!(
                        "root account '{root}' already exists with type {stored:?}; \
                         '{rendered}' cannot be created as {requested:?}"
                    )));
                }
                (Some(stored), _) => stored,
                (None, Some(requested)) => requested,
                (None, None) => derive_account_type(root).ok_or_else(|| {
                    BcError::InvalidInput(format!(
                        "cannot derive an account type from root segment '{root}'; \
                         expected one of {KNOWN_ROOTS}; pass an explicit type to set it"
                    ))
                })?,
            };

            let mut parent: Option<AccountId> = None;
            let mut walked: Vec<&str> = Vec::new();

            for (index, segment) in segments.iter().enumerate() {
                walked.push(segment.as_str());
                let is_leaf = index.saturating_add(1) == segments.len();

                let existing = known
                    .iter()
                    .find(|a| a.name() == segment.as_str() && a.parent_id() == parent.as_ref())
                    .cloned();

                let id = if let Some(account) = existing {
                    if is_leaf {
                        if let Some(conflict) = conflict_of(&account, spec, account_type, &rendered)
                        {
                            return Err(BcError::InvalidInput(conflict));
                        }
                        if account.archived_at().is_some() {
                            tracing::warn!(
                                account = %rendered,
                                "reusing an archived account"
                            );
                        }
                    }
                    account.id().clone()
                } else {
                    let new_id = if is_leaf {
                        create_in_tx(
                            &mut db_tx,
                            segment,
                            account_type,
                            spec.kind().unwrap_or(AccountKind::DepositAccount),
                            spec.description(),
                            parent.as_ref(),
                            spec.commodity_ids(),
                            spec.tag_ids(),
                            spec.acquisition_date(),
                            spec.acquisition_cost(),
                            spec.depreciation_policy(),
                            None,
                        )
                        .await?
                    } else {
                        create_in_tx(
                            &mut db_tx,
                            segment,
                            account_type,
                            AccountKind::Group,
                            None,
                            parent.as_ref(),
                            &[],
                            &[],
                            None,
                            None,
                            None,
                            None,
                        )
                        .await?
                    };

                    // Keep the snapshot current so a later segment — or a later
                    // path in this batch — resolves against what was just
                    // inserted. The pushed record mirrors every attribute
                    // `create_in_tx` was just given (gated on `is_leaf`, so an
                    // ancestor still gets nothing extra), so a later spec
                    // naming this same leaf compares against the real values
                    // rather than field defaults.
                    known.push(
                        Account::builder()
                            .id(new_id.clone())
                            .name(segment.clone())
                            .account_type(account_type)
                            .kind(if is_leaf {
                                spec.kind().unwrap_or(AccountKind::DepositAccount)
                            } else {
                                AccountKind::Group
                            })
                            .maybe_description(spec.description().filter(|_| is_leaf))
                            .maybe_parent_id(parent.clone())
                            .commodities(if is_leaf {
                                spec.commodity_ids().to_vec()
                            } else {
                                Vec::new()
                            })
                            .tag_ids(if is_leaf {
                                spec.tag_ids().to_vec()
                            } else {
                                Vec::new()
                            })
                            .maybe_acquisition_date(spec.acquisition_date().filter(|_| is_leaf))
                            .maybe_acquisition_cost(spec.acquisition_cost().filter(|_| is_leaf))
                            .maybe_depreciation_policy(
                                spec.depreciation_policy().filter(|_| is_leaf).cloned(),
                            )
                            .build(),
                    );
                    out.created.push(walked.join(":"));
                    new_id
                };
                parent = Some(id);
            }

            if let Some(leaf) = parent {
                out.ids.insert(rendered, leaf);
            }
        }

        db_tx.commit().await?;
        out.created.sort_unstable();
        out.created.dedup();
        Ok(out)
    }

    /// Materialises one path, reusing existing ancestors and creating only the
    /// missing segments. This is the single-path form of [`Self::create_paths`],
    /// which does the work.
    ///
    /// Callers that need to report which ancestors were minted should call
    /// [`Self::create_paths`] with a one-element slice instead — this form
    /// returns only the leaf.
    ///
    /// # Arguments
    ///
    /// * `spec` - The path to materialise, with its leaf attributes.
    ///
    /// # Returns
    ///
    /// The ID of the leaf (last-segment) account.
    ///
    /// # Errors
    ///
    /// As [`Self::create_paths`]. The [`BcError::BadData`] guard on a segmentless
    /// path is unreachable — [`crate::AccountPath`] cannot hold one — and exists
    /// only so the lookup has a total result.
    #[inline]
    pub async fn create_path(&self, spec: &PathSpec) -> BcResult<AccountId> {
        let outcome = self.create_paths(core::slice::from_ref(spec)).await?;
        outcome
            .ids
            .get(&spec.path().to_string())
            .cloned()
            .ok_or_else(|| BcError::BadData("account path had no segments".to_owned()))
    }
}

/// Creates one account inside a caller-supplied transaction.
///
/// This is the single write path for account creation: both
/// [`Service::create`] and [`Service::create_paths`] call it, so an account
/// minted as a path ancestor is indistinguishable from a hand-created one —
/// same `AccountCreated` event, same projection row.
///
/// # Arguments
///
/// * `conn` - The open transaction to write through.
/// * `name` - Display name for the new account.
/// * `account_type` - Classification in the chart of accounts.
/// * `kind` - Account maintenance kind.
/// * `description` - Optional free-text description.
/// * `parent_id` - Optional parent account ID for sub-accounts.
/// * `commodity_ids` - Ordered list of allowed commodity IDs; first entry is the default.
/// * `tag_ids` - Tags to attach to the account.
/// * `acquisition_date` - Date the asset was acquired (only for [`AccountKind::ManualAsset`]).
/// * `acquisition_cost` - Cost of acquisition (only for [`AccountKind::ManualAsset`]).
/// * `depreciation_policy` - Depreciation method (only for [`AccountKind::ManualAsset`]).
/// * `opened_on` - Business date the account opened; `None` leaves it undeclared.
///
/// # Returns
///
/// The ID of the newly created account.
///
/// # Errors
///
/// Returns [`BcError::BadData`] if `acquisition_date`, `acquisition_cost`, or
/// `depreciation_policy` is `Some` and `kind` is not [`AccountKind::ManualAsset`];
/// [`BcError::BadData`] if `parent_id` names an account whose type differs from
/// `account_type`;
/// [`BcError::NotFound`] if `parent_id` names no account;
/// [`BcError::Database`] on event append or insert failure;
/// [`BcError::Serialisation`] if the depreciation policy cannot be encoded.
#[expect(
    clippy::too_many_arguments,
    reason = "mirrors the public builder's parameter set exactly"
)]
async fn create_in_tx(
    conn: &mut sqlx::SqliteConnection,
    name: &str,
    account_type: AccountType,
    kind: AccountKind,
    description: Option<&str>,
    parent_id: Option<&AccountId>,
    commodity_ids: &[CommodityId],
    tag_ids: &[TagId],
    acquisition_date: Option<jiff::civil::Date>,
    acquisition_cost: Option<rust_decimal::Decimal>,
    depreciation_policy: Option<&bc_models::DepreciationPolicy>,
    opened_on: Option<jiff::civil::Date>,
) -> BcResult<AccountId> {
    if kind != AccountKind::ManualAsset
        && (acquisition_date.is_some()
            || acquisition_cost.is_some()
            || depreciation_policy.is_some())
    {
        return Err(BcError::BadData(
            "acquisition and depreciation fields are only valid for ManualAsset accounts".into(),
        ));
    }

    // A child's type must match its parent's. Checking the immediate parent is
    // enough: every write path holds this invariant, so a parent's stored type
    // already equals its root ancestor's. `create_paths` enforces the same rule
    // via `conflict_of`; this keeps the two entry points in agreement.
    if let Some(parent) = parent_id {
        let parent_type_row: Option<String> =
            sqlx::query_scalar("SELECT account_type FROM accounts WHERE id = ?")
                .bind(parent.to_string())
                .fetch_optional(&mut *conn)
                .await?;
        let Some(parent_type_str) = parent_type_row else {
            return Err(BcError::NotFound(parent.to_string()));
        };
        let parent_type = from_db_str::<AccountType>(&parent_type_str)?;
        if parent_type != account_type {
            return Err(BcError::BadData(format!(
                "child account '{name}' has type {account_type:?} but its parent has type \
                 {parent_type:?}; a child must share its root ancestor's type"
            )));
        }
    }

    let id = AccountId::new();
    let now = Timestamp::now();
    let event = Event::AccountCreated {
        id: id.clone(),
        name: name.to_owned(),
        account_type,
        kind,
        description: description.map(str::to_owned),
    };

    insert_event(&event, conn).await?;

    sqlx::query(
        "INSERT INTO accounts (id, name, account_type, kind, description, parent_id, created_at, \
         acquisition_date, acquisition_cost, depreciation_policy, opened_on) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(name)
    .bind(to_db_str(account_type)?)
    .bind(to_db_str(kind)?)
    .bind(description)
    .bind(parent_id.map(AccountId::to_string))
    .bind(now.to_string())
    .bind(acquisition_date.map(|d| d.to_string()))
    .bind(acquisition_cost.map(|c| c.to_string()))
    .bind(
        depreciation_policy
            .map(serde_json::to_string)
            .transpose()
            .map_err(BcError::Serialisation)?,
    )
    .bind(opened_on.map(|d| d.to_string()))
    .execute(&mut *conn)
    .await?;

    for (position, commodity_id) in commodity_ids.iter().enumerate() {
        sqlx::query(
            "INSERT INTO account_commodities (account_id, commodity_id, position) VALUES (?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(commodity_id.to_string())
        .bind(
            i64::try_from(position)
                .map_err(|e| BcError::BadData(format!("commodity position overflow: {e}")))?,
        )
        .execute(&mut *conn)
        .await?;
    }

    crate::tag::insert_account_tags(conn, &id, tag_ids).await?;

    tracing::info!(account_id = %id, %name, "account creation staged in transaction");
    Ok(id)
}

/// The five root segments [`derive_account_type`] recognises, for error messages.
const KNOWN_ROOTS: &str = "Assets, Liabilities, Equity, Income, Expenses";

/// Maps a path's root segment to the account type the whole path takes.
///
/// Matching is **case-sensitive**, matching [`crate::AccountPath`]'s
/// case-sensitive resolution: a creation rule looser than the resolution rule
/// would mint accounts that later fail to resolve.
///
/// # Arguments
///
/// * `root` - The first segment of an account path, e.g. `"Assets"`.
///
/// # Returns
///
/// The derived [`AccountType`], or `None` if `root` is not one of the five
/// recognised roots — in which case the caller must be given an explicit type.
fn derive_account_type(root: &str) -> Option<AccountType> {
    match root {
        "Assets" => Some(AccountType::Asset),
        "Liabilities" => Some(AccountType::Liability),
        "Equity" => Some(AccountType::Equity),
        "Income" => Some(AccountType::Income),
        "Expenses" => Some(AccountType::Expense),
        _ => None,
    }
}

/// One account path to materialise, plus the attributes its **leaf** should carry.
///
/// Every attribute is optional, and absence means *not specified* rather than
/// *use the default*. That distinction is what makes the reuse comparison in
/// [`Service::create_paths`] well-defined: an omitted attribute is never
/// compared against an existing account, so re-running a bare
/// `create_paths` over an existing tree is a clean no-op.
///
/// Ancestors are never configured from a `PathSpec` — they are always minted as
/// [`AccountKind::Group`] with no other attributes.
///
/// # Example
///
/// ```rust
/// use bc_core::AccountPath;
/// use bc_core::PathSpec;
///
/// let spec = PathSpec::builder()
///     .path(AccountPath::parse("Assets:BankA:Checking")?)
///     .build();
/// assert_eq!(spec.path().to_string(), "Assets:BankA:Checking");
/// assert_eq!(spec.kind(), None);
/// # Ok::<(), bc_core::BcError>(())
/// ```
#[derive(bon::Builder, Debug, Clone)]
#[non_exhaustive]
pub struct PathSpec {
    /// The colon-separated path to materialise.
    path: AccountPath,
    /// Explicit account type. `None` derives it from the root segment.
    account_type: Option<AccountType>,
    /// Leaf kind. `None` means unspecified — [`AccountKind::DepositAccount`] when
    /// creating, and not compared when reusing.
    kind: Option<AccountKind>,
    /// Leaf description.
    #[builder(into)]
    description: Option<String>,
    /// Allowed commodities for the leaf; first entry is the default.
    #[builder(default)]
    commodity_ids: Vec<CommodityId>,
    /// Tags to attach to the leaf.
    #[builder(default)]
    tag_ids: Vec<TagId>,
    /// Acquisition date (only for [`AccountKind::ManualAsset`] leaves).
    acquisition_date: Option<jiff::civil::Date>,
    /// Acquisition cost (only for [`AccountKind::ManualAsset`] leaves).
    acquisition_cost: Option<rust_decimal::Decimal>,
    /// Depreciation policy (only for [`AccountKind::ManualAsset`] leaves).
    depreciation_policy: Option<bc_models::DepreciationPolicy>,
}

impl PathSpec {
    /// Returns the path to materialise.
    #[inline]
    #[must_use]
    pub fn path(&self) -> &AccountPath {
        &self.path
    }

    /// Returns the explicit account type, if one was given.
    #[inline]
    #[must_use]
    pub fn account_type(&self) -> Option<AccountType> {
        self.account_type
    }

    /// Returns the requested leaf kind, if one was given.
    #[inline]
    #[must_use]
    pub fn kind(&self) -> Option<AccountKind> {
        self.kind
    }

    /// Returns the requested leaf description, if one was given.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the requested leaf commodities; empty means unspecified.
    #[inline]
    #[must_use]
    pub fn commodity_ids(&self) -> &[CommodityId] {
        &self.commodity_ids
    }

    /// Returns the requested leaf tags; empty means unspecified.
    #[inline]
    #[must_use]
    pub fn tag_ids(&self) -> &[TagId] {
        &self.tag_ids
    }

    /// Returns the requested acquisition date, if one was given.
    #[inline]
    #[must_use]
    pub fn acquisition_date(&self) -> Option<jiff::civil::Date> {
        self.acquisition_date
    }

    /// Returns the requested acquisition cost, if one was given.
    #[inline]
    #[must_use]
    pub fn acquisition_cost(&self) -> Option<rust_decimal::Decimal> {
        self.acquisition_cost
    }

    /// Returns the requested depreciation policy, if one was given.
    #[inline]
    #[must_use]
    pub fn depreciation_policy(&self) -> Option<&bc_models::DepreciationPolicy> {
        self.depreciation_policy.as_ref()
    }
}

/// The outcome of materialising a batch of account paths.
///
/// Re-exported from the crate root as [`crate::CreatedAccounts`].
#[non_exhaustive]
#[derive(Debug, Clone, Default)]
pub struct Created {
    /// Leaf account ID for every requested path, keyed by the path as rendered
    /// by [`crate::AccountPath::to_string`].
    pub ids: HashMap<String, AccountId>,
    /// Every path this call brought into existence, sorted and deduplicated.
    ///
    /// Unlike [`crate::CreatedTags`], this **includes auto-created ancestors**.
    /// A tag ancestor is a bookkeeping artefact, but an account ancestor is a
    /// real account that appears in `account list` and in every report, so the
    /// caller must be told it was minted.
    pub created: Vec<String>,
}

/// Reports how an existing account contradicts an explicitly-requested attribute.
///
/// Only attributes the caller actually specified are compared: an omitted
/// attribute means *don't care*, which is what makes re-running a bare path a
/// clean no-op regardless of how the account was originally configured.
///
/// # Arguments
///
/// * `existing` - The account already stored at this path.
/// * `spec` - The request, whose `Some`/non-empty fields are the ones compared.
/// * `account_type` - The type resolved for this path.
/// * `rendered` - The path, for the error message.
///
/// # Returns
///
/// `None` if nothing contradicts, or a described conflict.
fn conflict_of(
    existing: &Account,
    spec: &PathSpec,
    account_type: AccountType,
    rendered: &str,
) -> Option<String> {
    if existing.account_type() != account_type {
        return Some(format!(
            "account '{rendered}' already exists with type {:?} (requested {account_type:?})",
            existing.account_type()
        ));
    }
    if let Some(kind) = spec.kind()
        && existing.kind() != kind
    {
        return Some(format!(
            "account '{rendered}' already exists with kind {:?} (requested {kind:?})",
            existing.kind()
        ));
    }
    if let Some(description) = spec.description()
        && existing.description() != Some(description)
    {
        return Some(format!(
            "account '{rendered}' already exists with a different description"
        ));
    }
    if spec.acquisition_date().is_some() && existing.acquisition_date() != spec.acquisition_date() {
        return Some(format!(
            "account '{rendered}' already exists with a different acquisition date"
        ));
    }
    if spec.acquisition_cost().is_some() && existing.acquisition_cost() != spec.acquisition_cost() {
        return Some(format!(
            "account '{rendered}' already exists with a different acquisition cost"
        ));
    }
    if spec.depreciation_policy().is_some()
        && existing.depreciation_policy() != spec.depreciation_policy()
    {
        return Some(format!(
            "account '{rendered}' already exists with a different depreciation policy"
        ));
    }
    // Commodity order is significant — the first entry is the account's default
    // — so this compares as an ordered sequence.
    if !spec.commodity_ids().is_empty() && existing.commodities() != spec.commodity_ids() {
        return Some(format!(
            "account '{rendered}' already exists with different commodities"
        ));
    }
    // Tags carry no order, so compare them as sets.
    if !spec.tag_ids().is_empty() {
        let stored: HashSet<&TagId> = existing.tag_ids().iter().collect();
        let requested: HashSet<&TagId> = spec.tag_ids().iter().collect();
        if stored != requested {
            return Some(format!(
                "account '{rendered}' already exists with different tags"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;
    use crate::AccountPath;

    #[rstest]
    #[case::assets("Assets", AccountType::Asset)]
    #[case::liabilities("Liabilities", AccountType::Liability)]
    #[case::equity("Equity", AccountType::Equity)]
    #[case::income("Income", AccountType::Income)]
    #[case::expenses("Expenses", AccountType::Expense)]
    fn derives_the_type_from_a_known_root(#[case] root: &str, #[case] expected: AccountType) {
        assert_eq!(derive_account_type(root), Some(expected));
    }

    #[rstest]
    #[case::unknown_word("Cash")]
    #[case::lowercase("assets")]
    #[case::singular("Asset")]
    #[case::plural_expense("Expense")]
    fn refuses_to_derive_a_type_from_an_unknown_root(#[case] root: &str) {
        // Case-sensitive, matching AccountPath's case-sensitive resolution: a
        // creation rule looser than the resolution rule would mint accounts that
        // later fail to resolve.
        assert_eq!(derive_account_type(root), None, "'{root}' must not derive");
    }

    #[test]
    fn a_path_spec_defaults_every_optional_attribute_to_unspecified() {
        let path = AccountPath::parse("Assets:BankA:Checking").expect("valid path");
        let spec = PathSpec::builder().path(path).build();

        assert_eq!(spec.account_type(), None);
        assert_eq!(spec.kind(), None);
        assert_eq!(spec.description(), None);
        assert!(spec.commodity_ids().is_empty());
        assert!(spec.tag_ids().is_empty());
        assert_eq!(spec.acquisition_date(), None);
        assert_eq!(spec.acquisition_cost(), None);
        assert!(spec.depreciation_policy().is_none());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_via_builder_api(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Savings")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create via builder");
        assert!(id.to_string().starts_with("account_"));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_round_trips_opened_on(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let id = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .opened_on(jiff::civil::date(2020, 1, 1))
            .call()
            .await
            .expect("create account");

        let found = svc.find_by_id(&id).await.expect("find account");
        assert_eq!(found.opened_on(), Some(jiff::civil::date(2020, 1, 1)));
        assert_eq!(found.closed_on(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_rejects_child_type_contradicting_parent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc
            .create()
            .name("Assets")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::Group)
            .call()
            .await
            .expect("create parent");

        let error = svc
            .create()
            .name("CardX")
            .account_type(bc_models::AccountType::Liability)
            .kind(bc_models::AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect_err("a Liability child under an Asset parent must be rejected");

        assert!(
            matches!(error, BcError::BadData(ref m) if m.contains("Liability") && m.contains("Asset")),
            "unexpected error: {error:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_accepts_child_type_matching_parent(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc
            .create()
            .name("Assets")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::Group)
            .call()
            .await
            .expect("create parent");

        svc.create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect("a matching child type must be accepted");
    }

    #[test]
    fn account_kind_round_trips() {
        use bc_models::AccountKind;
        for (kind, expected) in [
            (AccountKind::DepositAccount, "deposit_account"),
            (AccountKind::ManualAsset, "manual_asset"),
            (AccountKind::Receivable, "receivable"),
            (AccountKind::VirtualAllocation, "virtual_allocation"),
        ] {
            let s = to_db_str(kind).expect("known variant should serialise");
            assert_eq!(s, expected);
            let back = from_db_str::<AccountKind>(&s).expect("known string should deserialise");
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn account_kind_from_str_rejects_unknown() {
        from_db_str::<AccountKind>("bogus").expect_err("unknown string should fail");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_persists_projection(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.name(), "Checking");
        assert!(found.is_active());
        assert!(found.commodities().is_empty());
        assert!(found.tag_ids().is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_account_sets_archived_at(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Old Account")
            .account_type(bc_models::AccountType::Liability)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");

        svc.archive(&id, Cascade::Reject)
            .await
            .expect("archive should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert!(!found.is_active());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_with_kind_persists(pool: sqlx::SqlitePool) {
        use bc_models::AccountKind;
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("House")
            .account_type(bc_models::AccountType::Asset)
            .kind(AccountKind::ManualAsset)
            .call()
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.account_type(), bc_models::AccountType::Asset);
        assert_eq!(found.kind(), AccountKind::ManualAsset);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_nonexistent_account_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::AccountId::new();
        let result = svc.archive(&fake_id, Cascade::Reject).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
        // Verify the failed archive did not leave any orphaned events.
        let store = crate::events::SqliteStore::new(pool.clone());
        let events = store
            .replay_for(&fake_id.to_string())
            .await
            .expect("replay should succeed");
        assert!(
            events.is_empty(),
            "failed archive must not leave events in the log"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_already_archived_returns_already_archived(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Savings")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        svc.archive(&id, Cascade::Reject)
            .await
            .expect("first archive should succeed");
        let result = svc.archive(&id, Cascade::Reject).await;
        assert!(matches!(result, Err(BcError::AlreadyArchived(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_by_id_nonexistent_returns_not_found(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let fake_id = bc_models::AccountId::new();
        let result = svc.find_by_id(&fake_id).await;
        assert!(matches!(result, Err(BcError::NotFound(_))));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_account_with_commodities_and_tags(pool: sqlx::SqlitePool) {
        // Insert a commodity row directly since there is no CommodityService yet.
        let commodity_id = bc_models::CommodityId::new();
        sqlx::query("INSERT INTO commodities (id, code, decimals, is_iso, symbol_after) VALUES (?, ?, 2, 1, 0)")
            .bind(commodity_id.to_string())
            .bind("USD")
            .execute(&pool)
            .await
            .expect("commodity insert should succeed");

        // Insert a tag row directly since there is no TagService yet.
        let tag_id = bc_models::TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
            .bind(tag_id.to_string())
            .bind("savings")
            .bind(jiff::Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("tag insert should succeed");

        let svc = Service::new(pool.clone());
        let id = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .commodity_ids(core::slice::from_ref(&commodity_id))
            .tag_ids(core::slice::from_ref(&tag_id))
            .call()
            .await
            .expect("create should succeed");

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(found.commodities(), &[commodity_id]);
        assert_eq!(found.tag_ids(), &[tag_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_with_acquisition_cost_for_deposit_account_returns_error(
        pool: sqlx::SqlitePool,
    ) {
        let svc = Service::new(pool);
        let result = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .acquisition_cost(rust_decimal::Decimal::new(100_000, 2))
            .call()
            .await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "expected BadData error, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_with_acquisition_date_for_deposit_account_returns_error(
        pool: sqlx::SqlitePool,
    ) {
        let svc = Service::new(pool);
        let result = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .acquisition_date(jiff::civil::Date::new(2024, 1, 1).expect("valid date"))
            .call()
            .await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "expected BadData error, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_with_depreciation_policy_for_deposit_account_returns_error(
        pool: sqlx::SqlitePool,
    ) {
        let svc = Service::new(pool);
        let result = svc
            .create()
            .name("Checking")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .depreciation_policy(&bc_models::DepreciationPolicy::StraightLine {
                annual_rate: rust_decimal::Decimal::new(2, 1),
            })
            .call()
            .await;
        assert!(
            matches!(result, Err(BcError::BadData(_))),
            "expected BadData error, got: {result:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_manual_asset_with_acquisition_fields_succeeds(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let result = svc
            .create()
            .name("House")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::ManualAsset)
            .acquisition_date(jiff::civil::Date::new(2024, 1, 1).expect("valid date"))
            .acquisition_cost(rust_decimal::Decimal::new(100_000, 0))
            .depreciation_policy(&bc_models::DepreciationPolicy::StraightLine {
                annual_rate: rust_decimal::Decimal::new(2, 1),
            })
            .call()
            .await;
        assert!(result.is_ok(), "expected Ok(AccountId), got: {result:?}");
        let id = result.expect("create should succeed");
        assert!(id.to_string().starts_with("account_"));

        let found = svc.find_by_id(&id).await.expect("find should succeed");
        assert_eq!(
            found.acquisition_date(),
            Some(jiff::civil::Date::new(2024, 1, 1).expect("valid"))
        );
        assert_eq!(
            found.acquisition_cost(),
            Some(rust_decimal::Decimal::new(100_000, 0))
        );
        assert!(
            found.depreciation_policy().is_some(),
            "depreciation_policy should be persisted"
        );
        assert_eq!(
            found.depreciation_policy(),
            Some(&bc_models::DepreciationPolicy::StraightLine {
                annual_rate: rust_decimal::Decimal::new(2, 1),
            })
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_active_excludes_archived(pool: sqlx::SqlitePool) {
        let svc = Service::new(pool.clone());
        let _id1 = svc
            .create()
            .name("Active")
            .account_type(bc_models::AccountType::Asset)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        let id2 = svc
            .create()
            .name("Archived")
            .account_type(bc_models::AccountType::Expense)
            .kind(bc_models::AccountKind::DepositAccount)
            .call()
            .await
            .expect("create should succeed");
        svc.archive(&id2, Cascade::Reject)
            .await
            .expect("archive should succeed");

        let active = svc.list_active().await.expect("list should succeed");
        assert_eq!(active.len(), 1);
        let first = active.first().expect("one active account should exist");
        assert_eq!(first.name(), "Active");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn sibling_accounts_cannot_share_a_name(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let parent = svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create parent");

        svc.create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect("first child");

        let duplicate = svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await;

        assert!(
            duplicate.is_err(),
            "a second sibling named 'Checking' must be rejected so paths stay unambiguous"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn root_accounts_cannot_share_a_name(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("first root");

        let duplicate = svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await;

        assert!(
            duplicate.is_err(),
            "COALESCE(parent_id, '') must de-duplicate roots too"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn list_all_includes_archived_accounts(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let live = svc
            .create()
            .name("Live")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create live");
        let gone = svc
            .create()
            .name("Gone")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create gone");
        svc.archive(&gone, Cascade::Reject).await.expect("archive");

        let active: Vec<AccountId> = svc
            .list_active()
            .await
            .expect("list_active")
            .into_iter()
            .map(|a| a.id().clone())
            .collect();
        assert_eq!(active, vec![live.clone()], "list_active hides archived");

        let mut all: Vec<AccountId> = svc
            .list_all()
            .await
            .expect("list_all")
            .into_iter()
            .map(|a| a.id().clone())
            .collect();
        all.sort_by_key(ToString::to_string);
        let mut expected = vec![live, gone];
        expected.sort_by_key(ToString::to_string);
        assert_eq!(all, expected, "list_all includes archived accounts");
    }

    /// Builds a bare spec for `path`, with no leaf attributes specified.
    fn spec(path: &str) -> PathSpec {
        PathSpec::builder()
            .path(AccountPath::parse(path).expect("valid path"))
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn creates_every_segment_of_a_path_in_an_empty_tree(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let out = svc
            .create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("create the path");

        assert_eq!(
            out.created,
            vec![
                "Assets".to_owned(),
                "Assets:BankA".to_owned(),
                "Assets:BankA:Checking".to_owned(),
            ],
            "every path brought into existence is reported, ancestors included"
        );

        let all = svc.list_all().await.expect("list");
        assert_eq!(all.len(), 3);
        for account in &all {
            assert_eq!(
                account.account_type(),
                AccountType::Asset,
                "every segment inherits the root's type"
            );
        }
        let leaf_id = out.ids.get("Assets:BankA:Checking").expect("leaf id");
        let leaf = all.iter().find(|a| a.id() == leaf_id).expect("leaf");
        assert_eq!(leaf.kind(), AccountKind::DepositAccount);
        for ancestor in all.iter().filter(|a| a.id() != leaf_id) {
            assert_eq!(
                ancestor.kind(),
                AccountKind::Group,
                "auto-created ancestors are Group nodes"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reuses_existing_ancestors_and_mints_only_the_gap(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA")])
            .await
            .expect("seed");

        let out = svc
            .create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("extend");

        assert_eq!(
            out.created,
            vec!["Assets:BankA:Checking".to_owned()],
            "only the missing leaf is new"
        );
        assert_eq!(svc.list_all().await.expect("list").len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_paths_sharing_an_ancestor_mint_it_once(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let out = svc
            .create_paths(&[
                spec("Expenses:Food:Restaurants"),
                spec("Expenses:Food:Groceries"),
            ])
            .await
            .expect("create both");

        assert_eq!(
            out.created,
            vec![
                "Expenses".to_owned(),
                "Expenses:Food".to_owned(),
                "Expenses:Food:Groceries".to_owned(),
                "Expenses:Food:Restaurants".to_owned(),
            ],
            "sorted and deduplicated; Expenses:Food appears once"
        );
        assert_eq!(svc.list_all().await.expect("list").len(), 4);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_underivable_root_is_rejected_without_an_explicit_type(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let error = svc
            .create_paths(&[spec("Cash:Wallet")])
            .await
            .expect_err("an unrecognised root must not be guessed");

        assert!(
            matches!(error, BcError::InvalidInput(ref m) if m.contains("Cash") && m.contains("Assets")),
            "the error must name the offending root and the accepted spellings, got: {error:?}"
        );
        assert_eq!(
            svc.list_all().await.expect("list").len(),
            0,
            "a rejected path creates nothing"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_underivable_root_succeeds_with_an_explicit_type(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let out = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Cash:Wallet").expect("valid"))
                .account_type(AccountType::Asset)
                .build()])
            .await
            .expect("explicit type is accepted");

        assert_eq!(
            out.created,
            vec!["Cash".to_owned(), "Cash:Wallet".to_owned()]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_existing_root_type_is_inherited_and_a_conflict_is_rejected(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[PathSpec::builder()
            .path(AccountPath::parse("Cash").expect("valid"))
            .account_type(AccountType::Asset)
            .build()])
            .await
            .expect("seed the root");

        // Inherited: no explicit type needed the second time.
        svc.create_paths(&[spec("Cash:Wallet")])
            .await
            .expect("the existing root's type is inherited");

        let error = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Cash:Card").expect("valid"))
                .account_type(AccountType::Liability)
                .build()])
            .await
            .expect_err("a type contradicting the existing root must be rejected");

        assert!(matches!(error, BcError::InvalidInput(_)), "got: {error:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn re_running_a_bare_path_is_a_no_op(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("first run");

        let out = svc
            .create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("second run");

        assert!(
            out.created.is_empty(),
            "nothing was brought into existence, so nothing is reported"
        );
        assert!(out.ids.contains_key("Assets:BankA:Checking"));
        assert_eq!(svc.list_all().await.expect("list").len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_matching_kind_on_an_existing_leaf_is_a_no_op(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("first run");

        let out = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Assets:BankA:Checking").expect("valid"))
                .kind(AccountKind::DepositAccount)
                .build()])
            .await
            .expect("a matching kind agrees");

        assert!(out.created.is_empty());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_conflicting_kind_on_an_existing_leaf_is_rejected(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("first run");

        let error = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Assets:BankA:Checking").expect("valid"))
                .kind(AccountKind::ManualAsset)
                .build()])
            .await
            .expect_err("a contradicting kind must not be silently dropped");

        assert!(
            matches!(error, BcError::InvalidInput(ref m) if m.contains("Assets:BankA:Checking")),
            "the error must name the path, got: {error:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn commodities_against_an_existing_leaf_are_rejected(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("first run");

        let error = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Assets:BankA:Checking").expect("valid"))
                .commodity_ids(vec![CommodityId::new()])
                .build()])
            .await
            .expect_err("silently not applying commodities is the failure we prevent");

        assert!(matches!(error, BcError::InvalidInput(_)), "got: {error:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn replaying_a_spec_with_its_own_commodities_and_tags_is_a_no_op(pool: SqlitePool) {
        // Insert a commodity and a tag row directly since neither has a service yet.
        let commodity_id = CommodityId::new();
        sqlx::query("INSERT INTO commodities (id, code, decimals, is_iso, symbol_after) VALUES (?, ?, 2, 1, 0)")
            .bind(commodity_id.to_string())
            .bind("AUD")
            .execute(&pool)
            .await
            .expect("commodity insert");

        let tag_id = TagId::new();
        sqlx::query("INSERT INTO tags (id, name, created_at) VALUES (?, ?, ?)")
            .bind(tag_id.to_string())
            .bind("savings")
            .bind(Timestamp::now().to_string())
            .execute(&pool)
            .await
            .expect("tag insert");

        let svc = Service::new(pool.clone());
        let build = || {
            PathSpec::builder()
                .path(AccountPath::parse("Assets:BankA:Checking").expect("valid"))
                .commodity_ids(vec![commodity_id.clone()])
                .tag_ids(vec![tag_id.clone()])
                .build()
        };

        let first = svc.create_paths(&[build()]).await.expect("first run");
        let leaf = first
            .ids
            .get("Assets:BankA:Checking")
            .expect("leaf")
            .clone();

        // Re-running the identical spec must reuse the leaf rather than reject
        // it: the requested commodities and tags are exactly what it already
        // carries, so nothing contradicts.
        let second = svc
            .create_paths(&[build()])
            .await
            .expect("an identical replay contradicts nothing");

        assert!(second.created.is_empty(), "nothing new should be created");
        assert_eq!(second.ids.get("Assets:BankA:Checking"), Some(&leaf));

        let found = svc.find_by_id(&leaf).await.expect("find");
        assert_eq!(found.commodities(), &[commodity_id]);
        assert_eq!(found.tag_ids(), &[tag_id]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_archived_leaf_is_reused_not_recreated(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let first = svc
            .create_paths(&[spec("Assets:BankA:OldCard")])
            .await
            .expect("first run");
        let leaf = first.ids.get("Assets:BankA:OldCard").expect("leaf").clone();
        svc.archive(&leaf, Cascade::Reject).await.expect("archive");

        let out = svc
            .create_paths(&[spec("Assets:BankA:OldCard")])
            .await
            .expect("an archived account exists, so it is reused");

        assert!(out.created.is_empty());
        assert_eq!(out.ids.get("Assets:BankA:OldCard"), Some(&leaf));
        assert_eq!(svc.list_all().await.expect("list").len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn every_minted_account_gets_an_account_created_event(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA:Checking")])
            .await
            .expect("create");

        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE kind = 'AccountCreated'")
                .fetch_one(&pool)
                .await
                .expect("count events");

        assert_eq!(
            count, 3,
            "ancestors are real accounts, so each gets its own event"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_failure_mid_batch_leaves_no_orphan_ancestors(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let error = svc
            .create_paths(&[spec("Assets:BankA:Checking"), spec("Cash:Wallet")])
            .await
            .expect_err("the second path has an underivable root");

        assert!(matches!(error, BcError::InvalidInput(_)), "got: {error:?}");
        assert_eq!(
            svc.list_all().await.expect("list").len(),
            0,
            "the whole batch is one transaction, so the first path rolls back too"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn segment_matching_is_case_sensitive(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        svc.create_paths(&[spec("Assets:BankA")])
            .await
            .expect("seed");

        let out = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Assets:banka").expect("valid"))
                .build()])
            .await
            .expect("create");

        assert_eq!(
            out.created,
            vec!["Assets:banka".to_owned()],
            "'banka' does not reuse 'BankA' — resolution is case-sensitive"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_path_returns_the_leaf_id(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let id = svc
            .create_path(&spec("Assets:BankA:Checking"))
            .await
            .expect("create");

        let all = svc.list_all().await.expect("list");
        let leaf = all
            .iter()
            .find(|a| a.name() == "Checking")
            .expect("the leaf");
        assert_eq!(&id, leaf.id());
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_same_leaf_path_twice_in_one_batch_agrees_with_itself(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let make_spec = || {
            PathSpec::builder()
                .path(AccountPath::parse("Assets:BankA:House").expect("valid"))
                .kind(AccountKind::ManualAsset)
                .acquisition_date(jiff::civil::Date::new(2024, 1, 1).expect("valid date"))
                .acquisition_cost(rust_decimal::Decimal::new(100_000, 0))
                .build()
        };

        let out = svc
            .create_paths(&[make_spec(), make_spec()])
            .await
            .expect("two identical specs for the same leaf must not conflict with each other");

        assert_eq!(
            out.created,
            vec![
                "Assets".to_owned(),
                "Assets:BankA".to_owned(),
                "Assets:BankA:House".to_owned(),
            ],
            "the leaf is created once even though it was requested twice"
        );
        assert_eq!(svc.list_all().await.expect("list").len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn valuation_fields_round_trip_through_create_paths(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let date = jiff::civil::Date::new(2023, 6, 15).expect("valid date");
        let cost = rust_decimal::Decimal::new(250_000, 2);
        let policy = bc_models::DepreciationPolicy::StraightLine {
            annual_rate: rust_decimal::Decimal::new(15, 2),
        };

        let out = svc
            .create_paths(&[PathSpec::builder()
                .path(AccountPath::parse("Assets:BankA:House").expect("valid"))
                .kind(AccountKind::ManualAsset)
                .acquisition_date(date)
                .acquisition_cost(cost)
                .depreciation_policy(policy.clone())
                .build()])
            .await
            .expect("create a manual asset leaf with valuation fields");

        let leaf_id = out.ids.get("Assets:BankA:House").expect("leaf id");
        let leaf = svc.find_by_id(leaf_id).await.expect("find leaf");

        assert_eq!(leaf.acquisition_date(), Some(date), "acquisition date");
        assert_eq!(leaf.acquisition_cost(), Some(cost), "acquisition cost");
        assert_eq!(
            leaf.depreciation_policy(),
            Some(&policy),
            "depreciation policy"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn ancestors_do_not_inherit_the_leaf_attributes(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let date = jiff::civil::Date::new(2022, 3, 10).expect("valid date");
        let cost = rust_decimal::Decimal::new(500_000, 2);
        let policy = bc_models::DepreciationPolicy::StraightLine {
            annual_rate: rust_decimal::Decimal::new(2, 1),
        };

        svc.create_paths(&[PathSpec::builder()
            .path(AccountPath::parse("Assets:BankA:Checking").expect("valid"))
            .description("Everyday spending account")
            .build()])
            .await
            .expect("create with a description");

        let all = svc.list_all().await.expect("list");
        let leaf = all.iter().find(|a| a.name() == "Checking").expect("leaf");
        let ancestors: Vec<_> = all.iter().filter(|a| a.id() != leaf.id()).collect();

        assert_eq!(
            leaf.description(),
            Some("Everyday spending account"),
            "the leaf carries the requested description"
        );
        for ancestor in &ancestors {
            assert_eq!(
                ancestor.description(),
                None,
                "an auto-created ancestor must not inherit the leaf's description"
            );
        }

        // Same guard, same risk, for the leaf's valuation fields: a regression
        // that let an ancestor inherit them would pass every other test.
        svc.create_paths(&[PathSpec::builder()
            .path(AccountPath::parse("Assets:BankA:House").expect("valid"))
            .kind(AccountKind::ManualAsset)
            .acquisition_date(date)
            .acquisition_cost(cost)
            .depreciation_policy(policy)
            .build()])
            .await
            .expect("create a second manual asset leaf");

        let after_house = svc.list_all().await.expect("list");
        let house = after_house
            .iter()
            .find(|a| a.name() == "House")
            .expect("house");
        for ancestor in after_house
            .iter()
            .filter(|a| a.id() != house.id() && a.id() != leaf.id())
        {
            assert_eq!(
                ancestor.acquisition_date(),
                None,
                "an auto-created ancestor must not inherit the leaf's acquisition date"
            );
            assert_eq!(
                ancestor.acquisition_cost(),
                None,
                "an auto-created ancestor must not inherit the leaf's acquisition cost"
            );
            assert!(
                ancestor.depreciation_policy().is_none(),
                "an auto-created ancestor must not inherit the leaf's depreciation policy"
            );
        }
    }

    /// Builds `Assets` -> `Assets:BankA` -> `Assets:BankA:Checking`, returning
    /// the three ids in that order.
    async fn three_deep(svc: &Service) -> (AccountId, AccountId, AccountId) {
        let assets = svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Group)
            .call()
            .await
            .expect("create Assets");
        let bank_a = svc
            .create()
            .name("BankA")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Group)
            .parent_id(&assets)
            .call()
            .await
            .expect("create BankA");
        let checking = svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&bank_a)
            .call()
            .await
            .expect("create Checking");
        (assets, bank_a, checking)
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_rejects_an_account_with_open_descendants(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, _checking) = three_deep(&svc).await;

        let error = svc
            .close(&bank_a, jiff::civil::date(2024, 6, 30), Cascade::Reject)
            .await
            .expect_err("closing a parent with an open child must be rejected");

        assert!(
            matches!(error, BcError::BadData(ref m) if m.contains("Checking")),
            "the error must name the open descendants: {error:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_with_cascade_closes_every_descendant(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, checking) = three_deep(&svc).await;

        svc.close(&bank_a, jiff::civil::date(2024, 6, 30), Cascade::Into)
            .await
            .expect("cascade close");

        for id in [&bank_a, &checking] {
            let account = svc.find_by_id(id).await.expect("find");
            assert_eq!(account.closed_on(), Some(jiff::civil::date(2024, 6, 30)));
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_leaves_archived_at_alone(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;

        svc.close(&checking, jiff::civil::date(2024, 6, 30), Cascade::Reject)
            .await
            .expect("close");
        let account = svc.find_by_id(&checking).await.expect("find");
        assert_eq!(account.archived_at(), None, "closing must not archive");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_an_already_closed_account_returns_already_closed(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let (_assets, _bank_a, checking) = three_deep(&svc).await;

        svc.close(&checking, jiff::civil::date(2024, 6, 30), Cascade::Reject)
            .await
            .expect("first close");

        let error = svc
            .close(&checking, jiff::civil::date(2024, 7, 31), Cascade::Reject)
            .await
            .expect_err("closing an already-closed account must be rejected");
        assert!(matches!(error, BcError::AlreadyClosed(ref id) if *id == checking));

        // The stored date and the event log must be untouched by the rejected
        // second close.
        let account = svc.find_by_id(&checking).await.expect("find");
        assert_eq!(account.closed_on(), Some(jiff::civil::date(2024, 6, 30)));

        let store = crate::events::SqliteStore::new(pool);
        let events = store
            .replay_for(&checking.to_string())
            .await
            .expect("replay should succeed");
        let closed_count = events.iter().filter(|e| e.kind == "AccountClosed").count();
        assert_eq!(
            closed_count, 1,
            "a rejected re-close must not append a second AccountClosed event"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cascade_close_repairs_an_already_closed_parent(pool: SqlitePool) {
        let svc = Service::new(pool);
        let parent = svc
            .create()
            .name("BankA")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Group)
            .call()
            .await
            .expect("create parent");

        svc.close(&parent, jiff::civil::date(2024, 6, 30), Cascade::Reject)
            .await
            .expect("close the parent while it has no children");

        // Nothing today stops creating a child under an already-closed
        // parent — this reaches exactly the state `cascade` must repair.
        let child = svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect("create a child under a closed parent");

        svc.close(&parent, jiff::civil::date(2024, 7, 31), Cascade::Into)
            .await
            .expect("cascade close repairs an already-closed parent");

        let found = svc.find_by_id(&child).await.expect("find child");
        assert_eq!(
            found.closed_on(),
            Some(jiff::civil::date(2024, 7, 31)),
            "the open child must end up closed"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_rejects_an_account_with_active_descendants(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, _checking) = three_deep(&svc).await;

        let error = svc
            .archive(&bank_a, Cascade::Reject)
            .await
            .expect_err("archiving a parent with an active child must be rejected");
        assert!(
            matches!(error, BcError::BadData(ref m) if m.contains("Checking")),
            "the error must name the active descendants: {error:?}"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn archive_with_cascade_archives_every_descendant(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, checking) = three_deep(&svc).await;

        svc.archive(&bank_a, Cascade::Into)
            .await
            .expect("cascade archive");
        for id in [&bank_a, &checking] {
            let account = svc.find_by_id(id).await.expect("find");
            assert!(account.archived_at().is_some());
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cascade_archive_repairs_an_already_archived_parent(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let parent = svc
            .create()
            .name("BankA")
            .account_type(AccountType::Asset)
            .kind(AccountKind::Group)
            .call()
            .await
            .expect("create parent");

        svc.archive(&parent, Cascade::Reject)
            .await
            .expect("archive the parent while it has no children");

        // Nothing today stops creating a child under an already-archived
        // parent — this reaches exactly the state `cascade` must repair.
        let child = svc
            .create()
            .name("Checking")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&parent)
            .call()
            .await
            .expect("create a child under an archived parent");

        svc.archive(&parent, Cascade::Into)
            .await
            .expect("cascade archive repairs an already-archived parent");

        let found = svc.find_by_id(&child).await.expect("find child");
        assert!(
            found.archived_at().is_some(),
            "the active child must end up archived"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn cascade_close_writes_one_closed_event_per_account(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let (assets, bank_a, checking) = three_deep(&svc).await;

        svc.close(&assets, jiff::civil::date(2024, 6, 30), Cascade::Into)
            .await
            .expect("cascade close");

        let store = crate::events::SqliteStore::new(pool);
        for id in [&assets, &bank_a, &checking] {
            let events = store
                .replay_for(&id.to_string())
                .await
                .expect("replay should succeed");
            let closed_count = events.iter().filter(|e| e.kind == "AccountClosed").count();
            assert_eq!(
                closed_count, 1,
                "expected exactly one AccountClosed event for {id}"
            );
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reopen_rejects_a_child_under_a_closed_parent(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, checking) = three_deep(&svc).await;

        svc.close(&bank_a, jiff::civil::date(2024, 6, 30), Cascade::Into)
            .await
            .expect("cascade close");
        let error = svc
            .reopen(&checking)
            .await
            .expect_err("reopening under a closed parent must be rejected");
        assert!(matches!(error, BcError::BadData(_)), "{error:?}");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reopen_clears_closed_on_when_the_parent_is_open(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;

        svc.close(&checking, jiff::civil::date(2024, 6, 30), Cascade::Reject)
            .await
            .expect("close");
        svc.reopen(&checking).await.expect("reopen");
        let account = svc.find_by_id(&checking).await.expect("find");
        assert_eq!(account.closed_on(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn reopen_an_account_that_is_not_closed_returns_not_closed(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;

        let error = svc
            .reopen(&checking)
            .await
            .expect_err("reopening an account that was never closed must be rejected");
        assert!(matches!(error, BcError::NotClosed(ref id) if *id == checking));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_opened_on_is_unconstrained_by_the_parent(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, checking) = three_deep(&svc).await;

        // A child may open before its parent — accounts get re-parented.
        svc.set_opened_on(&bank_a, Some(jiff::civil::date(2021, 1, 1)))
            .await
            .expect("set parent opened_on");
        svc.set_opened_on(&checking, Some(jiff::civil::date(2019, 1, 1)))
            .await
            .expect("a child opening before its parent is allowed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_before_the_opening_date_is_rejected(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;
        svc.set_opened_on(&checking, Some(jiff::civil::date(2021, 1, 1)))
            .await
            .expect("set opened_on");

        let error = svc
            .close(&checking, jiff::civil::date(2019, 1, 1), Cascade::Reject)
            .await
            .expect_err("closing before opening must be rejected");

        assert!(
            matches!(error, BcError::BadData(ref msg) if msg.contains("before it opened on 2021-01-01")),
            "{error:?}"
        );

        // The rejection left the account open rather than half-applying.
        let account = svc.find_by_id(&checking).await.expect("find");
        assert_eq!(account.closed_on(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn close_on_the_opening_date_is_allowed(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;
        svc.set_opened_on(&checking, Some(jiff::civil::date(2021, 1, 1)))
            .await
            .expect("set opened_on");

        svc.close(&checking, jiff::civil::date(2021, 1, 1), Cascade::Reject)
            .await
            .expect("a same-day open and close is a real, if short, life");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_cascade_is_rejected_by_a_descendant_that_opened_later(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, bank_a, checking) = three_deep(&svc).await;
        svc.set_opened_on(&checking, Some(jiff::civil::date(2024, 1, 1)))
            .await
            .expect("set child opened_on");

        let error = svc
            .close(&bank_a, jiff::civil::date(2022, 6, 30), Cascade::Into)
            .await
            .expect_err("a cascade must not stamp an inverted window on a child");

        assert!(
            matches!(error, BcError::BadData(ref msg) if msg.contains("Checking")),
            "{error:?}"
        );

        // Neither the parent nor the child was closed.
        assert_eq!(
            svc.find_by_id(&bank_a)
                .await
                .expect("find parent")
                .closed_on(),
            None
        );
        assert_eq!(
            svc.find_by_id(&checking)
                .await
                .expect("find child")
                .closed_on(),
            None
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn set_opened_on_after_the_closing_date_is_rejected(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;
        svc.close(&checking, jiff::civil::date(2020, 1, 1), Cascade::Reject)
            .await
            .expect("close");

        let error = svc
            .set_opened_on(&checking, Some(jiff::civil::date(2025, 1, 1)))
            .await
            .expect_err("opening after closing must be rejected");

        assert!(
            matches!(error, BcError::BadData(ref msg) if msg.contains("closed on 2020-01-01")),
            "{error:?}"
        );

        // The stored opening date is untouched.
        let account = svc.find_by_id(&checking).await.expect("find");
        assert_eq!(account.opened_on(), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn clearing_the_opening_date_of_a_closed_account_is_allowed(pool: SqlitePool) {
        let svc = Service::new(pool);
        let (_assets, _bank_a, checking) = three_deep(&svc).await;
        svc.set_opened_on(&checking, Some(jiff::civil::date(2019, 1, 1)))
            .await
            .expect("set opened_on");
        svc.close(&checking, jiff::civil::date(2020, 1, 1), Cascade::Reject)
            .await
            .expect("close");

        svc.set_opened_on(&checking, None)
            .await
            .expect("clearing declares nothing, so it cannot invert the window");
    }
}
