//! A counter of window focus events, for data another window can change.
//!
//! A second app window or browser tab writes to the same ledger. Its changes
//! first matter when the user returns, so a resource that tracks this counter
//! refetches on every return.

use leptos::ev;
use leptos::prelude::*;

/// Counts the window's `focus` events from this call on.
///
/// The listener is removed when the calling owner is cleaned up.
///
/// # Returns
///
/// A counter that starts at zero and wraps on overflow.
#[must_use]
pub fn focus_count() -> ReadSignal<u32> {
    let (count, set_count) = signal(0_u32);
    let handle = window_event_listener(ev::focus, move |_| {
        set_count.update(|v| *v = v.wrapping_add(1));
    });
    on_cleanup(move || handle.remove());
    count
}
