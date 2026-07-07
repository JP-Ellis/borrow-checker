//! Removable chips for the active global filter dimensions.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

#[cfg(all(target_arch = "wasm32", debug_assertions))]
pub mod qa;

#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "chips.module.scss");

/// Renders the active filter dimensions as removable chips. Empty filter ⇒ nothing.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn FilterChips() -> impl IntoView {
    let store = crate::filter_ctx::use_filter_store();
    let chips = Signal::derive(move || crate::filter_ctx::chips_from_filter(&store.filter.get()));

    view! {
        <div class=style::chips data-testid="filter-chips">
            <For each=move || chips.get() key=|c| c.key let:chip>
                {
                    let key = chip.key;
                    view! {
                        <span class=style::chip>
                            <span class=style::chip_label>{chip.label.clone()}</span>
                            <button
                                class=style::chip_remove
                                aria-label=format!("remove {} filter", chip.key)
                                on:click=move |_| store.clear_dimension(key)
                            >
                                "✕"
                            </button>
                        </span>
                    }
                }
            </For>
        </div>
    }
}
