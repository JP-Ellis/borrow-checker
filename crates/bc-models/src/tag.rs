//! Tag entity and hierarchical tag path types.
//!
//! These types are re-exported from the crate root with fuller names:
//! - [`crate::Tag`] for [`Tag`]
//! - [`crate::TagId`] for [`TagId`]
//! - [`crate::TagForest`] for [`Forest`]
//! - [`crate::TagPath`] for [`Path`]
//! - [`crate::TagPathError`] for [`ParseError`]
//! - [`crate::TagBuilder`] for the bon-generated [`TagBuilder`]

use core::fmt;
use core::str::FromStr;

use jiff::Timestamp;
use mti::prelude::*;
use serde::Deserialize;
use serde::Serialize;

crate::define_id!(TagId, "tag");

/// A named tag entity with optional parent for hierarchy.
///
/// Re-exported from the crate root as [`crate::TagId`] and [`crate::Tag`].
///
/// # Example
///
/// ```
/// use bc_models::{Tag, TagId};
/// use jiff::Timestamp;
///
/// let tag = Tag::builder()
///     .name("institution")
///     .created_at(Timestamp::now())
///     .build();
///
/// assert_eq!(tag.name(), "institution");
/// assert!(tag.parent_id().is_none());
/// ```
// NOTE: the field docstrings propagate to the setter methods on the builder, so
// keep them accurate and self-contained.
#[derive(bon::Builder, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct Tag {
    /// Stable, opaque identifier for this tag. Assigned by `bc-core` on creation.
    #[builder(default)]
    id: TagId,

    /// Leaf name segment — the final component of the full colon-separated path
    /// (e.g. `"commbank"` for the path `"institution:commbank"`). Must be non-empty.
    /// Use [`crate::TagForest::path_of`] to compute the full path from a forest.
    #[builder(into)]
    name: String,

    /// ID of the parent tag in the hierarchy. `None` means this is a root tag
    /// (e.g. `"institution"`). `Some(id)` means this tag is a child of that tag
    /// (e.g. `"commbank"` under `"institution"`).
    parent_id: Option<TagId>,

    /// Optional human-readable description for this tag. `None` means no
    /// description has been recorded.
    #[builder(into)]
    description: Option<String>,

    /// Timestamp recorded when this tag was first persisted. Defaults to
    /// [`jiff::Timestamp::now()`].
    #[builder(default = jiff::Timestamp::now())]
    created_at: Timestamp,
}

impl Tag {
    /// Returns the tag ID.
    #[inline]
    #[must_use]
    pub fn id(&self) -> &TagId {
        &self.id
    }

    /// Returns the leaf name segment.
    #[inline]
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the parent tag ID, if any.
    #[inline]
    #[must_use]
    pub fn parent_id(&self) -> Option<&TagId> {
        self.parent_id.as_ref()
    }

    /// Returns the description, if any.
    #[inline]
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Returns the creation timestamp.
    #[inline]
    #[must_use]
    pub fn created_at(&self) -> &Timestamp {
        &self.created_at
    }
}

/// A collection of [`Tag`] entities supporting hierarchy traversal.
///
/// Load all tags from `bc-core`, wrap them in a `Forest`, then use the
/// methods here for path computation and tree navigation.
///
/// Re-exported from the crate root as [`crate::TagForest`].
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct Forest(Vec<Tag>);

impl Forest {
    /// Constructs a forest from a flat list of tags (any order).
    #[inline]
    #[must_use]
    pub fn new(tags: Vec<Tag>) -> Self {
        Self(tags)
    }

    /// Looks up a tag by ID.
    #[inline]
    #[must_use]
    pub fn get(&self, id: &TagId) -> Option<&Tag> {
        self.0.iter().find(|t| t.id() == id)
    }

