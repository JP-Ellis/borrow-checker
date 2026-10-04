//! Byte ranges into query text.

/// A half-open byte range `[start, end)` into the query text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Span {
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset one past the last character.
    pub end: usize,
}

impl Span {
    /// Creates the span `[start, end)`.
    ///
    /// # Arguments
    ///
    /// * `start` - Byte offset of the first character.
    /// * `end` - Byte offset one past the last character.
    #[inline]
    #[must_use]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// The smallest span covering `self` and `other`.
    ///
    /// # Arguments
    ///
    /// * `other` - The span to join.
    #[inline]
    #[must_use]
    pub fn to(self, other: Self) -> Self {
        Self::new(self.start.min(other.start), self.end.max(other.end))
    }
}
