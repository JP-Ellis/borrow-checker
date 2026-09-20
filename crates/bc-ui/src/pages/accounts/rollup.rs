//! Page-level "Include sub-accounts" toggle shared by the sidebar, dashboard
//! and register through Leptos context.

use leptos::prelude::*;

/// `localStorage` key holding `"1"` or `"0"`.
pub const STORAGE_KEY: &str = "bc.accounts.rollup";

/// Context handle for the toggle. Default is on: root accounts hold no
/// postings of their own, so an un-rolled root shows nothing.
#[derive(Clone, Copy)]
pub struct IncludeDescendants(pub RwSignal<bool>);

/// Creates the toggle signal seeded from storage and persists every change.
#[must_use]
pub fn create_include_descendants() -> IncludeDescendants {
    let initial = crate::storage::get(STORAGE_KEY).is_none_or(|v| v != "0");
    let signal = RwSignal::new(initial);
    Effect::new(move |_| {
        crate::storage::set(STORAGE_KEY, if signal.get() { "1" } else { "0" });
    });
    IncludeDescendants(signal)
}

/// Reads the toggle from context, or a detached default (on) outside the
/// accounts page (QA showcases).
#[must_use]
pub fn use_include_descendants() -> RwSignal<bool> {
    use_context::<IncludeDescendants>().map_or_else(|| RwSignal::new(true), |c| c.0)
}
