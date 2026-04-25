//! Application input mode — Normal, Insert, or Visual.
//!
//! [`AppMode`] lives in [`crate::app::Model`] and is passed to components as
//! a prop on each render so they can style themselves accordingly.

/// The current input mode, inspired by vim's modal editing model.
///
/// - `Normal` — navigation and command keys are active.
/// - `Insert` — key events are routed to the focused input widget; `Esc` returns to Normal.
/// - `Visual` — `j`/`k` extend a selection; `Esc` clears it and returns to Normal.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[non_exhaustive]
#[expect(
    clippy::module_name_repetitions,
    reason = "referenced externally as mode::AppMode; repetition is intentional"
)]
pub enum AppMode {
    /// Navigation and command mode (default).
    #[default]
    Normal,
    /// Text input mode — active while a form overlay is open.
    Insert,
    /// Selection mode — active while building a multi-item selection.
    Visual,
}

impl core::fmt::Display for AppMode {
    #[inline]
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Normal => write!(f, "NORMAL"),
            Self::Insert => write!(f, "INSERT"),
            Self::Visual => write!(f, "VISUAL"),
        }
    }
}
