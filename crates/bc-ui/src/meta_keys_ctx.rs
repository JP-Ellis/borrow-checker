//! App-level metadata key registry: one snapshot of every registered key and
//! its type, shared by every metadata editor instance.
//!
//! The registry is global and small, so it is fetched whole rather than by key.
//! It is re-fetched whenever the window regains focus, which is when a key
//! registered elsewhere — by the CLI, or by a plugin import — can first matter
//! to the user. A key created inside an editor is appended to the snapshot
//! locally, following `TagPicker`'s create-new row; the backend registers it on
//! the save that first writes a value under it, and the next re-fetch drops any
//! local append that never got that far.

#[cfg(target_arch = "wasm32")]
use bc_ipc::MetaKeyDefDto;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

#[cfg(target_arch = "wasm32")]
use crate::window_focus::focus_count;

/// Reactive handle to the metadata key registry, provided once at the shell root.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct MetaKeyStore(pub RwSignal<Vec<MetaKeyDefDto>>);

/// Provides an empty [`MetaKeyStore`] into context and kicks off a
/// `list_metadata_keys` load to populate it, repeated on every window focus.
/// Call once, at the shell root.
///
/// # Returns
///
/// The provided [`MetaKeyStore`] handle.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn provide_meta_key_store() -> MetaKeyStore {
    let signal = RwSignal::new(Vec::<MetaKeyDefDto>::new());
    let store = MetaKeyStore(signal);
    provide_context(store);
    let focused = focus_count();
    let _resource = LocalResource::new(move || {
        focused.track();
        async move {
            if let Ok(list) = bc_ipc::client::list_metadata_keys().await {
                signal.set(list);
            }
        }
    });
    store
}

/// Reads the shared registry snapshot from context, or an empty one when the
/// context is unavailable.
///
/// A key missing from the snapshot makes its editor row untyped rather than
/// broken, so an empty snapshot degrades to a read-only view of existing entries
/// rather than to a wrong one.
///
/// # Returns
///
/// A reactive [`RwSignal`] of the registered keys.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn use_meta_key_store() -> RwSignal<Vec<MetaKeyDefDto>> {
    use_context::<MetaKeyStore>().map_or_else(|| RwSignal::new(Vec::new()), |store| store.0)
}