    /// Computes the full colon-separated path for a tag.
    ///
    /// Returns `None` if the tag is not in this forest.
    ///
    /// If a cycle is detected in the parent chain the traversal stops early and
    /// returns the path accumulated so far (which may be a partial path).
    #[inline]
    #[must_use]
    pub fn path_of(&self, id: &TagId) -> Option<Path> {
        let mut segments: Vec<String> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current_id = Some(id.clone());
        while let Some(cid) = current_id {
            if !visited.insert(cid.clone()) {
                // Cycle detected — stop traversal to avoid an infinite loop.
                break;
            }
            let tag = self.get(&cid)?;
            segments.push(tag.name().to_owned());
            current_id = tag.parent_id().cloned();
        }
        segments.reverse();
        Path::new(segments).ok()
    }

    /// Iterates from a tag up to its root (nearest-first, inclusive).
    ///
    /// If a cycle is detected in the parent chain the traversal stops early,
    /// yielding only the tags visited before the cycle was encountered.
    #[inline]
    pub fn ancestors_of(&self, id: &TagId) -> impl Iterator<Item = &Tag> {
        let mut chain: Vec<&Tag> = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut current_id: Option<TagId> = Some(id.clone());
        while let Some(cid) = current_id {
            if !visited.insert(cid.clone()) {
                // Cycle detected — stop traversal to avoid an infinite loop.
                break;
            }
            match self.get(&cid) {
                Some(tag) => {
                    chain.push(tag);
                    current_id = tag.parent_id().cloned();
                }
                None => break,
            }
        }
        chain.into_iter()
    }

    /// Returns the root tag in the ancestor chain of `id`.
    #[inline]
    #[must_use]
    pub fn root_of(&self, id: &TagId) -> Option<&Tag> {
        self.ancestors_of(id).last()
    }

    /// Iterates over tags with the same parent as `id`, excluding `id` itself.
    #[inline]
    pub fn siblings_of<'a>(&'a self, id: &TagId) -> impl Iterator<Item = &'a Tag> {
        let parent = self.get(id).and_then(|t| t.parent_id()).cloned();
        let target = id.clone();
        self.0.iter().filter(move |t| {
            t.id() != &target && t.parent_id().cloned() == parent && parent.is_some()
        })
    }

    /// Iterates over direct children of `id`.
    #[inline]
    pub fn children_of<'a>(&'a self, id: &TagId) -> impl Iterator<Item = &'a Tag> {
        let target = id.clone();
        self.0
            .iter()
            .filter(move |t| t.parent_id() == Some(&target))
    }

    /// Iterates over all descendants of `id` (depth-first).
    #[inline]
    pub fn descendants_of<'a>(&'a self, id: &TagId) -> impl Iterator<Item = &'a Tag> {
        let all_tags: &'a [Tag] = &self.0;
        let mut stack: Vec<&'a Tag> = all_tags
            .iter()
            .filter(|t| t.parent_id() == Some(id))
            .collect();
        core::iter::from_fn(move || {
            let tag = stack.pop()?;
            for child in all_tags.iter().filter(|t| t.parent_id() == Some(tag.id())) {
                stack.push(child);
            }
            Some(tag)
        })
    }
}

/// Error returned when constructing a [`crate::TagPath`] with invalid segments.
///
/// Re-exported from the crate root as [`crate::TagPathError`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    /// The segment list was empty; a tag path requires at least one segment.
    #[error("tag path must contain at least one segment")]
    EmptyPath,
    /// One of the segments was an empty string.
    #[error("segment at index {index} is empty; all segments must be non-empty")]
    EmptySegment {
        /// Zero-based index of the offending segment.
        index: usize,
    },
}

/// A hierarchical tag expressed as an ordered sequence of non-empty segments.
///
/// The canonical display form joins segments with `:`, so `["institution", "commbank"]`
/// displays as `institution:commbank`. The segments are stored explicitly so that code
/// can inspect the hierarchy without parsing a string.
///
/// Re-exported from the crate root as [`crate::TagPath`].
///
/// # Examples
///
/// ```
/// use bc_models::TagPath;
///
/// let tag = TagPath::new(["institution", "commbank"]).expect("valid tag");
/// assert_eq!(tag.to_string(), "institution:commbank");
/// assert!(tag.has_prefix(&["institution"]));
/// assert!(!tag.has_prefix(&["owner"]));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Path(Vec<String>);

