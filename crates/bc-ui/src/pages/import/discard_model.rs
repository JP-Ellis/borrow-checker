//! Leptos-free text and state for discarding a batch.

use bc_ipc::BcError;
use bc_ipc::DiscardCounts;
use bc_ipc::DiscardDependant;
use bc_ipc::DiscardPreview;
use jiff::tz::TimeZone;

use crate::pages::import::history_model::short_timestamp;
use crate::pages::import::model::UNKNOWN_REPLY;
use crate::pages::import::model::count_noun;

/// Whether the consequences describe a discard to come or one done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tense {
    /// Before confirm: "will be".
    Future,
    /// On the discarded row: "was" / "were".
    Past,
}

/// One consequence sentence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Consequence {
    /// Whether it describes the user's own work being lost (warn tone).
    pub warn: bool,
    /// The sentence.
    pub text: String,
}

/// One consequence line's count, nouns, qualifier, predicate and tone.
type Entry = (
    u64,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    bool,
);

/// The countable consequences in risk order.
fn entries(c: &DiscardCounts) -> [Entry; 12] {
    // (count, singular, plural, qualifier, predicate, warn)
    [
        (
            c.edited_postings,
            "posting",
            "postings",
            "you edited",
            "lost",
            true,
        ),
        (
            c.reconciled_postings,
            "posting",
            "postings",
            "in reconciled transactions",
            "lost",
            true,
        ),
        (
            c.flagged_postings,
            "posting",
            "postings",
            "in flagged transactions",
            "lost",
            true,
        ),
        (
            c.other_batch_references_removed,
            "reference",
            "references",
            "from other batches",
            "removed with their transactions",
            false,
        ),
        (
            c.other_batch_references_tombstoned,
            "reference",
            "references",
            "from other batches",
            "left as tombstones",
            false,
        ),
        (
            c.detached_adopted,
            "adopted posting",
            "adopted postings",
            "",
            "kept but detached from this import",
            false,
        ),
        (
            c.freed_tombstones,
            "tombstone slot",
            "tombstone slots",
            "",
            "freed",
            false,
        ),
        (
            c.removed_tags,
            "tag",
            "tags",
            "created by this import",
            "removed",
            false,
        ),
        (
            c.kept_tags,
            "tag",
            "tags",
            "created by this import",
            "kept, since applied elsewhere",
            false,
        ),
        (
            c.removed_accounts,
            "account",
            "accounts",
            "created by this import",
            "removed",
            false,
        ),
        (
            c.kept_accounts,
            "account",
            "accounts",
            "created by this import",
            "kept, since named elsewhere",
            false,
        ),
        (
            c.reverted_fields,
            "account field",
            "account fields",
            "filled by this import",
            "cleared",
            false,
        ),
    ]
}

/// A discard's consequences in risk order — the user's lost edits, other
/// batches, kept-but-detached postings, housekeeping — zero counts hidden,
/// then the removal every discard makes.
///
/// # Arguments
///
/// * `c` - The counts.
/// * `tense` - Before or after the discard.
///
/// # Returns
///
/// The sentences, in display order.
#[must_use]
pub(crate) fn consequences(c: &DiscardCounts, tense: Tense) -> Vec<Consequence> {
    let entries = entries(c);
    let mut lines: Vec<Consequence> = entries
        .into_iter()
        .filter(|&(n, ..)| n > 0)
        .map(|(n, one, many, qualifier, predicate, warn)| {
            let subject = if qualifier.is_empty() {
                count_noun(&n, one, many)
            } else {
                format!("{} {qualifier}", count_noun(&n, one, many))
            };
            let verb = match tense {
                Tense::Future => "will be",
                Tense::Past if n == 1 => "was",
                Tense::Past => "were",
            };
            Consequence {
                warn,
                text: format!("{subject} {verb} {predicate}."),
            }
        })
        .collect();
    let lead = match tense {
        Tense::Future => "Removes",
        Tense::Past => "Removed",
    };
    lines.push(Consequence {
        warn: false,
        text: format!(
            "{lead} {} across {}.",
            count_noun(&c.removed_postings, "posting", "postings"),
            count_noun(&c.removed_transactions, "transaction", "transactions")
        ),
    });
    lines
}

/// The confirm button's label.
///
/// # Arguments
///
/// * `c` - The previewed counts.
///
/// # Returns
///
/// Such as `"Discard: remove 42 transactions"`.
#[must_use]
pub(crate) fn confirm_label(c: &DiscardCounts) -> String {
    format!(
        "Discard: remove {}",
        count_noun(&c.removed_transactions, "transaction", "transactions")
    )
}

