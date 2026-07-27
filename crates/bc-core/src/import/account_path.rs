//! Colon-separated account paths and their resolution to account IDs.

use crate::BcError;
use crate::BcResult;

/// A validated, colon-separated account path such as `Assets:Bank:Checking`.
///
/// Segments are trimmed of surrounding whitespace but otherwise preserved
/// verbatim: matching against stored account names is exact and
/// case-sensitive, because Beancount capitalises its roots and is itself
/// case-sensitive, so normalising would invent ambiguity rather than remove it.
/// Spaces *inside* a segment are kept, since Ledger allows them in account
/// names.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountPath {
    /// Path segments, root first. Never empty; no segment is empty.
    segments: Vec<String>,
}

impl AccountPath {
    /// Parses a colon-separated account path.
    ///
    /// # Arguments
    ///
    /// * `raw` - The path as written by an importer, e.g. `"Assets:Bank:Checking"`.
    ///
    /// # Returns
    ///
    /// The parsed [`AccountPath`].
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] if `raw` is empty, or if any segment is
    /// empty or whitespace-only — both mean the importer emitted a path no
    /// account could ever match.
    ///
    /// # Example
    ///
    /// ```rust
    /// use bc_core::AccountPath;
    ///
    /// let path = AccountPath::parse("Assets:Bank:Checking")?;
    /// assert_eq!(path.to_string(), "Assets:Bank:Checking");
    /// assert!(AccountPath::parse("Assets::Bank").is_err());
    /// # Ok::<(), bc_core::BcError>(())
    /// ```
    #[inline]
    pub fn parse(raw: &str) -> BcResult<Self> {
        let segments: Vec<String> = raw.split(':').map(|s| s.trim().to_owned()).collect();
        if segments.iter().any(String::is_empty) {
            return Err(BcError::BadData(format!(
                "malformed account path '{raw}': segments must be non-empty"
            )));
        }
        Ok(Self { segments })
    }

    /// Returns the path segments, root first.
    #[inline]
    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.segments
    }
}

impl core::fmt::Display for AccountPath {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.segments.join(":"))
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    #[test]
    fn parses_a_multi_segment_path() {
        let path = AccountPath::parse("Assets:Bank:Checking").expect("valid path");
        assert_eq!(
            path.segments(),
            [
                "Assets".to_owned(),
                "Bank".to_owned(),
                "Checking".to_owned()
            ]
        );
    }

    #[test]
    fn parses_a_single_segment_path() {
        let path = AccountPath::parse("Assets").expect("valid path");
        assert_eq!(path.segments(), ["Assets".to_owned()]);
    }

    #[test]
    fn trims_whitespace_around_segments() {
        let path = AccountPath::parse(" Assets : Bank ").expect("valid path");
        assert_eq!(path.segments(), ["Assets".to_owned(), "Bank".to_owned()]);
    }

    #[test]
    fn preserves_internal_spaces_in_a_segment() {
        // Ledger allows spaces inside an account name.
        let path = AccountPath::parse("Assets:Joint Savings").expect("valid path");
        assert_eq!(
            path.segments(),
            ["Assets".to_owned(), "Joint Savings".to_owned()]
        );
    }

    #[test]
    fn matching_is_case_sensitive_so_case_survives_parsing() {
        let path = AccountPath::parse("assets:bank").expect("valid path");
        assert_eq!(path.segments(), ["assets".to_owned(), "bank".to_owned()]);
    }

    #[rstest]
    #[case::empty("")]
    #[case::only_whitespace("   ")]
    #[case::only_separator(":")]
    #[case::leading_separator(":Assets")]
    #[case::trailing_separator("Assets:")]
    #[case::empty_middle_segment("Assets::Bank")]
    #[case::whitespace_segment("Assets: :Bank")]
    fn rejects_malformed_paths(#[case] raw: &str) {
        assert!(
            AccountPath::parse(raw).is_err(),
            "'{raw}' must be rejected as a malformed account path"
        );
    }

    #[test]
    fn displays_as_a_colon_joined_string() {
        let path = AccountPath::parse(" Assets : Bank ").expect("valid path");
        assert_eq!(path.to_string(), "Assets:Bank");
    }
}