impl Path {
    /// Constructs a new [`crate::TagPath`] from an iterator of segment strings.
    ///
    /// # Arguments
    ///
    /// * `segments` - An iterator of non-empty strings forming the tag hierarchy.
    ///
    /// # Returns
    ///
    /// A new tag path if all segments are non-empty.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::EmptyPath`] if the iterator yields no items.
    /// Returns [`ParseError::EmptySegment`] if any segment is an empty string.
    ///
    /// # Example
    ///
    /// ```
    /// use bc_models::TagPath;
    ///
    /// let tag = TagPath::new(["institution", "commbank"]).expect("valid tag");
    /// assert_eq!(tag.segments(), &["institution", "commbank"]);
    /// ```
    #[inline]
    pub fn new(segments: impl IntoIterator<Item = impl Into<String>>) -> Result<Self, ParseError> {
        let collected: Vec<String> = segments.into_iter().map(Into::into).collect();
        if collected.is_empty() {
            return Err(ParseError::EmptyPath);
        }
        for (index, segment) in collected.iter().enumerate() {
            if segment.is_empty() {
                return Err(ParseError::EmptySegment { index });
            }
        }
        Ok(Self(collected))
    }

    /// Returns the individual segments of this tag path.
    ///
    /// # Example
    ///
    /// ```
    /// use bc_models::TagPath;
    ///
    /// let tag = TagPath::new(["owner", "mine"]).expect("valid tag");
    /// assert_eq!(tag.segments(), &["owner", "mine"]);
    /// ```
    #[inline]
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    /// Returns `true` if this tag path starts with the given prefix segments.
    ///
    /// A tag matches its own full path as a prefix (exact match).
    ///
    /// # Arguments
    ///
    /// * `prefix` - The prefix segments to test against.
    ///
    /// # Example
    ///
    /// ```
    /// use bc_models::TagPath;
    ///
    /// let tag = TagPath::new(["institution", "commbank"]).expect("valid tag");
    /// assert!(tag.has_prefix(&["institution"]));
    /// assert!(tag.has_prefix(&["institution", "commbank"]));
    /// assert!(!tag.has_prefix(&["owner"]));
    /// assert!(!tag.has_prefix(&["institution", "commbank", "savings"]));
    /// ```
    #[inline]
    #[must_use]
    pub fn has_prefix(&self, prefix: &[&str]) -> bool {
        self.0.len() >= prefix.len()
            && self
                .0
                .iter()
                .zip(prefix.iter())
                .all(|(seg, pfx)| seg.as_str() == *pfx)
    }
}

impl fmt::Display for Path {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, segment) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(":")?;
            }
            f.write_str(segment)?;
        }
        Ok(())
    }
}

impl FromStr for Path {
    type Err = ParseError;

