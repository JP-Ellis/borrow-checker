//! Tag projection service: hierarchy lifecycle, path resolution, and membership.

use bc_models::Tag;
use bc_models::TagForest;
use bc_models::TagId;
use bc_models::TagPath;
use jiff::Timestamp;
use sqlx::SqlitePool;

use crate::BcError;
use crate::BcResult;

/// Raw row from the `tags` table.
#[derive(sqlx::FromRow)]
struct TagRow {
    /// Raw tag ID string.
    id: String,
    /// Leaf name segment.
    name: String,
    /// Raw parent tag ID string, if any.
    parent_id: Option<String>,
    /// Optional description.
    description: Option<String>,
    /// ISO 8601 creation timestamp.
    created_at: String,
}

impl TryFrom<TagRow> for Tag {
    type Error = BcError;

    fn try_from(row: TagRow) -> BcResult<Self> {
        let id = row
            .id
            .parse::<TagId>()
            .map_err(|e| BcError::BadData(format!("invalid tag id '{}': {e}", row.id)))?;
        let parent_id = row
            .parent_id
            .map(|p| {
                p.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid parent tag id '{p}': {e}")))
            })
            .transpose()?;
        let created_at = row.created_at.parse::<Timestamp>().map_err(|e| {
            BcError::BadData(format!("invalid created_at '{}': {e}", row.created_at))
        })?;
        Ok(Tag::builder()
            .id(id)
            .name(row.name)
            .maybe_parent_id(parent_id)
            .maybe_description(row.description)
            .created_at(created_at)
            .build())
    }
}

/// Tag service: owns the `tags` hierarchy and all tag membership join tables.
#[derive(Debug, Clone)]
pub struct Service {
    /// Shared SQLite connection pool.
    pool: SqlitePool,
}

impl Service {
    /// Creates a new [`Service`] with the given connection pool.
    #[must_use]
    #[inline]
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Loads every tag into a [`TagForest`] for path resolution and navigation.
    ///
    /// # Returns
    ///
    /// A forest containing all tags in the database.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if a
    /// stored row cannot be parsed.
    #[inline]
    pub async fn forest(&self) -> BcResult<TagForest> {
        Ok(TagForest::new(self.list().await?))
    }

