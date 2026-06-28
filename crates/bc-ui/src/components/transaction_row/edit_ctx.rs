//! Shared edit context for the editable transaction detail view.

use bc_ipc::AccountRef;
use bc_ipc::TagInfo;
use leptos::prelude::*;

use crate::components::transaction_row::editable::EditableTransaction;

/// Context shared across the editable detail view via Leptos context.
///
/// The detail view is always editable: edits flow into `working`, which is
/// diffed against the pristine `original` snapshot to drive the dirty-gated save
/// bar. Discarding restores `working` from `original`.
#[derive(Clone)]
pub struct TxEditCtx {
    /// The dirty working buffer.
    pub working: RwSignal<EditableTransaction>,
    /// The pristine buffer to diff against / restore on discard.
    pub original: StoredValue<EditableTransaction>,
    /// All selectable accounts for the per-row picker.
    pub accounts: StoredValue<Vec<AccountRef>>,
    /// All known tags, populated asynchronously after context creation.
    pub all_tags: RwSignal<Vec<TagInfo>>,
    /// Monotonic counter bumped whenever `working` is reset externally (discard).
    ///
    /// Per-posting inputs that mirror `working` into local signals watch this to
    /// re-seed themselves after a reset without coupling to every keystroke.
    pub reset_epoch: RwSignal<u32>,
}

impl TxEditCtx {
    /// Creates a context seeded from `original`.
    ///
    /// `all_tags` starts empty and is populated asynchronously by the owning
    /// component after creation.
    ///
    /// # Arguments
    ///
    /// * `original` - The pristine working buffer.
    /// * `accounts` - All selectable accounts.
    ///
    /// # Returns
    ///
    /// The new context.
    #[must_use]
    pub fn new(original: EditableTransaction, accounts: Vec<AccountRef>) -> Self {
        Self {
            working: RwSignal::new(original.clone()),
            original: StoredValue::new(original),
            accounts: StoredValue::new(accounts),
            all_tags: RwSignal::new(Vec::new()),
            reset_epoch: RwSignal::new(0),
        }
    }

    /// Returns whether the working buffer differs from the original.
    ///
    /// # Returns
    ///
    /// `true` when the working buffer is not value-equal to the original.
    #[must_use]
    pub fn dirty(&self) -> bool {
        self.working.with(|w| self.original.with_value(|o| w != o))
    }

    /// Restores the working buffer from the pristine original snapshot.
    ///
    /// Bumps `reset_epoch` so per-posting inputs mirroring `working` re-seed
    /// themselves from the restored buffer.
    pub fn discard(&self) {
        self.working.set(self.original.get_value());
        self.reset_epoch.update(|e| *e = e.wrapping_add(1));
    }
}
