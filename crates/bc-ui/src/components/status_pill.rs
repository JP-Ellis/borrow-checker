//! Status indicator pill component.

use leptos::prelude::*;

/// Semantic tone for a [`StatusPill`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[expect(dead_code, reason = "to be used in future status pill implementations")]
pub enum Tone {
    /// Reconciled, on track, cleared, synced.
    Good,
    /// Pending, unallocated, needs attention.
    Warn,
    /// Overspent, failed, negative delta, error.
    Bad,
}

impl Tone {
    /// Returns the BEM modifier suffix used in the CSS class.
    #[must_use]
    #[inline]
    pub fn css_modifier(self) -> &'static str {
        match self {
            Self::Good => "good",
            Self::Warn => "warn",
            Self::Bad => "bad",
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
    let class = format!("status-pill status-pill--{}", tone.css_modifier());
    view! {
        <span class=class>
            <span class="status-pill__dot"></span>
            {label}
        </span>
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::Tone;

    #[test]
    fn good_modifier() {
        assert_eq!(Tone::Good.css_modifier(), "good");
    }

    #[test]
    fn warn_modifier() {
        assert_eq!(Tone::Warn.css_modifier(), "warn");
    }

    #[test]
    fn bad_modifier() {
        assert_eq!(Tone::Bad.css_modifier(), "bad");
    }
}
