//! Select for a budget's optional tag filter.

/// Pure option-list and ID-to-tag mapping with no framework dependencies.
mod options;

#[cfg(target_arch = "wasm32")]
use bc_ipc::TagInfo;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

/// A `<select>` over the known tags with a leading "none" option.
///
/// Loads the tag catalogue itself. The current selection is always offered,
/// even while the catalogue loads, when it fails to load or when the tag is
/// absent from it, so the selection renders and survives a save. Choosing an
/// option writes the matching [`TagInfo`] to `selection`; "none" writes `None`.
///
/// # Arguments
///
/// * `selection` - The chosen tag filter, read and written by the select.
/// * `class` - CSS class for the `<select>` element.
/// * `disabled` - Disables the select when `true`.
#[cfg(target_arch = "wasm32")]
#[component]
pub fn TagFilterSelect(
    /// The chosen tag filter.
    selection: RwSignal<Option<TagInfo>>,
    /// CSS class for the `<select>` element.
    class: &'static str,
    /// Disables the select.
    #[prop(into, optional)]
    disabled: Signal<bool>,
) -> impl IntoView {
    let catalogue: RwSignal<Vec<TagInfo>> = RwSignal::new(Vec::new());
    let _tags_resource = LocalResource::new(move || async move {
        if let Ok(list) = bc_ipc::client::list_tags().await {
            catalogue.set(list);
        }
    });
    let choices =
        move || selection.with(|cur| catalogue.with(|cat| options::options(cat, cur.as_ref())));
    let selected_id =
        move || selection.with(|s| s.as_ref().map(|t| t.id.clone()).unwrap_or_default());

    view! {
        <select
            class=class
            disabled=move || disabled.get()
            on:change=move |ev| {
                selection.set(options::resolve(&choices(), &event_target_value(&ev)));
            }
            prop:value=selected_id
        >
            <option value="">"none"</option>
            {move || {
                choices()
                    .into_iter()
                    .map(|t| {
                        let id = t.id.clone();
                        view! {
                            <option value=t.id.clone() selected=move || selected_id() == id>
                                {t.path}
                            </option>
                        }
                    })
                    .collect::<Vec<_>>()
            }}
        </select>
    }
}
