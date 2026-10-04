//! Removable chips for the active global filter query.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its QA route module"
    )
)]

#[cfg(all(target_arch = "wasm32", debug_assertions))]
pub mod qa;

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
use crate::components::ChipVariant;
#[cfg(target_arch = "wasm32")]
use crate::components::chip::Chip;
#[cfg(target_arch = "wasm32")]
use crate::components::chip::ChipRow;
#[cfg(target_arch = "wasm32")]
use crate::filter_ctx::Edit;

import_style!(
    #[cfg_attr(
        not(target_arch = "wasm32"),
        expect(
            dead_code,
            reason = "label and edit-query classes are only used in the wasm32-gated component"
        )
    )]
    style,
    "filter_chips.module.scss"
);

/// Renders one removable chip per top-level conjunct of the query. Picked
/// account chips show the short label cached when the account was picked, with
/// the canonical text on hover; an empty query renders nothing.
///
/// Each chip's label opens the palette on that conjunct, and the row's last
/// button opens it on the whole query.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn FilterChips(
    /// `data-testid` for the chip row.
    #[prop(default = "filter-chips")]
    testid: &'static str,
) -> impl IntoView {
    let store = crate::filter_ctx::use_filter_store();
    let opener = crate::filter_ctx::use_palette_opener();

    let chips = Signal::derive(move || {
        crate::filter_ctx::chips_from_filter(&store.filter.get(), &store.labels.get())
    });

    view! {
        <Show when=move || !chips.get().is_empty()>
            <ChipRow testid=testid.to_owned()>
                // Keyed rows capture props once; the label and title in the
                // key force a fresh row when a value flips.
                <For
                    each=move || chips.get()
                    key=|c| {
                        format!("{}:{}:{}", c.key, c.label, c.title.as_deref().unwrap_or_default())
                    }
                    let:chip
                >
                    {
                        let target = chip.remove.clone();
                        let edit_text = chip.remove.clone();
                        let label = chip.label.clone();
                        view! {
                            <Chip
                                variant=ChipVariant::Outlined
                                on_remove=Callback::new(move |()| store.remove_chip(&target))
                                remove_label=format!("remove {label} filter")
                                title=chip.title.clone()
                            >
                                <button
                                    type="button"
                                    class=style::label
                                    aria-label=format!("edit {} filter", chip.label)
                                    on:click=move |_| opener.open(Edit::conjunct(&edit_text))
                                >
                                    {chip.label.clone()}
                                </button>
                            </Chip>
                        }
                    }
                </For>
                <button
                    type="button"
                    class=style::edit_query
                    on:click=move |_| {
                        opener.open(Edit::whole(&store.filter.with_untracked(|f| f.query.clone())));
                    }
                >
                    "edit query"
                </button>
            </ChipRow>
        </Show>
    }
}