    /// Parses a colon-separated string into a [`crate::TagPath`].
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::EmptyPath`] if `s` is empty.
    /// Returns [`ParseError::EmptySegment`] if any segment between colons is empty.
    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseError::EmptyPath);
        }
        Self::new(s.split(':'))
    }
}

impl serde::Serialize for Path {
    #[inline]
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for Path {
    #[inline]
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn tag_id_has_correct_prefix() {
        assert!(TagId::new().to_string().starts_with("tag_"));
    }

    #[test]
    fn tag_builder_creates_root_tag() {
        use jiff::Timestamp;
        let tag = Tag::builder()
            .id(TagId::new())
            .name("institution")
            .created_at(Timestamp::now())
            .build();
        assert_eq!(tag.name(), "institution");
        assert!(tag.parent_id().is_none());
    }

    #[test]
    fn tag_forest_path_of_root() {
        use jiff::Timestamp;
        let id = TagId::new();
        let tag = Tag::builder()
            .id(id.clone())
            .name("institution")
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![tag]);
        let path = forest.path_of(&id).expect("tag exists");
        assert_eq!(path.to_string(), "institution");
    }

    #[test]
    fn tag_forest_path_of_child() {
        use jiff::Timestamp;
        let parent_id = TagId::new();
        let child_id = TagId::new();
        let parent = Tag::builder()
            .id(parent_id.clone())
            .name("institution")
            .created_at(Timestamp::now())
            .build();
        let child = Tag::builder()
            .id(child_id.clone())
            .name("commbank")
            .parent_id(parent_id.clone())
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![parent, child]);
        let path = forest.path_of(&child_id).expect("child exists");
        assert_eq!(path.to_string(), "institution:commbank");
    }

    #[test]
    fn tag_forest_root_of_nested_tag() {
        use jiff::Timestamp;
        let root_id = TagId::new();
        let mid_id = TagId::new();
        let leaf_id = TagId::new();
        let root = Tag::builder()
            .id(root_id.clone())
            .name("a")
            .created_at(Timestamp::now())
            .build();
        let mid = Tag::builder()
            .id(mid_id.clone())
            .name("b")
            .parent_id(root_id.clone())
            .created_at(Timestamp::now())
            .build();
        let leaf = Tag::builder()
            .id(leaf_id.clone())
            .name("c")
            .parent_id(mid_id)
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![root, mid, leaf]);
        let found_root = forest.root_of(&leaf_id).expect("root exists");
        assert_eq!(found_root.name(), "a");
    }

    #[test]
    fn tag_forest_siblings() {
        use jiff::Timestamp;
        let parent_id = TagId::new();
        let a_id = TagId::new();
        let b_id = TagId::new();
        let parent = Tag::builder()
            .id(parent_id.clone())
            .name("p")
            .created_at(Timestamp::now())
            .build();
        let a = Tag::builder()
            .id(a_id.clone())
            .name("a")
            .parent_id(parent_id.clone())
            .created_at(Timestamp::now())
            .build();
        let b = Tag::builder()
            .id(b_id.clone())
            .name("b")
            .parent_id(parent_id.clone())
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![parent, a, b]);
        let siblings: Vec<_> = forest.siblings_of(&a_id).collect();
        assert_eq!(siblings.len(), 1);
        assert_eq!(siblings.first().expect("one sibling exists").name(), "b");
    }

    #[test]
    fn tag_forest_descendants() {
        use jiff::Timestamp;
        let root_id = TagId::new();
        let child1_id = TagId::new();
        let child2_id = TagId::new();
        let grandchild_id = TagId::new();
        let root = Tag::builder()
            .id(root_id.clone())
            .name("root")
            .created_at(Timestamp::now())
            .build();
        let c1 = Tag::builder()
            .id(child1_id.clone())
            .name("c1")
            .parent_id(root_id.clone())
            .created_at(Timestamp::now())
            .build();
        let c2 = Tag::builder()
            .id(child2_id.clone())
            .name("c2")
            .parent_id(root_id.clone())
            .created_at(Timestamp::now())
            .build();
        let gc = Tag::builder()
            .id(grandchild_id.clone())
            .name("gc")
            .parent_id(child1_id.clone())
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![root, c1, c2, gc]);
        let descendants: Vec<_> = forest.descendants_of(&root_id).collect();
        // Should include c1, c2, and gc (3 total)
        assert_eq!(descendants.len(), 3);
    }

    #[test]
    fn single_segment_displays_without_colon() {
        let tag = Path::new(["institution"]).expect("valid single-segment tag");
        assert_eq!(tag.to_string(), "institution");
    }

    #[test]
    fn multi_segment_displays_colon_joined() {
        let tag = Path::new(["institution", "commbank"]).expect("valid two-segment tag");
        assert_eq!(tag.to_string(), "institution:commbank");
    }

    #[test]
    fn parses_from_colon_separated_string() {
        let tag: Path = "institution:commbank".parse().expect("valid colon string");
        assert_eq!(tag.segments(), &["institution", "commbank"]);
    }

    #[test]
    fn display_and_fromstr_roundtrip() {
        let original = Path::new(["owner", "mine"]).expect("valid tag");
        let roundtripped: Path = original
            .to_string()
            .parse()
            .expect("display output should be parseable");
        assert_eq!(original, roundtripped);
    }

    #[test]
    fn empty_iterator_is_rejected() {
        let err = Path::new(Vec::<&str>::new()).expect_err("empty iterator should fail");
        assert!(matches!(err, ParseError::EmptyPath));
    }

    #[test]
    fn empty_segment_is_rejected() {
        let err = Path::new(["institution", ""]).expect_err("empty segment should fail");
        assert!(matches!(err, ParseError::EmptySegment { index: 1 }));
    }

    #[test]
    fn has_prefix_matches_exact_path() {
        let tag = Path::new(["institution", "commbank"]).expect("valid tag");
        assert!(tag.has_prefix(&["institution", "commbank"]));
    }

    #[test]
    fn has_prefix_matches_parent_namespace() {
        let tag = Path::new(["institution", "commbank"]).expect("valid tag");
        assert!(tag.has_prefix(&["institution"]));
    }

    #[test]
    fn has_prefix_rejects_different_namespace() {
        let tag = Path::new(["institution", "commbank"]).expect("valid tag");
        assert!(!tag.has_prefix(&["owner"]));
    }

    #[test]
    fn has_prefix_rejects_prefix_longer_than_tag() {
        let tag = Path::new(["institution"]).expect("valid tag");
        assert!(!tag.has_prefix(&["institution", "commbank"]));
    }

    #[test]
    fn serializes_as_colon_string() {
        let tag = Path::new(["institution", "commbank"]).expect("valid tag");
        insta::assert_json_snapshot!(tag, @r#""institution:commbank""#);
    }

    #[test]
    fn deserializes_from_colon_string() {
        let tag: Path = serde_json::from_str(r#""institution:commbank""#)
            .expect("valid JSON string should deserialize");
        assert_eq!(tag.segments(), &["institution", "commbank"]);
    }

    #[test]
    fn ancestors_of_does_not_infinite_loop_on_cycle() {
        use jiff::Timestamp;
        // Build a Forest with a synthetic cycle: tag A's parent is B, B's parent is A.
        let id_a = TagId::new();
        let id_b = TagId::new();
        let tag_a = Tag::builder()
            .id(id_a.clone())
            .name("a")
            .parent_id(id_b.clone())
            .created_at(Timestamp::now())
            .build();
        let tag_b = Tag::builder()
            .id(id_b.clone())
            .name("b")
            .parent_id(id_a.clone())
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![tag_a, tag_b]);

        // ancestors_of must terminate; collect() would hang if it didn't.
        let ancestors: Vec<_> = forest.ancestors_of(&id_a).collect();
        // Both tags are reachable before the cycle is detected.
        assert_eq!(ancestors.len(), 2);
    }

    #[test]
    fn path_of_does_not_infinite_loop_on_cycle() {
        use jiff::Timestamp;
        // Build a Forest with a synthetic cycle: tag A's parent is B, B's parent is A.
        let id_a = TagId::new();
        let id_b = TagId::new();
        let tag_a = Tag::builder()
            .id(id_a.clone())
            .name("a")
            .parent_id(id_b.clone())
            .created_at(Timestamp::now())
            .build();
        let tag_b = Tag::builder()
            .id(id_b.clone())
            .name("b")
            .parent_id(id_a.clone())
            .created_at(Timestamp::now())
            .build();
        let forest = Forest::new(vec![tag_a, tag_b]);

        // path_of must terminate; it will return a partial path rather than loop.
        let _ = forest.path_of(&id_a);
    }
}
