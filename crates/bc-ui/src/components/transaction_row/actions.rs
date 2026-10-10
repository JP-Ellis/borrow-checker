// Pure text and decisions for the register detail's delete and reverse gates.
// No Leptos here; this is host-tested via the `include!` shim in `main.rs`.

use bc_ipc::AuditEntry;
use bc_ipc::DeleteOutcome;
use bc_ipc::TransactionProvenance;

/// Which inline confirmation the register detail shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(
    not(target_arch = "wasm32"),
    expect(dead_code, reason = "only the wasm detail view switches on the gate")
)]
pub(crate) enum Gate {
    /// No gate is open; the Delete and Reverse buttons show.
    #[default]
    Closed,
    /// The delete confirmation is open.
    Delete,
    /// The reverse confirmation is open.
    Reverse,
}

/// Headline of the delete gate.
///
/// # Arguments
///
/// * `provenance` - The import history lookup: `None` while it is in flight,
///   `Err(())` when it failed.
///
/// # Returns
///
/// The sentence that opens the gate.
pub(crate) fn delete_headline(provenance: Option<&Result<TransactionProvenance, ()>>) -> String {
    match provenance {
        None => "Checking import history…".to_owned(),
        Some(Err(())) => {
            "Delete this transaction? Its import history could not be read.".to_owned()
        }
        Some(Ok(p)) if p.rows == 0 => "Delete this transaction?".to_owned(),
        Some(Ok(p)) => format!("Imported from {}.", p.accounts.join(", ")),
    }
}

/// Whether the delete gate offers both the skip and the allow re-import buttons.
///
/// A known hand entry gets one button. While the lookup is in flight this is
/// `false`, and that single button stays disabled until the answer arrives.
///
/// # Arguments
///
/// * `provenance` - The import history lookup, as for [`delete_headline`].
///
/// # Returns
///
/// `true` unless the transaction is known to carry no imported legs or the
/// lookup has not answered.
pub(crate) fn offers_both_deletes(provenance: Option<&Result<TransactionProvenance, ()>>) -> bool {
    match provenance {
        None => false,
        Some(Err(())) => true,
        Some(Ok(p)) => p.rows > 0,
    }
}

/// Warnings shown in the delete gate. None of them disables a button.
///
/// # Arguments
///
/// * `reconciled` - Whether the saved transaction is reconciled.
/// * `dirty` - Whether the detail holds unsaved edits.
///
/// # Returns
///
/// The warnings that apply, in display order.
pub(crate) fn delete_warnings(reconciled: bool, dirty: bool) -> Vec<&'static str> {
    let mut warnings = Vec::new();
    if reconciled {
        warnings.push("This transaction is reconciled; deleting it changes a reconciled balance.");
    }
    if dirty {
        warnings.push("Unsaved edits will be discarded.");
    }
    warnings
}

/// Headline of the reverse gate.
///
/// # Arguments
///
/// * `date` - The saved transaction's date, which the reversal takes.
///
/// # Returns
///
/// The sentence that opens the gate.
pub(crate) fn reverse_headline(date: jiff::civil::Date) -> String {
    format!("Add a reversing transaction dated {date}?")
}

/// Warnings shown in the reverse gate. None of them disables a button.
///
/// # Arguments
///
/// * `already_reversed` - Whether the audit trail records an earlier reversal.
/// * `dirty` - Whether the detail holds unsaved edits.
///
/// # Returns
///
/// The warnings that apply, in display order.
pub(crate) fn reverse_warnings(already_reversed: bool, dirty: bool) -> Vec<&'static str> {
    let mut warnings = Vec::new();
    if already_reversed {
        warnings.push("This transaction has already been reversed.");
    }
    if dirty {
        warnings
            .push("Unsaved edits are not part of the reversal; it negates the saved transaction.");
    }
    warnings
}

/// Whether the audit trail records a reversal of this transaction.
///
/// # Arguments
///
/// * `entries` - The transaction's audit trail.
///
/// # Returns
///
/// `true` when any entry has kind `"reverse"`.
pub(crate) fn already_reversed(entries: &[AuditEntry]) -> bool {
    entries.iter().any(|e| e.kind == "reverse")
}

