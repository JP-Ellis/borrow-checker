//! How an open editor answers a new server copy of its transaction.
//!
//! The IPC transaction has no version, so the editor recognises its own
//! echoes by content: every copy it has based itself on stays known while
//! it is open.

use bc_ipc::Transaction;

/// What the editor does with a server copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerCopy {
    /// The copy is one the editor already holds, or a save owns the buffer.
    Ignore,
    /// A foreign change under a clean draft: replace the buffer with it.
    Adopt,
    /// A foreign change under a dirty draft: keep the draft, flag the base.
    MarkStale,
}

/// Decides how the editor answers a server copy.
///
/// # Arguments
///
/// * `saving` - A save owns `working` and `original` until its refetch settles.
/// * `known` - The copy equals a base the editor has held.
/// * `dirty` - The draft differs from its base.
#[must_use]
#[expect(
    clippy::module_name_repetitions,
    reason = "imported unqualified into the editor, where `on_server_copy` names the event"
)]
pub const fn on_server_copy(saving: bool, known: bool, dirty: bool) -> ServerCopy {
    match (saving, known, dirty) {
        (true, _, _) | (false, true, _) => ServerCopy::Ignore,
        (false, false, false) => ServerCopy::Adopt,
        (false, false, true) => ServerCopy::MarkStale,
    }
}

/// Every server copy an editor has based itself on, oldest first.
///
/// An older base stays known: a register response that predates a save's
/// refetch carries it, and must not read as a foreign change.
#[derive(Clone, Debug)]
pub struct KnownBases(Vec<Transaction>);

impl KnownBases {
    /// Starts with the copy the editor opened on.
    #[must_use]
    pub fn new(first: Transaction) -> Self {
        Self(vec![first])
    }

    /// Records `tx` as a base; a copy already known is not stored again.
    pub fn record(&mut self, tx: Transaction) {
        if !self.contains(&tx) {
            self.0.push(tx);
        }
    }

    /// Whether `tx` equals any recorded base.
    #[must_use]
    pub fn contains(&self, tx: &Transaction) -> bool {
        self.0.iter().any(|base| base == tx)
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::Reconciliation;
    use jiff::civil::Date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn tx(description: &str) -> Transaction {
        Transaction::new(
            "tx-1",
            Date::constant(2026, 4, 30),
            description,
            vec![],
            Reconciliation::Unreconciled,
            vec![],
            vec![],
            vec![],
            true,
        )
    }

    #[rstest]
    #[case::saving_known_clean(true, true, false, ServerCopy::Ignore)]
    #[case::saving_foreign_dirty(true, false, true, ServerCopy::Ignore)]
    #[case::saving_foreign_clean(true, false, false, ServerCopy::Ignore)]
    #[case::known_clean(false, true, false, ServerCopy::Ignore)]
    #[case::known_dirty(false, true, true, ServerCopy::Ignore)]
    #[case::foreign_clean(false, false, false, ServerCopy::Adopt)]
    #[case::foreign_dirty(false, false, true, ServerCopy::MarkStale)]
    fn decision_table(
        #[case] saving: bool,
        #[case] known: bool,
        #[case] dirty: bool,
        #[case] expected: ServerCopy,
    ) {
        assert_eq!(on_server_copy(saving, known, dirty), expected);
    }

    #[test]
    fn an_older_base_stays_known_after_a_newer_one() {
        let mut bases = KnownBases::new(tx("opened"));
        bases.record(tx("saved"));
        assert!(bases.contains(&tx("opened")));
        assert!(bases.contains(&tx("saved")));
        assert!(!bases.contains(&tx("someone else's")));
    }

    #[test]
    fn a_repeat_is_stored_once() {
        let mut bases = KnownBases::new(tx("opened"));
        bases.record(tx("opened"));
        bases.record(tx("saved"));
        bases.record(tx("saved"));
        assert_eq!(bases.0.len(), 2);
    }
}