    /// Lists all tags in creation order.
    ///
    /// # Returns
    ///
    /// Every tag in the database.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if a
    /// stored row cannot be parsed.
    #[inline]
    pub async fn list(&self) -> BcResult<Vec<Tag>> {
        let rows: Vec<TagRow> = sqlx::query_as(
            "SELECT id, name, parent_id, description, created_at FROM tags ORDER BY created_at",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(Tag::try_from).collect()
    }

    /// Finds a tag by ID.
    ///
    /// # Arguments
    ///
    /// * `id` - The tag ID to look up.
    ///
    /// # Returns
    ///
    /// `Some(tag)` if found, `None` otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if the
    /// stored row cannot be parsed.
    #[inline]
    pub async fn find_by_id(&self, id: &TagId) -> BcResult<Option<Tag>> {
        let row: Option<TagRow> = sqlx::query_as(
            "SELECT id, name, parent_id, description, created_at FROM tags WHERE id = ?",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        row.map(Tag::try_from).transpose()
    }

    /// Creates the tag hierarchy for `path`, reusing existing ancestors and
    /// creating only the missing segments. This is the only tag-creation path.
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path to materialise (e.g. `person:josh`).
    ///
    /// # Returns
    ///
    /// The ID of the leaf (last-segment) tag.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query/insert failure or [`BcError::BadData`]
    /// if a stored row cannot be parsed.
    #[inline]
    pub async fn create_path(&self, path: &TagPath) -> BcResult<TagId> {
        let mut conn = self.pool.acquire().await?;
        let mut parent: Option<TagId> = None;
        for segment in path.segments() {
            let parent_str = parent.as_ref().map(TagId::to_string);
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT id FROM tags WHERE name = ? AND parent_id IS ?")
                    .bind(segment)
                    .bind(parent_str.as_deref())
                    .fetch_optional(&mut *conn)
                    .await?;

            let id = if let Some((id,)) = existing {
                id.parse::<TagId>()
                    .map_err(|e| BcError::BadData(format!("invalid tag id '{id}': {e}")))?
            } else {
                let new_id = TagId::new();
                sqlx::query(
                    "INSERT INTO tags (id, name, parent_id, created_at) VALUES (?, ?, ?, ?)",
                )
                .bind(new_id.to_string())
                .bind(segment)
                .bind(parent.as_ref().map(TagId::to_string))
                .bind(Timestamp::now().to_string())
                .execute(&mut *conn)
                .await?;
                new_id
            };
            parent = Some(id);
        }
        parent.ok_or_else(|| BcError::BadData("tag path had no segments".to_owned()))
    }

    /// Resolves a full colon-path to an existing tag ID.
    ///
    /// Walks the hierarchy segment by segment from the root, matching each name
    /// against the children of the previously matched tag.
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path to resolve (e.g. `person:josh`).
    ///
    /// # Returns
    ///
    /// `Some(id)` if a tag with exactly this path exists, `None` otherwise.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Database`] on query failure or [`BcError::BadData`] if a
    /// stored row cannot be parsed.
    #[inline]
    pub async fn find_by_path(&self, path: &TagPath) -> BcResult<Option<TagId>> {
        let tags = self.list().await?;
        Ok(resolve_path_in(&tags, path))
    }

    /// Renames a tag's leaf-name segment. Descendant paths update automatically.
    ///
    /// # Arguments
    ///
    /// * `id` - The tag to rename.
    /// * `new_name` - The new leaf name (must be unique among the tag's siblings).
    ///
    /// # Returns
    ///
    /// Nothing on success.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if the tag does not exist; [`BcError::InvalidInput`]
    /// if a sibling already has `new_name`; [`BcError::Database`] on update failure.
    #[inline]
    pub async fn rename(&self, id: &TagId, new_name: &str) -> BcResult<()> {
        let current = self
            .find_by_id(id)
            .await?
            .ok_or_else(|| BcError::NotFound(format!("tag '{id}'")))?;
        let parent_str = current.parent_id().map(ToString::to_string);

        let clash: Option<(String,)> =
            sqlx::query_as("SELECT id FROM tags WHERE name = ? AND parent_id IS ? AND id <> ?")
                .bind(new_name)
                .bind(parent_str.as_deref())
                .bind(id.to_string())
                .fetch_optional(&self.pool)
                .await?;
        if clash.is_some() {
            return Err(BcError::InvalidInput(format!(
                "a sibling tag named '{new_name}' already exists"
            )));
        }

        sqlx::query("UPDATE tags SET name = ? WHERE id = ?")
            .bind(new_name)
            .bind(id.to_string())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Deletes a tag, its entire subtree, and every membership row referencing any
    /// tag in that subtree.
    ///
    /// Hard-errors if any tag in the subtree is referenced as a budget revision's
    /// `tag_filter`, to avoid silently breaking a budget definition.
    ///
    /// # Arguments
    ///
    /// * `id` - The root of the subtree to delete.
    ///
    /// # Returns
    ///
    /// Nothing on success.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::TagInUse`] if a subtree tag is a budget filter;
    /// [`BcError::Database`] on failure; [`BcError::BadData`] on row parse failure.
    #[inline]
    pub async fn delete(&self, id: &TagId) -> BcResult<()> {
        let forest = self.forest().await?;
        // Root first, then descendants in pre-order: every parent precedes its children.
        let mut subtree: Vec<TagId> = vec![id.clone()];
        subtree.extend(forest.descendants_of(id).map(|t| t.id().clone()));
        let id_strings: Vec<String> = subtree.iter().map(TagId::to_string).collect();

        let mut tx = self.pool.begin().await?;
        for sid in &id_strings {
            let used: Option<(String,)> =
                sqlx::query_as("SELECT id FROM budget_revisions WHERE tag_filter = ?")
                    .bind(sid)
                    .fetch_optional(&mut *tx)
                    .await?;
            if used.is_some() {
                return Err(BcError::TagInUse(format!("tag {sid} is a budget filter")));
            }
        }
        for sid in &id_strings {
            sqlx::query("DELETE FROM account_tags WHERE tag_id = ?")
                .bind(sid)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM transaction_tags WHERE tag_id = ?")
                .bind(sid)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM posting_tags WHERE tag_id = ?")
                .bind(sid)
                .execute(&mut *tx)
                .await?;
        }
        // Delete tags children-first (reverse of root-first pre-order) to satisfy the
        // tags.parent_id self-referential FK under foreign_keys = ON.
        for sid in id_strings.iter().rev() {
            sqlx::query("DELETE FROM tags WHERE id = ?")
                .bind(sid)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Resolves a colon-path to an existing tag ID, erroring if it does not exist.
    ///
    /// Used when saving transactions/postings/accounts: tag references must point
    /// at tags that were created deliberately (no implicit creation from input).
    ///
    /// # Arguments
    ///
    /// * `path` - The hierarchical path to resolve.
    ///
    /// # Returns
    ///
    /// The ID of the existing leaf tag.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::NotFound`] if no tag matches the path; [`BcError::Database`]
    /// on query failure; [`BcError::BadData`] if a stored row cannot be parsed.
    #[inline]
    pub async fn resolve_existing(&self, path: &TagPath) -> BcResult<TagId> {
        self.find_by_path(path)
            .await?
            .ok_or_else(|| BcError::NotFound(format!("tag '{path}'")))
    }
}

/// Resolves a colon-path against an in-memory tag slice, returning the leaf ID.
fn resolve_path_in(tags: &[Tag], path: &TagPath) -> Option<TagId> {
    let mut parent: Option<TagId> = None;
    for segment in path.segments() {
        let found = tags
            .iter()
            .find(|t| t.name() == segment.as_str() && t.parent_id() == parent.as_ref())?;
        parent = Some(found.id().clone());
    }
    parent
}

/// Inserts account↔tag membership rows on the given connection.
///
/// Accepts an executor (not the pool) so it composes into the caller's existing
/// transaction, keeping the aggregate write atomic.
///
/// # Arguments
///
/// * `conn` - An active SQLite connection (typically `&mut *tx`).
/// * `account_id` - The account to attach tags to.
/// * `tag_ids` - The tags to attach.
///
/// # Errors
///
/// Returns [`BcError::Database`] on insert failure.
pub(crate) async fn insert_account_tags(
    conn: &mut sqlx::SqliteConnection,
    account_id: &bc_models::AccountId,
    tag_ids: &[TagId],
) -> BcResult<()> {
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO account_tags (account_id, tag_id) VALUES (?, ?)")
            .bind(account_id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Inserts transaction↔tag membership rows on the given connection.
///
/// # Arguments
///
/// * `conn` - An active SQLite connection (typically `&mut *tx`).
/// * `tx_id` - The transaction to attach tags to.
/// * `tag_ids` - The tags to attach.
///
/// # Errors
///
/// Returns [`BcError::Database`] on insert failure.
pub(crate) async fn insert_transaction_tags(
    conn: &mut sqlx::SqliteConnection,
    tx_id: &bc_models::TransactionId,
    tag_ids: &[TagId],
) -> BcResult<()> {
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO transaction_tags (transaction_id, tag_id) VALUES (?, ?)")
            .bind(tx_id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Inserts posting↔tag membership rows on the given connection.
///
/// # Arguments
///
/// * `conn` - An active SQLite connection (typically `&mut *tx`).
/// * `posting_id` - The posting to attach tags to.
/// * `tag_ids` - The tags to attach.
///
/// # Errors
///
/// Returns [`BcError::Database`] on insert failure.
pub(crate) async fn insert_posting_tags(
    conn: &mut sqlx::SqliteConnection,
    posting_id: &bc_models::PostingId,
    tag_ids: &[TagId],
) -> BcResult<()> {
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO posting_tags (posting_id, tag_id) VALUES (?, ?)")
            .bind(posting_id.to_string())
            .bind(tag_id.to_string())
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    async fn insert_tag(pool: &SqlitePool, id: &TagId, name: &str, parent: Option<&TagId>) {
        sqlx::query("INSERT INTO tags (id, name, parent_id, created_at) VALUES (?, ?, ?, ?)")
            .bind(id.to_string())
            .bind(name)
            .bind(parent.map(ToString::to_string))
            .bind("2026-01-01T00:00:00Z")
            .execute(pool)
            .await
            .expect("tag insert should succeed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn forest_resolves_nested_path(pool: SqlitePool) {
        let person_id = TagId::new();
        let josh_id = TagId::new();
        insert_tag(&pool, &person_id, "person", None).await;
        insert_tag(&pool, &josh_id, "josh", Some(&person_id)).await;
        let svc = Service::new(pool);

        let forest = svc.forest().await.expect("forest loads");
        let path = forest.path_of(&josh_id).expect("path resolves");
        assert_eq!(path.to_string(), "person:josh");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_by_path_matches_existing(pool: SqlitePool) {
        let person_id = TagId::new();
        let josh_id = TagId::new();
        insert_tag(&pool, &person_id, "person", None).await;
        insert_tag(&pool, &josh_id, "josh", Some(&person_id)).await;
        let svc = Service::new(pool);

        let path: TagPath = "person:josh".parse().expect("valid path");
        let id = svc.find_by_path(&path).await.expect("query ok");
        assert_eq!(id, Some(josh_id));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn find_by_path_returns_none_when_missing(pool: SqlitePool) {
        let person_id = TagId::new();
        insert_tag(&pool, &person_id, "person", None).await;
        let svc = Service::new(pool);

        let path: TagPath = "person:bec".parse().expect("valid path");
        assert_eq!(svc.find_by_path(&path).await.expect("query ok"), None);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_path_creates_full_hierarchy(pool: SqlitePool) {
        let svc = Service::new(pool);
        let path: TagPath = "person:josh".parse().expect("valid path");

        let leaf = svc.create_path(&path).await.expect("create ok");

        let forest = svc.forest().await.expect("forest loads");
        assert_eq!(
            forest.path_of(&leaf).map(|p| p.to_string()),
            Some("person:josh".to_owned())
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_path_reuses_existing_ancestors(pool: SqlitePool) {
        let svc = Service::new(pool);
        let josh = svc
            .create_path(&"person:josh".parse().expect("path"))
            .await
            .expect("ok");
        let bec = svc
            .create_path(&"person:bec".parse().expect("path"))
            .await
            .expect("ok");

        let josh_tag = svc.find_by_id(&josh).await.expect("ok").expect("exists");
        let bec_tag = svc.find_by_id(&bec).await.expect("ok").expect("exists");
        assert_eq!(
            josh_tag.parent_id(),
            bec_tag.parent_id(),
            "shared 'person' parent"
        );
        assert_eq!(
            svc.list().await.expect("ok").len(),
            3,
            "person + josh + bec"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn create_path_is_idempotent(pool: SqlitePool) {
        let svc = Service::new(pool);
        let path: TagPath = "a:b:c".parse().expect("path");
        let first = svc.create_path(&path).await.expect("ok");
        let second = svc.create_path(&path).await.expect("ok");
        assert_eq!(first, second);
        assert_eq!(svc.list().await.expect("ok").len(), 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolve_existing_returns_id(pool: SqlitePool) {
        let svc = Service::new(pool);
        let made = svc
            .create_path(&"person:josh".parse().expect("path"))
            .await
            .expect("ok");
        let got = svc
            .resolve_existing(&"person:josh".parse().expect("path"))
            .await
            .expect("ok");
        assert_eq!(got, made);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn resolve_existing_errors_when_missing(pool: SqlitePool) {
        let svc = Service::new(pool);
        let err = svc
            .resolve_existing(&"person:nope".parse().expect("path"))
            .await
            .expect_err("missing tag must error");
        assert!(matches!(err, BcError::NotFound(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_updates_descendant_paths(pool: SqlitePool) {
        let svc = Service::new(pool);
        svc.create_path(&"person:josh".parse().expect("path"))
            .await
            .expect("ok");
        let person = svc
            .find_by_path(&"person".parse().expect("path"))
            .await
            .expect("ok")
            .expect("exists");
        let josh = svc
            .find_by_path(&"person:josh".parse().expect("path"))
            .await
            .expect("ok")
            .expect("exists");

        svc.rename(&person, "people").await.expect("rename ok");

        let forest = svc.forest().await.expect("ok");
        assert_eq!(
            forest.path_of(&josh).map(|p| p.to_string()),
            Some("people:josh".to_owned())
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_rejects_sibling_collision(pool: SqlitePool) {
        let svc = Service::new(pool);
        let josh = svc
            .create_path(&"person:josh".parse().expect("path"))
            .await
            .expect("ok");
        svc.create_path(&"person:bec".parse().expect("path"))
            .await
            .expect("ok");

        let err = svc
            .rename(&josh, "bec")
            .await
            .expect_err("collision must error");
        assert!(matches!(err, BcError::InvalidInput(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rename_missing_tag_errors(pool: SqlitePool) {
        let svc = Service::new(pool);
        let err = svc
            .rename(&TagId::new(), "x")
            .await
            .expect_err("missing must error");
        assert!(matches!(err, BcError::NotFound(_)));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_cascades_subtree_and_memberships(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let person = svc
            .create_path(&"person".parse().expect("path"))
            .await
            .expect("ok");
        let josh = svc
            .create_path(&"person:josh".parse().expect("path"))
            .await
            .expect("ok");

        // account_tags.account_id has an FK, so insert a real account first.
        let account_id = bc_models::AccountId::new();
        sqlx::query(
            "INSERT INTO accounts (id, name, account_type, kind, created_at) VALUES (?, ?, ?, 'deposit_account', ?)",
        )
        .bind(account_id.to_string())
        .bind("Test")
        .bind("asset")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("account insert ok");
        sqlx::query("INSERT INTO account_tags (account_id, tag_id) VALUES (?, ?)")
            .bind(account_id.to_string())
            .bind(josh.to_string())
            .execute(&pool)
            .await
            .expect("membership insert ok");

        svc.delete(&person).await.expect("delete ok");

        assert!(svc.list().await.expect("ok").is_empty(), "subtree removed");
        let remaining: (i64,) = sqlx::query_as("SELECT count(*) FROM account_tags")
            .fetch_one(&pool)
            .await
            .expect("count ok");
        assert_eq!(remaining.0, 0, "memberships removed");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_blocks_when_used_by_budget_filter(pool: SqlitePool) {
        let svc = Service::new(pool.clone());
        let person = svc
            .create_path(&"person".parse().expect("path"))
            .await
            .expect("ok");

        let account_id = bc_models::AccountId::new();
        sqlx::query(
            "INSERT INTO accounts (id, name, account_type, kind, created_at) VALUES (?, ?, ?, 'deposit_account', ?)",
        )
        .bind(account_id.to_string())
        .bind("Test")
        .bind("asset")
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("account insert ok");

        let budget_id = bc_models::BudgetId::new();
        sqlx::query("INSERT INTO budgets (id, account_id, created_at) VALUES (?, ?, ?)")
            .bind(budget_id.to_string())
            .bind(account_id.to_string())
            .bind("2026-01-01T00:00:00Z")
            .execute(&pool)
            .await
            .expect("budget insert ok");

        let revision_id = bc_models::BudgetRevisionId::new();
        sqlx::query(
            "INSERT INTO budget_revisions \
             (id, budget_id, effective_from, name, target_amount, target_currency, \
              period, rollover, tag_filter, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(revision_id.to_string())
        .bind(budget_id.to_string())
        .bind("2026-01-01")
        .bind(None::<String>)
        .bind(None::<String>)
        .bind(None::<String>)
        .bind("monthly")
        .bind("reset_to_zero")
        .bind(person.to_string())
        .bind("2026-01-01T00:00:00Z")
        .execute(&pool)
        .await
        .expect("revision insert ok");

        let err = svc.delete(&person).await.expect_err("must block");
        assert!(matches!(err, BcError::TagInUse(_)));
    }
}
