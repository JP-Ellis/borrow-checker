//! Removable chips for the active global filter dimensions.

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

#[cfg(target_arch = "wasm32")]
use crate::components::ChipVariant;
#[cfg(target_arch = "wasm32")]
use crate::components::chip::Chip;
#[cfg(target_arch = "wasm32")]
use crate::components::chip::ChipRow;

/// Renders each active filter value as its own removable chip. Account and tag
/// chips show the display name cached when the value was picked; an empty filter
/// renders nothing.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn FilterChips() -> impl IntoView {
    let store = crate::filter_ctx::use_filter_store();

    let chips = Signal::derive(move || {
        crate::filter_ctx::chips_from_filter(&store.filter.get(), &store.labels.get())
    });

    view! {
        <Show when=move || !chips.get().is_empty()>
            <ChipRow testid="filter-chips".to_owned()>
                <For each=move || chips.get() key=|c| c.key.clone() let:chip>
                    {
                        let target = chip.remove.clone();
                        let label = chip.label.clone();
                        view! {
                            <Chip
                                variant=ChipVariant::Outlined
                                on_remove=Callback::new(move |()| store.remove_chip(&target))
                                remove_label=format!("remove {label} filter")
                            >
                                {chip.label.clone()}
                            </Chip>
                        }
                    }
                </For>
            </ChipRow>
        </Show>
    }
}
