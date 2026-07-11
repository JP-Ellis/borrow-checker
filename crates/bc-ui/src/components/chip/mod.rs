//! Removable chip atom: a labelled pill plus an optional `×` remove button.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]
#![cfg_attr(
    target_arch = "wasm32",
    expect(
        dead_code,
        reason = "Chip/ChipRow and ChipVariant::{Filled,Bare} are wired up by call \
                  sites migrated in later tasks of issue #288"
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
            reason = "chip/remove/row classes are only used in the wasm32-gated components"
        )
    )]
    style,
    "chip.module.scss"
);

/// Visual skin for a [`Chip`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Variant {
    /// Surface-alt background + border, sans caption. Standalone filter chips.
    #[default]
    Outlined,
    /// Surface-accent background, no border, mono caption, tight padding.
    /// Dense inline chips (currency aliases).
    Filled,
    /// No background/border/padding — structure only, for chips nested inside a
    /// container that already carries the pill skin (the tag picker input).
    Bare,
}

impl Variant {
    /// Returns the CSS class for this variant's skin.
    #[must_use]
    #[inline]
    pub fn css_class(self) -> &'static str {
        match self {
            Self::Outlined => style::outlined,
            Self::Filled => style::filled,
            Self::Bare => style::bare,
        }
    }
}

/// A labelled chip with an optional `×` remove button.
///
/// The label is provided via `children`, so it may be plain text or a nested
/// component such as [`TagToken`](crate::components::tag_token::TagToken).
///
/// # Arguments
///
/// * `children` - The label content.
/// * `variant` - Visual skin. Defaults to [`Variant::Outlined`].
/// * `on_remove` - When `Some`, renders the trailing `×` remove button.
/// * `remove_label` - `aria-label` for the remove button. Supply whenever
///   `on_remove` is set.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn Chip(
    /// The label content (plain text or a nested component).
    children: Children,
    /// Visual skin. Defaults to [`Variant::Outlined`].
    #[prop(optional)]
    variant: Variant,
    /// When `Some`, renders the trailing `×` remove button.
    #[prop(optional)]
    on_remove: Option<Callback<()>>,
    /// `aria-label` for the remove button.
    #[prop(optional, into)]
    remove_label: String,
) -> impl IntoView {
    let class = format!("{} {}", style::chip, variant.css_class());
    view! {
        <span class=class>
            {children()}
            {on_remove
                .map(|cb| {
                    view! {
                        <button
                            class=style::remove
                            type="button"
                            aria-label=remove_label
                            on:click=move |_| cb.run(())
                        >
                            "×"
                        </button>
                    }
                })}
        </span>
    }
}

/// A flex-wrap row that lays out a set of [`Chip`]s (and, optionally, a trailing
/// input, as in the currency alias editor).
///
/// # Arguments
///
/// * `children` - The chips (and any siblings) to lay out.
/// * `testid` - Optional `data-testid` for the row element.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn ChipRow(
    /// The chips (and any siblings) to lay out.
    children: Children,
    /// Optional `data-testid` for the row element.
    #[prop(optional)]
    testid: Option<String>,
) -> impl IntoView {
    view! {
        <div class=style::row data-testid=testid>
            {children()}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_ne;

    use super::Variant;

    #[test]
    fn variants_have_distinct_skin_classes() {
        assert_ne!(Variant::Outlined.css_class(), Variant::Filled.css_class());
        assert_ne!(Variant::Outlined.css_class(), Variant::Bare.css_class());
        assert_ne!(Variant::Filled.css_class(), Variant::Bare.css_class());
    }
}