/// The line under the consequences saying how a discard can be undone.
///
/// # Arguments
///
/// * `snapshot_planned` - Whether `backup.auto-pre-discard` will snapshot first.
///
/// # Returns
///
/// The sentence, and whether it takes the warn tone.
#[must_use]
pub(crate) fn safety_line(snapshot_planned: bool) -> (&'static str, bool) {
    if snapshot_planned {
        (
            "A snapshot is taken first. Discard has no undo; restoring the snapshot is the way back.",
            false,
        )
    } else {
        (
            "No snapshot will be taken (`backup.auto-pre-discard` is off). This cannot be reversed.",
            true,
        )
    }
}

/// An armed discard panel's state.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DiscardState {
    /// `preview_discard` is in flight.
    Loading,
    /// The discard can run; these are its rolled-back consequences.
    Ready {
        /// What it would remove.
        counts: DiscardCounts,
        /// Whether a snapshot is taken first.
        snapshot_planned: bool,
    },
    /// Later batches must be discarded first.
    Blocked(Vec<DiscardDependant>),
    /// A call failed.
    Error(String),
}

impl DiscardState {
    /// The previewed counts, when the discard can run.
    #[must_use]
    pub(crate) fn ready_counts(&self) -> Option<&DiscardCounts> {
        match self {
            Self::Ready { counts, .. } => Some(counts),
            Self::Loading | Self::Blocked(_) | Self::Error(_) => None,
        }
    }
}

/// The state a `preview_discard` reply leads to.
///
/// # Arguments
///
/// * `result` - The reply.
///
/// # Returns
///
/// `Ready`, `Blocked` or `Error`.
#[must_use]
pub(crate) fn after_preview_discard(result: Result<DiscardPreview, BcError>) -> DiscardState {
    match result {
        Ok(DiscardPreview::Ready {
            counts,
            snapshot_planned,
        }) => DiscardState::Ready {
            counts,
            snapshot_planned,
        },
        Ok(DiscardPreview::Blocked { dependants }) => DiscardState::Blocked(dependants),
        Ok(_) => DiscardState::Error(UNKNOWN_REPLY.to_owned()),
        Err(e) => DiscardState::Error(e.to_string()),
    }
}

/// What a failed `discard_batch` means.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DiscardFailure {
    /// A later batch landed since the preview; show the Blocked panel.
    Blocked,
    /// The batch is already discarded; refetch History with a notice.
    AlreadyDiscarded,
    /// Anything else, as its message.
    Other(String),
}

/// Classifies a failed `discard_batch`.
///
/// # Arguments
///
/// * `err` - The error.
/// * `now_discarded` - Whether a History refetch shows the batch discarded.
///
/// # Returns
///
/// The failure's meaning; a batch now discarded wins over the error's kind.
#[must_use]
pub(crate) fn classify(err: &BcError, now_discarded: bool) -> DiscardFailure {
    if now_discarded {
        DiscardFailure::AlreadyDiscarded
    } else if matches!(err, BcError::Conflict(_)) {
        DiscardFailure::Blocked
    } else {
        DiscardFailure::Other(err.to_string())
    }
}

/// Whether `batch_id`'s discard call is in flight, which disables its Confirm
/// and Cancel.
///
/// # Arguments
///
/// * `discarding` - The batch whose `discard_batch` call is in flight, if any.
/// * `batch_id` - The panel's batch.
///
/// # Returns
///
/// `true` when `discarding` names `batch_id`.
#[must_use]
pub(crate) fn is_discarding(discarding: Option<&str>, batch_id: &str) -> bool {
    discarding == Some(batch_id)
}