/// Success toast after a delete, naming what a re-import will do.
///
/// # Arguments
///
/// * `outcome` - The delete's reference counts.
/// * `id` - The deleted transaction's ID, quoted in the release command.
///
/// # Returns
///
/// The toast text.
pub(crate) fn delete_toast(outcome: &DeleteOutcome, id: &str) -> String {
    if outcome.references_kept > 0 {
        format!(
            "Deleted. A re-import will skip it; `borrow-checker import rejected release {id}` undoes that."
        )
    } else if outcome.references_forgotten > 0 {
        "Deleted. A re-import will recreate it.".to_owned()
    } else {
        "Transaction deleted.".to_owned()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use pretty_assertions::assert_eq;
    use rstest::rstest;

    use super::*;

    fn provenance(rows: u64, accounts: &[&str]) -> TransactionProvenance {
        TransactionProvenance::new(rows, accounts.iter().map(|a| (*a).to_owned()).collect())
    }

    #[rstest]
    #[case::loading(None, "Checking import history…")]
    #[case::hand(Some(Ok(provenance(0, &[]))), "Delete this transaction?")]
    #[case::one(Some(Ok(provenance(1, &["Assets:Bank"]))), "Imported from Assets:Bank.")]
    #[case::two(
        Some(Ok(provenance(2, &["Assets:Bank", "Liabilities:Card"]))),
        "Imported from Assets:Bank, Liabilities:Card."
    )]
    #[case::failed(Some(Err(())), "Delete this transaction? Its import history could not be read.")]
    fn delete_headline_reads_the_provenance(
        #[case] p: Option<Result<TransactionProvenance, ()>>,
        #[case] expected: &str,
    ) {
        assert_eq!(delete_headline(p.as_ref()), expected);
    }

    #[rstest]
    #[case::loading(None, false)]
    #[case::hand(Some(Ok(provenance(0, &[]))), false)]
    #[case::imported(Some(Ok(provenance(2, &["Assets:Bank"]))), true)]
    #[case::failed(Some(Err(())), true)]
    fn both_deletes_are_offered_unless_known_hand_entry(
        #[case] p: Option<Result<TransactionProvenance, ()>>,
        #[case] both: bool,
    ) {
        assert_eq!(offers_both_deletes(p.as_ref()), both);
    }

    const RECONCILED: &str =
        "This transaction is reconciled; deleting it changes a reconciled balance.";
    const DISCARDED: &str = "Unsaved edits will be discarded.";
    const REVERSED: &str = "This transaction has already been reversed.";
    const NOT_IN_REVERSAL: &str =
        "Unsaved edits are not part of the reversal; it negates the saved transaction.";

    #[rstest]
    #[case::clean(false, false, &[])]
    #[case::reconciled(true, false, &[RECONCILED])]
    #[case::dirty(false, true, &[DISCARDED])]
    #[case::both(true, true, &[RECONCILED, DISCARDED])]
    fn delete_warnings_list_each_condition(
        #[case] reconciled: bool,
        #[case] dirty: bool,
        #[case] expected: &[&str],
    ) {
        assert_eq!(delete_warnings(reconciled, dirty), expected);
    }

    #[rstest]
    #[case::clean(false, false, &[])]
    #[case::reversed(true, false, &[REVERSED])]
    #[case::dirty(false, true, &[NOT_IN_REVERSAL])]
    #[case::both(true, true, &[REVERSED, NOT_IN_REVERSAL])]
    fn reverse_warnings_list_each_condition(
        #[case] reversed: bool,
        #[case] dirty: bool,
        #[case] expected: &[&str],
    ) {
        assert_eq!(reverse_warnings(reversed, dirty), expected);
    }

    #[test]
    fn reverse_headline_names_the_date() {
        assert_eq!(
            reverse_headline(jiff::civil::date(2026, 3, 14)),
            "Add a reversing transaction dated 2026-03-14?"
        );
    }

    #[test]
    fn already_reversed_reads_the_reverse_entry() {
        let entry = |kind: &str| AuditEntry::new(jiff::Timestamp::UNIX_EPOCH, kind, "m");
        assert!(already_reversed(&[entry("create"), entry("reverse")]));
        assert!(!already_reversed(&[entry("create")]));
    }

    #[rstest]
    #[case::kept(
        2,
        0,
        "Deleted. A re-import will skip it; `borrow-checker import rejected release t1` undoes that."
    )]
    #[case::kept_one(
        1,
        0,
        "Deleted. A re-import will skip it; `borrow-checker import rejected release t1` undoes that."
    )]
    #[case::forgotten(0, 2, "Deleted. A re-import will recreate it.")]
    #[case::hand(0, 0, "Transaction deleted.")]
    fn delete_toast_names_the_consequence(
        #[case] kept: u64,
        #[case] forgotten: u64,
        #[case] text: &str,
    ) {
        let outcome = DeleteOutcome::new(kept, forgotten);
        assert_eq!(delete_toast(&outcome, "t1"), text);
    }
}
