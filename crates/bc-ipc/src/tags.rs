/// A tag presented to the UI: stable ID plus resolved colon-path.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct TagInfo {
    /// Stable tag ID string.
    pub id: String,
    /// Full colon-joined path (e.g. `person:josh`).
    pub path: String,
}

impl TagInfo {
    /// Creates a new [`TagInfo`].
    #[must_use]
    pub fn new(id: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
        }
    }
}
