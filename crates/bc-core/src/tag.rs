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
}
