//! Multi-select tag picker with autocomplete and inline creation.

#![cfg_attr(
    not(target_arch = "wasm32"),
    expect(
        clippy::mod_module_files,
        reason = "mod.rs collocates the component source with its SCSS module file"
    )
)]

#[cfg(all(target_arch = "wasm32", debug_assertions))]
pub mod qa;

/// Pure tag-filtering logic with no framework dependencies.
mod matching;
#[cfg(target_arch = "wasm32")]
use bc_ipc::TagInfo;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;
#[cfg(target_arch = "wasm32")]
use leptos::task::spawn_local;
#[cfg(target_arch = "wasm32")]
pub use matching::exact_path_exists;
#[cfg(target_arch = "wasm32")]
pub use matching::filter_tags;
#[cfg(target_arch = "wasm32")]
use stylance::import_style;

#[cfg(target_arch = "wasm32")]
import_style!(style, "tag_picker.module.scss");

/// A multi-select tag input with autocomplete and inline tag creation.
///
/// Renders the current selection as removable chips plus a text input.
/// Typing filters known tags; if the typed text matches nothing, a
/// "create new" row appears and creates the tag via IPC before adding it.
///
/// # Arguments
///
/// * `tags` - Currently-selected tag paths (read signal).
/// * `all_tags` - All known tags for autocomplete.
/// * `on_add` - Called with a tag path when the user adds a tag.
/// * `on_remove` - Called with a tag path when the user removes a chip.
/// * `on_created` - Called with the new [`TagInfo`] when a tag is created on the fly.
/// * `compact` - When `true`, renders as an inline tagbox with no border or
///   background — suitable for embedding inside a posting extras row. Defaults
///   to `false`, which preserves the full-width bordered appearance.
#[cfg(target_arch = "wasm32")]
#[component]
#[expect(
    clippy::too_many_lines,
    reason = "Leptos view! macro expands to many lines; the logic is straightforward"
)]
pub fn TagPicker(
    /// Currently-selected tag paths.
    tags: Signal<Vec<String>>,
    /// All known tags for autocomplete.
    all_tags: Signal<Vec<TagInfo>>,
    /// Called with a tag path when a tag is added.
    on_add: Callback<String>,
    /// Called with a tag path when a chip is removed.
    on_remove: Callback<String>,
    /// Called with the new [`TagInfo`] when a tag is created on the fly.
    on_created: Callback<TagInfo>,
    /// Render as a compact inline tagbox (no border, no background, auto width).
    #[prop(optional)]
    compact: bool,
) -> impl IntoView {
    let picker_cls = if compact {
        format!("{} {}", style::picker, style::picker_compact)
    } else {
        style::picker.to_owned()
    };
    let chips_row_cls = if compact {
        format!("{} {}", style::chips_row, style::chips_row_compact)
    } else {
        style::chips_row.to_owned()
    };
    let input_cls = if compact {
        format!("{} {}", style::input, style::input_compact)
    } else {
        style::input.to_owned()
    };

    let query = RwSignal::new(String::new());
    let open = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);

    let chips = move || {
        tags.get()
            .into_iter()
            .map(|path| {
                let remove_path = path.clone();
                view! {
                    <span class=style::chip>
                        <crate::components::tag_token::TagToken label=path />
                        <button
                            class=style::chip_remove
                            type="button"
                            on:click=move |_| on_remove.run(remove_path.clone())
                        >
                            "×"
                        </button>
                    </span>
                }
            })
            .collect::<Vec<_>>()
    };

    let suggestions = move || filter_tags(&all_tags.get(), &query.get(), &tags.get());

    let show_create = move || {
        let q = query.get();
        !q.trim().is_empty() && !exact_path_exists(&all_tags.get(), &q)
    };

    view! {
        <div class=picker_cls>
            <div class=chips_row_cls>
                {chips}
                <input
                    class=input_cls
                    prop:value=move || query.get()
                    on:input=move |ev| {
                        query.set(event_target_value(&ev));
                        open.set(true);
                    }
                    on:focus=move |_| open.set(true)
                    on:blur=move |_| open.set(false)
                    data-testid="tag-input"
                    placeholder="add tag"
                />
            </div>
            {move || {
                open.get()
                    .then(|| {
                        let list = suggestions();
                        let has_create = show_create();
                        if list.is_empty() && !has_create {
                            return None;
                        }
                        Some(
                            view! {
                                <ul class=style::menu>
                                    {list
                                        .into_iter()
                                        .map(|t| {
                                            let add_path = t.path.clone();
                                            view! {
                                                <li
                                                    class=style::option
                                                    on:mousedown=move |ev| {
                                                        ev.prevent_default();
                                                        on_add.run(add_path.clone());
                                                        query.set(String::new());
                                                    }
                                                >
                                                    {t.path.clone()}
                                                </li>
                                            }
                                        })
                                        .collect::<Vec<_>>()}
                                    {move || {
                                        has_create
                                            .then(|| {
                                                let create_query = query.get();
                                                view! {
                                                    <li
                                                        class=style::create_option
                                                        on:mousedown=move |ev| {
                                                            ev.prevent_default();
                                                            if saving.get() {
                                                                return;
                                                            }
                                                            saving.set(true);
                                                            error.set(None);
                                                            let path = create_query.trim().to_owned();
                                                            spawn_local(async move {
                                                                match bc_ipc::client::create_tag(&path).await {
                                                                    Ok(id) => {
                                                                        let info = TagInfo::new(id, path.clone());
                                                                        on_created.run(info);
                                                                        on_add.run(path);
                                                                        query.set(String::new());
                                                                    }
                                                                    Err(e) => {
                                                                        error.set(Some(e.to_string()));
                                                                    }
                                                                }
                                                                saving.set(false);
                                                            });
                                                        }
                                                    >
                                                        {format!("create new \"{}\"", create_query.trim())}
                                                    </li>
                                                }
                                            })
                                    }}
                                </ul>
                            },
                        )
                    })
                    .flatten()
            }}
            {move || {
                error
                    .get()
                    .map(|msg| {
                        view! { <p class=style::error>{msg}</p> }
                    })
            }}
        </div>
    }
}
