//! Status indicator pill component.
#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
use stylance::import_style;

import_style!(
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            dead_code,
            reason = "dot and pill are only used in the wasm32-gated StatusPill component"
        )
    )]
    style,
    "status_pill.module.scss"
);

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
#[cfg(target_arch = "wasm32")]
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

#[cfg(all(debug_assertions, target_arch = "wasm32"))]
pub mod qa;

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_ne;

    use super::Tone;

    #[test]
    fn tones_have_distinct_classes() {
        assert_ne!(Tone::Good.css_class(), Tone::Warn.css_class());
        assert_ne!(Tone::Warn.css_class(), Tone::Bad.css_class());
        assert_ne!(Tone::Good.css_class(), Tone::Bad.css_class());
    }
}
