//! Write warnings handed from a save to the row that will show them.
//!
//! A new-transaction draft has no row to hold its warning band. The saving
//! host records the warnings here under the new id, and the existing-mode
//! detail that next mounts for that id takes them as its band.

use bc_ipc::WriteWarning;
#[cfg(target_arch = "wasm32")]
use leptos::prelude::*;

/// One transaction's warnings, waiting for that transaction's detail to mount.
///
/// Holds a single slot: a later [`PendingWarnings::set`] replaces whatever an
/// earlier save left untaken.
#[cfg(target_arch = "wasm32")]
#[derive(Clone, Copy)]
pub struct PendingWarnings(StoredValue<Option<(String, Vec<WriteWarning>)>>);

#[cfg(target_arch = "wasm32")]
impl PendingWarnings {
    /// Records `warnings` for transaction `id`, replacing any untaken entry.
    ///
    /// # Arguments
    ///
    /// * `id` - The transaction the warnings belong to.
    /// * `warnings` - The warnings its detail should show.
    pub fn set(self, id: String, warnings: Vec<WriteWarning>) {
        self.0.set_value(Some((id, warnings)));
    }

    /// Takes the warnings recorded for `id`.
    ///
    /// # Arguments
    ///
    /// * `id` - The transaction whose detail is mounting.
    ///
    /// # Returns
    ///
    /// The recorded warnings, which leave the slot, or an empty list when the
    /// slot holds another transaction's warnings or nothing.
    #[must_use]
    pub fn take(self, id: &str) -> Vec<WriteWarning> {
        self.0
            .try_update_value(|slot| take_matching(slot, id))
            .unwrap_or_default()
    }
}

/// Empties `slot` and returns its warnings when it belongs to `id`.
///
/// # Arguments
///
/// * `slot` - The pending entry, if any.
/// * `id` - The transaction asking for its warnings.
///
/// # Returns
///
/// The warnings for `id`, or an empty list with `slot` left untouched.
fn take_matching(slot: &mut Option<(String, Vec<WriteWarning>)>, id: &str) -> Vec<WriteWarning> {
    if slot.as_ref().is_some_and(|(held, _)| held == id) {
        slot.take().map_or_default(|(_, warnings)| warnings)
    } else {
        Vec::new()
    }
}

/// The toast text for a save that warned: a headline, then one line per warning.
///
/// # Arguments
///
/// * `warnings` - The save's warnings.
#[must_use]
pub fn saved_with_warnings_message(warnings: &[WriteWarning]) -> String {
    core::iter::once("Transaction saved with warnings")
        .chain(warnings.iter().map(|w| w.message.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Provides an empty [`PendingWarnings`] into context. Call once, at the app root.
#[cfg(target_arch = "wasm32")]
pub fn provide_pending_warnings() {
    provide_context(PendingWarnings(StoredValue::new(None)));
}

/// Reads the [`PendingWarnings`] from context.
///
/// # Returns
///
/// The app's slot, or a fresh empty one outside the app root.
#[cfg(target_arch = "wasm32")]
#[must_use]
pub fn use_pending_warnings() -> PendingWarnings {
    use_context::<PendingWarnings>().unwrap_or_else(|| PendingWarnings(StoredValue::new(None)))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::WriteWarning;
    use pretty_assertions::assert_eq;

    use super::saved_with_warnings_message;
    use super::take_matching;

    fn entry() -> (String, Vec<WriteWarning>) {
        (
            "tx-1".to_owned(),
            vec![WriteWarning::new(
                Some("acct-cash".to_owned()),
                "account is closed",
            )],
        )
    }

    #[test]
    fn matching_id_takes_and_empties_the_slot() {
        let mut slot = Some(entry());
        let taken = take_matching(&mut slot, "tx-1");
        assert_eq!(
            taken,
            vec![WriteWarning::new(
                Some("acct-cash".to_owned()),
                "account is closed"
            )]
        );
        assert_eq!(slot, None);
    }

    #[test]
    fn other_id_takes_nothing_and_keeps_the_slot() {
        let mut slot = Some(entry());
        assert_eq!(take_matching(&mut slot, "tx-2"), Vec::new());
        assert_eq!(slot, Some(entry()));
    }

    #[test]
    fn message_puts_each_warning_on_its_own_line() {
        let warnings = [
            WriteWarning::new(Some("acct-cash".to_owned()), "account is closed"),
            WriteWarning::new(None, "unbalanced by 12.50 AUD"),
        ];
        assert_eq!(
            saved_with_warnings_message(&warnings),
            "Transaction saved with warnings\naccount is closed\nunbalanced by 12.50 AUD"
        );
    }

    #[test]
    fn empty_slot_takes_nothing() {
        let mut slot = None;
        assert_eq!(take_matching(&mut slot, "tx-1"), Vec::new());
    }
}
