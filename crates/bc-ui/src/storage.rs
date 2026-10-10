//! Thin `localStorage` access for per-browser UI preferences, and
//! `sessionStorage` access for per-tab marks.
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

/// Reads `key` from `sessionStorage`, or `None` when absent or unavailable.
///
/// # Arguments
///
/// * `key` - Storage key.
#[must_use]
pub fn session_get(key: &str) -> Option<String> {
    web_sys::window()?
        .session_storage()
        .ok()
        .flatten()?
        .get_item(key)
        .ok()
        .flatten()
}

/// Writes `value` under `key` in `sessionStorage`; a failure is logged and
/// otherwise ignored.
///
/// # Arguments
///
/// * `key` - Storage key.
/// * `value` - Value to persist.
pub fn session_set(key: &str, value: &str) {
    if let Some(storage) = web_sys::window().and_then(|w| w.session_storage().ok().flatten())
        && let Err(e) = storage.set_item(key, value)
    {
        leptos::logging::warn!("sessionStorage set_item failed: {e:?}");
    }
}
