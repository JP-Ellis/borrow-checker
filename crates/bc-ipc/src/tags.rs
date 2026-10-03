/// A tag on the wire: its stable ID and its colon-joined path.
///
/// Reads fill `path` from the current hierarchy. Writes read `id` only and
/// never parse `path`, so a tag renamed while a client holds it still
/// resolves to the right tag.
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
