//! QA showcase for [`TagPicker`](super::TagPicker).

use bc_ipc::TagInfo;
use leptos::prelude::*;

use super::TagPicker;

/// Renders the picker against a fixed tag list with local selection state.
#[component]
pub fn TagPickerQa() -> impl IntoView {
    let all_tags: RwSignal<Vec<TagInfo>> = RwSignal::new(vec![
        TagInfo::new("t1", "person:alice"),
        TagInfo::new("t2", "person:bob"),
        TagInfo::new("t3", "category:food"),
        TagInfo::new("t4", "category:transport"),
        TagInfo::new("t5", "project:borrow-checker"),
    ]);
    let selected: RwSignal<Vec<String>> = RwSignal::new(vec![]);

    view! {
        <div style="max-width: 32rem; padding: 1rem;">
            <TagPicker
                tags=selected.read_only().into()
                all_tags=all_tags.read_only().into()
                on_add=Callback::new(move |path: String| {
                    selected
                        .update(|v| {
                            if !v.contains(&path) {
                                v.push(path);
                            }
                        });
                })
                on_remove=Callback::new(move |path: String| {
                    selected.update(|v| v.retain(|p| p != &path));
                })
                on_created=Callback::new(move |info: TagInfo| {
                    all_tags.update(|v| v.push(info));
                })
            />
            <p style="margin-top: 1rem; font-family: var(--bc-font-mono); font-size: var(--bc-text-caption);">
                "selected: " {move || selected.get().join(", ")}
            </p>
        </div>
    }
}