/// A dependant batch as its Blocked-panel link text.
///
/// # Arguments
///
/// * `d` - The dependant.
/// * `tz` - The display zone.
///
/// # Returns
///
/// Such as `"csv batch from 2026-10-11 09:30: 3 postings on 2 transactions"`.
#[must_use]
pub(crate) fn dependant_text(d: &DiscardDependant, tz: &TimeZone) -> String {
    format!(
        "{} batch from {}: {} on {}",
        d.importer,
        short_timestamp(&d.started_at, tz),
        count_noun(&d.postings, "posting", "postings"),
        count_noun(&d.transactions, "transaction", "transactions")
    )
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::BcError;
    use bc_ipc::DiscardPreview;
    use jiff::tz::TimeZone;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::json;

    use super::*;
    use crate::pages::import::fixtures;

    fn texts(c: &[Consequence]) -> Vec<(bool, &str)> {
        c.iter().map(|l| (l.warn, l.text.as_str())).collect()
    }

    #[test]
    fn consequences_run_in_risk_order_with_zeros_hidden() {
        let c = fixtures::counts(json!({
            "removed_postings": 120_u64, "removed_transactions": 42_u64, "edited_postings": 2_u64,
            "flagged_postings": 1_u64, "other_batch_references_tombstoned": 3_u64,
            "detached_adopted": 1_u64, "removed_tags": 2_u64, "reverted_fields": 1_u64
        }));
        assert_eq!(
            texts(&consequences(&c, Tense::Future)),
            vec![
                (true, "2 postings you edited will be lost."),
                (true, "1 posting in flagged transactions will be lost."),
                (
                    false,
                    "3 references from other batches will be left as tombstones."
                ),
                (
                    false,
                    "1 adopted posting will be kept but detached from this import."
                ),
                (false, "2 tags created by this import will be removed."),
                (
                    false,
                    "1 account field filled by this import will be cleared."
                ),
                (false, "Removes 120 postings across 42 transactions."),
            ]
        );
    }

    #[test]
    fn the_past_tense_agrees_in_number() {
        let c = fixtures::counts(json!({
            "removed_postings": 1_u64, "removed_transactions": 1_u64, "reconciled_postings": 1_u64, "kept_accounts": 2_u64
        }));
        assert_eq!(
            texts(&consequences(&c, Tense::Past)),
            vec![
                (true, "1 posting in reconciled transactions was lost."),
                (
                    false,
                    "2 accounts created by this import were kept, since named elsewhere."
                ),
                (false, "Removed 1 posting across 1 transaction."),
            ]
        );
    }

    #[rstest]
    #[case(42, "Discard: remove 42 transactions")]
    #[case(1, "Discard: remove 1 transaction")]
    fn confirm_label_cases(#[case] n: u64, #[case] expected: &str) {
        assert_eq!(
            confirm_label(&fixtures::counts(json!({ "removed_transactions": n }))),
            expected
        );
    }

    #[rstest]
    #[case(
        true,
        false,
        "A snapshot is taken first. Discard has no undo; restoring the snapshot is the way back."
    )]
    #[case(
        false,
        true,
        "No snapshot will be taken (`backup.auto-pre-discard` is off). This cannot be reversed."
    )]
    fn safety_line_cases(#[case] planned: bool, #[case] warn: bool, #[case] text: &str) {
        assert_eq!(safety_line(planned), (text, warn));
    }

    #[test]
    fn a_ready_preview_carries_its_counts() {
        let state = after_preview_discard(Ok(fixtures::discard_ready(
            json!({ "removed_postings": 4_u64 }),
            true,
        )));
        assert_eq!(
            state,
            DiscardState::Ready {
                counts: fixtures::counts(json!({ "removed_postings": 4_u64 })),
                snapshot_planned: true,
            }
        );
        assert_eq!(
            state.ready_counts(),
            Some(&fixtures::counts(json!({ "removed_postings": 4_u64 })))
        );
    }

    #[test]
    fn a_blocked_preview_lists_its_dependants() {
        let DiscardPreview::Blocked { dependants } = fixtures::discard_blocked() else {
            panic!("fixture is Blocked");
        };
        let state = after_preview_discard(Ok(fixtures::discard_blocked()));
        assert_eq!(state, DiscardState::Blocked(dependants.clone()));
        assert_eq!(state.ready_counts(), None);
        let first = dependants.first().expect("one dependant");
        assert_eq!(
            dependant_text(first, &TimeZone::UTC),
            "csv batch from 2026-10-11 09:30: 3 postings on 2 transactions"
        );
    }

    #[rstest]
    #[case(Some("batch-0001"), "batch-0001", true)]
    #[case(Some("batch-0002"), "batch-0001", false)]
    #[case(None, "batch-0001", false)]
    fn is_discarding_cases(
        #[case] discarding: Option<&str>,
        #[case] batch_id: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(is_discarding(discarding, batch_id), expected);
    }

    #[rstest]
    #[case(DiscardState::Loading)]
    #[case(DiscardState::Error("not found: batch-0009".to_owned()))]
    fn only_ready_has_counts(#[case] state: DiscardState) {
        assert_eq!(state.ready_counts(), None);
    }

    #[test]
    fn a_failed_preview_is_an_error() {
        let state = after_preview_discard(Err(BcError::NotFound("batch-0009".to_owned())));
        assert_eq!(
            state,
            DiscardState::Error("not found: batch-0009".to_owned())
        );
    }

    #[rstest]
    #[case(BcError::Conflict("later batches own legs".to_owned()), false, DiscardFailure::Blocked)]
    #[case(BcError::Validation("already been discarded".to_owned()), true, DiscardFailure::AlreadyDiscarded)]
    #[case(BcError::Conflict("later batches own legs".to_owned()), true, DiscardFailure::AlreadyDiscarded)]
    #[case(BcError::Internal("disk full".to_owned()), false, DiscardFailure::Other("internal error: disk full".to_owned()))]
    fn classify_cases(
        #[case] err: BcError,
        #[case] now_discarded: bool,
        #[case] expected: DiscardFailure,
    ) {
        assert_eq!(classify(&err, now_discarded), expected);
    }
}
