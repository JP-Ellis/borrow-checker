//! Status indicator pill component.

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "status_pill.module.scss");

/// Semantic tone for a [`StatusPill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Reconciled, on track, cleared, synced.
    Good,
    /// Pending, unallocated, needs attention.
    Warn,
    /// Overspent, failed, negative delta, error.
    Bad,
}

impl Tone {
    /// Returns the CSS class for the pill background and text colour.
    #[must_use]
    #[inline]
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Good => style::good,
            Self::Warn => style::warn,
            Self::Bad => style::bad,
        }
    }
}

/// A semantic status pill: one word + a leading dot in the tone colour.
///
/// # Arguments
///
/// * `label` - One-word status label: `"synced"`, `"pending"`, `"error"`.
/// * `tone` - Semantic [`Tone`] controlling colour.
#[component]
pub fn StatusPill(
    /// One-word status label.
    label: String,
    /// Semantic colour tone.
    tone: Tone,
) -> impl IntoView {
    let class = format!("{} {}", style::pill, tone.css_class());
    view! {
        <span class=class>
            <span class=style::dot></span>
            {label}
        </span>
    }
}

#[cfg(debug_assertions)]
pub mod qa;

#[cfg(test)]
mod tests {
    use super::Tone;

    #[test]
    fn tones_have_distinct_classes() {
        assert_ne!(Tone::Good.css_class(), Tone::Warn.css_class());
        assert_ne!(Tone::Warn.css_class(), Tone::Bad.css_class());
        assert_ne!(Tone::Good.css_class(), Tone::Bad.css_class());
    }
}
