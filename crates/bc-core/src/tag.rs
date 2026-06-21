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
}
