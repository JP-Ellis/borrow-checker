//! How an open editor answers a new server copy of its transaction.
//!
//! The IPC transaction has no version, so the editor recognises its own
//! echoes by content: every copy it has based itself on stays known while
//! it is open.

use bc_ipc::Transaction;

/// What the editor does with a server copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerCopy {
    /// The copy belongs to another transaction, is one the editor already
    /// holds, or arrived while a save owns the buffer.
    Ignore,
    /// A foreign change under a clean draft: replace the buffer with it.
    Adopt,
    /// A foreign change under a dirty draft: keep the draft, flag the base.
    MarkStale,
}

/// How a server copy relates to the bases an editor has held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CopyOrigin {
    /// The copy has another transaction's id.
    OtherTransaction,
    /// The copy equals a base the editor has held.
    Known,
    /// The copy is of this transaction, but differs from every held base.
    Foreign,
}

/// Decides how the editor answers a server copy.
///
/// # Arguments
///
/// * `saving` - A save owns `working` and `original` until its refetch settles.
/// * `origin` - How the copy relates to the editor's transaction and bases.
/// * `dirty` - The draft differs from its base.
#[must_use]
#[expect(
    clippy::module_name_repetitions,
    reason = "imported unqualified into the editor, where `on_server_copy` names the event"
)]
pub const fn on_server_copy(saving: bool, origin: CopyOrigin, dirty: bool) -> ServerCopy {
    match (saving, origin, dirty) {
        (true, _, _) | (false, CopyOrigin::OtherTransaction | CopyOrigin::Known, _) => {
            ServerCopy::Ignore
        }
        (false, CopyOrigin::Foreign, false) => ServerCopy::Adopt,
        (false, CopyOrigin::Foreign, true) => ServerCopy::MarkStale,
    }
}

/// Every server copy an editor has based itself on, oldest first.
///
/// An older base stays known: a register response that predates a save's
/// refetch carries it, and must not read as a foreign change.
#[derive(Clone, Debug)]
pub struct KnownBases {
    /// The id of the editor's transaction, taken from the first base.
    id: String,
    /// The recorded bases, oldest first.
    bases: Vec<Transaction>,
}

impl KnownBases {
    /// Starts with the copy the editor opened on; its id is the editor's.
    #[must_use]
    pub fn new(first: Transaction) -> Self {
        Self {
            id: first.id.clone(),
            bases: vec![first],
        }
    }

    /// Records `tx` as a base; a copy already known is not stored again.
    pub fn record(&mut self, tx: Transaction) {
        if !self.contains(&tx) {
            self.bases.push(tx);
        }
    }

    /// Whether `tx` equals any recorded base.
    #[must_use]
    pub fn contains(&self, tx: &Transaction) -> bool {
        self.bases.iter().any(|base| base == tx)
    }

    /// Classifies `copy` against the editor's id and recorded bases.
    #[must_use]
    pub fn origin(&self, copy: &Transaction) -> CopyOrigin {
        if copy.id != self.id {
            CopyOrigin::OtherTransaction
        } else if self.contains(copy) {
            CopyOrigin::Known
        } else {
            CopyOrigin::Foreign
        }
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

    fn tx_with_id(id: &str, description: &str) -> Transaction {
        Transaction::new(
            id,
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

    fn tx(description: &str) -> Transaction {
        tx_with_id("tx-1", description)
    }

    #[rstest]
    #[case::saving_known_clean(true, CopyOrigin::Known, false, ServerCopy::Ignore)]
    #[case::saving_foreign_dirty(true, CopyOrigin::Foreign, true, ServerCopy::Ignore)]
    #[case::saving_foreign_clean(true, CopyOrigin::Foreign, false, ServerCopy::Ignore)]
    #[case::saving_other_clean(true, CopyOrigin::OtherTransaction, false, ServerCopy::Ignore)]
    #[case::other_clean(false, CopyOrigin::OtherTransaction, false, ServerCopy::Ignore)]
    #[case::other_dirty(false, CopyOrigin::OtherTransaction, true, ServerCopy::Ignore)]
    #[case::known_clean(false, CopyOrigin::Known, false, ServerCopy::Ignore)]
    #[case::known_dirty(false, CopyOrigin::Known, true, ServerCopy::Ignore)]
    #[case::foreign_clean(false, CopyOrigin::Foreign, false, ServerCopy::Adopt)]
    #[case::foreign_dirty(false, CopyOrigin::Foreign, true, ServerCopy::MarkStale)]
    fn decision_table(
        #[case] saving: bool,
        #[case] origin: CopyOrigin,
        #[case] dirty: bool,
        #[case] expected: ServerCopy,
    ) {
        assert_eq!(on_server_copy(saving, origin, dirty), expected);
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
        assert_eq!(bases.bases.len(), 2);
    }

    #[rstest]
    #[case::opened_copy(tx("opened"), CopyOrigin::Known)]
    #[case::recorded_copy(tx("saved"), CopyOrigin::Known)]
    #[case::changed_copy(tx("someone else's"), CopyOrigin::Foreign)]
    #[case::other_id(tx_with_id("tx-2", "opened"), CopyOrigin::OtherTransaction)]
    fn origin_checks_the_id_before_the_bases(
        #[case] copy: Transaction,
        #[case] expected: CopyOrigin,
    ) {
        let mut bases = KnownBases::new(tx("opened"));
        bases.record(tx("saved"));
        assert_eq!(bases.origin(&copy), expected);
    }
}
