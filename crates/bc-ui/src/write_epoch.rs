//! App-wide count of writes made outside the page on screen.
//!
//! The shell's new-transaction overlay saves above every page, so it cannot
//! reach a page's own refetch counter. It bumps this epoch instead, and each
//! page forwards a change into its counter.

use leptos::prelude::*;

/// The write epoch; provided once by the shell.
#[derive(Clone, Copy)]
pub struct WriteEpoch(RwSignal<u32>);

impl WriteEpoch {
    /// Records a write, so every page that forwards the epoch refetches.
    pub fn bump(self) {
        self.0.update(|v| *v = v.wrapping_add(1));
    }

    /// Bumps `data_version` whenever the epoch changes after this call.
    ///
    /// # Arguments
    ///
    /// * `data_version` - The page's refetch counter.
    pub fn forward_to(self, data_version: RwSignal<u32>) {
        let epoch = self.0;
        Effect::new(move |seen: Option<u32>| {
            let now = epoch.get();
            if seen.is_some_and(|seen| seen != now) {
                data_version.update(|v| *v = v.wrapping_add(1));
            }
            now
        });
    }
}

/// Provides a fresh [`WriteEpoch`] into context. Call once, at the shell root.
pub fn provide_write_epoch() {
    provide_context(WriteEpoch(RwSignal::new(0)));
}

/// Reads the [`WriteEpoch`] from context.
///
/// # Returns
///
/// The shell's epoch, or a detached one outside the shell that nothing else
/// watches.
#[must_use]
pub fn use_write_epoch() -> WriteEpoch {
    use_context::<WriteEpoch>().unwrap_or_else(|| WriteEpoch(RwSignal::new(0)))
}
