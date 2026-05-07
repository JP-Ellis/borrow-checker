//! Inline tag badge component.
#![expect(
    clippy::mod_module_files,
    reason = "mod.rs collocates source with its SCSS module file"
)]

use leptos::prelude::*;
use stylance::import_style;

import_style!(style, "tag_token.module.scss");

/// Returns the inline background style for a [`TagToken`] at 12% alpha of
/// the given CSS custom property tone.
#[must_use]
#[inline]
pub fn tag_background_style(tone_var: &str) -> String {
    format!("background-color: color-mix(in srgb, var({tone_var}) 12%, transparent)")
}

/// An inline tag badge with configurable tone.
///
/// # Arguments
///
/// * `label` - Text displayed inside the badge.
/// * `tone_var` - CSS variable name for the colour tone.
///   Defaults to `"--bc-string"` (green).
#[component]
#[expect(
    clippy::needless_pass_by_value,
    reason = "Leptos props must take String for default value support; the prop is used by reference internally"
)]
pub fn TagToken(
    /// The label text to display.
    label: String,
    /// CSS variable for the tone colour. Defaults to `"--bc-string"`.
    #[prop(default = "--bc-string".to_owned())]
    tone_var: String,
) -> impl IntoView {
    let bg = tag_background_style(&tone_var);
    let inline_style = format!("{bg}; color: var({tone_var})");
    view! {
        <span class=style::tag style=inline_style>
            {label}
        </span>
    }
}

#[cfg(test)]
mod tests {
    use super::tag_background_style;

    #[test]
    fn default_tone_references_variable() {
        let s = tag_background_style("--bc-string");
        assert!(
            s.contains("var(--bc-string)"),
            "must reference CSS variable"
        );
        assert!(s.contains("12%"), "alpha must be 12%");
    }

    #[test]
    fn fn_tone() {
        assert!(
            tag_background_style("--bc-fn").contains("var(--bc-fn)"),
            "must reference --bc-fn variable"
        );
    }
}
