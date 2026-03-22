//! Hierarchical tag path types.
//!
//! These types are re-exported from the crate root with fuller names:
//! - [`crate::TagPath`] for [`Path`]
//! - [`crate::TagPathError`] for [`ParseError`]

use core::{fmt, str::FromStr};

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
}
