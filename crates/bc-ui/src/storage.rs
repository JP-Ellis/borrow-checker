//! Thin `localStorage` access for per-browser UI preferences.
//!
//! Every call swallows failure: a private window or a blocked origin leaves
//! the UI on its defaults rather than failing to render.

use leptos::web_sys;

/// Reads `key` from `localStorage`, or `None` when absent or unavailable.
///
/// # Arguments
///
/// * `key` - Storage key.
#[must_use]
pub fn get(key: &str) -> Option<String> {
    web_sys::window()?
        .local_storage()
        .ok()
        .flatten()?
        .get_item(key)
        .ok()
        .flatten()
}

/// Writes `value` under `key` in `localStorage`; a failure is logged and
/// otherwise ignored.
///
/// # Arguments
///
/// * `key` - Storage key.
/// * `value` - Value to persist.
pub fn set(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten())
        && let Err(e) = storage.set_item(key, value)
    {
        leptos::logging::warn!("localStorage set_item failed: {e:?}");
    }
}
