//! Import IPC types: profiles, the dry-run preview, the commit outcome, and
//! batch history with discard.
//!
//! Counts cross as `u64`, ids as their string form, timestamps as RFC 3339
//! and dates as `YYYY-MM-DD`. A tagged enum serialises as an object whose
//! `kind` names the variant.

use serde::Deserialize;
use serde::Serialize;

use crate::Amount;

// MARK: Profiles

/// Every import profile, and whether any importer can read a file.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportProfiles {
    /// Whether `import.documents-root` is set; without it no importer can
    /// read a file.
    pub documents_root_set: bool,
    /// Every profile, by name.
    pub profiles: Vec<ImportProfileInfo>,
}

/// One import profile, read-only.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportProfileInfo {
    /// The profile's unique name.
    pub name: String,
    /// The importer the profile names.
    pub importer: String,
    /// Whether that importer is loaded.
    pub installed: bool,
    /// The importer configuration, as pretty-printed JSON.
    pub config_text: String,
}

// MARK: Preview

/// What committing a profile would write, computed without writing.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportPreview {
    /// The profile previewed.
    pub profile: String,
    /// The parsed source's fingerprint, 16 hex digits; `commit_import`
    /// refuses a source that parses to a different one.
    pub fingerprint: String,
    /// Transactions the commit would create.
    pub new_transactions: u64,
    /// Legs it would book onto transactions an earlier run created.
    pub attached_postings: u64,
    /// Rows whose every resolved leg is already stored.
    pub already_imported: u64,
    /// Legs it would skip, whatever the cause.
    pub skipped_postings: u64,
    /// Account paths naming no account, each with the legs it costs.
    pub unresolved_accounts: Vec<UnresolvedItem>,
    /// Commodity codes naming no registered commodity, each with the legs it
    /// costs.
    pub unresolved_commodities: Vec<UnresolvedItem>,
    /// Skipped legs by cause, in the cause's declaration order.
    pub skips_by_cause: Vec<CauseCount>,
    /// Advisory warnings on legs the commit would still write: declaration
    /// conflicts and postings into archived accounts. A lower bound; the
    /// commit may raise more, such as a date outside an account's life or a
    /// commodity outside its list.
    pub warnings: Vec<String>,
    /// Account paths the source's declarations would create, sorted.
    pub would_create_accounts: Vec<String>,
    /// Tag paths the commit would create, sorted.
    pub would_create_tags: Vec<String>,
    /// Net movement per account, sorted by account path.
    pub account_totals: Vec<AccountTotal>,
    /// Every parsed row with its fate, in source order.
    pub rows: Vec<PreviewRow>,
    /// Diagnostics whose location matches no row.
    pub other_diagnostics: Vec<DiagnosticInfo>,
}

/// An account path or commodity code naming nothing, and the legs it costs.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct UnresolvedItem {
    /// The path or code as the plan renders it.
    pub name: String,
    /// Legs skipped because of it.
    pub postings: u64,
}

/// Legs charged to one skip cause.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CauseCount {
    /// The cause's lower-case label, e.g. `unresolved account`.
    pub cause: String,
    /// Legs charged to it.
    pub postings: u64,
}

/// One account's net movement.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AccountTotal {
    /// The account path.
    pub account: String,
    /// One amount per commodity in first-seen order; empty when the legs net
    /// to zero.
    pub amounts: Vec<Amount>,
}

/// One parsed row and what a commit would do with it.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PreviewRow {
    /// Where the source says the row came from.
    pub location: String,
    /// The row's date, `YYYY-MM-DD`.
    pub date: String,
    /// The row's description.
    pub description: String,
    /// What the commit would do with the row.
    pub fate: RowFateInfo,
    /// The row's legs, in document order.
    pub legs: Vec<PreviewLeg>,
    /// Diagnostics at the row's location.
    pub diagnostics: Vec<DiagnosticInfo>,
}

/// What a commit would do with one row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RowFateInfo {
    /// Create a transaction.
    Create,
    /// Book at least one new leg onto `owner`.
    Attach {
        /// The existing transaction's id.
        owner: String,
    },
    /// Nothing: every resolved leg is already stored on `owner`.
    AlreadyImported {
        /// The existing transaction's id.
        owner: String,
    },
    /// Nothing: the legs already belong to several transactions.
    Conflict {
        /// Those transactions' ids.
        owners: Vec<String>,
    },
    /// Nothing: no leg reached the write.
    Skipped {
        /// The skip cause's label.
        cause: String,
    },
}

/// One leg as the document states it.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PreviewLeg {
    /// The account path as stated.
    pub account: String,
    /// The amount, or `None` for the elided residual.
    pub amount: Option<Amount>,
    /// What a commit would do with the leg.
    pub fate: LegFateInfo,
}

/// What a commit would do with one leg.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LegFateInfo {
    /// Write it.
    New,
    /// Nothing: it is already stored.
    Stored,
    /// Skip it.
    Skipped {
        /// The skip cause's label.
        cause: String,
    },
}

/// One leg or row a run cannot persist, and why.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DiagnosticInfo {
    /// Where the source says it came from.
    pub location: String,
    /// The skip cause's label.
    pub cause: String,
    /// The offending path, code or conflict.
    pub detail: String,
}

// MARK: Run results

/// The step at which a profile's run stopped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum FailureStageInfo {
    /// The profile names an importer that is not loaded.
    UnknownImporter,
    /// The importer could not read or parse its files.
    Importer,
    /// The engine failed while planning or writing.
    Engine,
    /// The source parsed differently from the previewed one.
    SourceChanged,
}

/// Why a profile's run produced nothing, or stopped part-way.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportFailure {
    /// The step that failed.
    pub stage: FailureStageInfo,
    /// The underlying error's message.
    pub message: String,
    /// The batch a commit opened before it stopped, holding every row
    /// written so far; discarding it undoes them.
    pub batch_id: Option<String>,
}

/// What `preview_import` produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
#[cfg_attr(
    target_pointer_width = "64",
    expect(
        clippy::large_enum_variant,
        reason = "built once per call and serialised at once; boxing buys nothing"
    )
)]
pub enum PreviewResult {
    /// The plan.
    Ready(ImportPreview),
    /// Why there is no plan.
    Failed(ImportFailure),
}

/// What `commit_import` produced.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum CommitResult {
    /// The rows were written.
    Imported(ImportResult),
    /// Nothing was written: the source changed since the preview. Carries
    /// the fresh preview.
    Changed(ImportPreview),
    /// The run failed; a `batch_id` names rows already written.
    Failed(ImportFailure),
}

/// What a committed run wrote.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportResult {
    /// The batch recording the run.
    pub batch_id: String,
    /// Transactions created.
    pub new_transactions: u64,
    /// Legs booked onto transactions an earlier run created.
    pub attached_postings: u64,
    /// Legs skipped, whatever the cause.
    pub skipped_postings: u64,
    /// Skipped legs by cause, in the cause's declaration order.
    pub skips_by_cause: Vec<CauseCount>,
    /// Account paths that named no account, sorted.
    pub unresolved_accounts: Vec<String>,
    /// Commodity codes that named no registered commodity, sorted.
    pub unresolved_commodities: Vec<String>,
    /// Tag paths the run created, sorted.
    pub created_tags: Vec<String>,
    /// Account paths the run's declarations created, sorted.
    pub created_accounts: Vec<String>,
    /// Advisory warnings on legs that were written.
    pub warnings: Vec<String>,
    /// The pre-import snapshot's path, when the policy took one.
    pub snapshot: Option<String>,
}

// MARK: Batches

/// Where an import batch stands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum BatchState {
    /// The run finished.
    Complete,
    /// The run stopped before finishing; its rows stay until discarded.
    Incomplete,
    /// The batch was discarded.
    Discarded,
}

/// A finished run's tallies.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BatchCounts {
    /// Transactions created.
    pub new_transactions: u64,
    /// Legs booked onto transactions an earlier run created.
    pub attached_postings: u64,
    /// Legs skipped, whatever the cause.
    pub skipped_postings: u64,
}

/// One import batch in the history.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ImportBatchInfo {
    /// The batch's id.
    pub id: String,
    /// The profile's name, or `None` when no profile drove the run or it
    /// has since been deleted.
    pub profile: Option<String>,
    /// The importer used.
    pub importer: String,
    /// When the run started, RFC 3339.
    pub started_at: String,
    /// When the run finished, RFC 3339.
    pub finished_at: Option<String>,
    /// Where the batch stands.
    pub state: BatchState,
    /// The run's tallies; `None` when it never finished.
    pub counts: Option<BatchCounts>,
    /// The discard's outcome, on a discarded batch.
    pub discard: Option<DiscardInfo>,
}

/// What a discard removed, kept or detached.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DiscardCounts {
    /// Postings deleted because the run had created them.
    pub removed_postings: u64,
    /// Transactions deleted because the discard left them with no postings.
    pub removed_transactions: u64,
    /// References removed from postings the run adopted; those postings stay.
    pub detached_adopted: u64,
    /// Tombstoned references removed, freeing their slots.
    pub freed_tombstones: u64,
    /// Other batches' references that went with a deleted transaction.
    pub other_batch_references_removed: u64,
    /// Other batches' references left naming a deleted posting, as tombstones.
    pub other_batch_references_tombstoned: u64,
    /// Removed postings the user had edited.
    pub edited_postings: u64,
    /// Removed postings in a reconciled transaction.
    pub reconciled_postings: u64,
    /// Removed postings in a flagged transaction.
    pub flagged_postings: u64,
    /// Tags the run created that nothing else names, deleted.
    pub removed_tags: u64,
    /// Tags the run created that something else now names, kept.
    pub kept_tags: u64,
    /// Accounts the run created that nothing else names, deleted.
    pub removed_accounts: u64,
    /// Accounts the run created that something else now names, kept.
    pub kept_accounts: u64,
    /// Fields the run filled on existing accounts, cleared back to empty.
    pub reverted_fields: u64,
}

/// A discard as the history keeps it.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DiscardInfo {
    /// What the discard did.
    pub counts: DiscardCounts,
    /// When it happened, RFC 3339.
    pub discarded_at: String,
    /// The pre-discard snapshot's path, or `None` when none was taken.
    pub snapshot: Option<String>,
}

/// A later batch that owns legs on a batch's transactions, blocking its
/// discard.
#[derive(bon::Builder, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DiscardDependant {
    /// The later batch's id.
    pub batch_id: String,
    /// Its importer.
    pub importer: String,
    /// When it started, RFC 3339.
    pub started_at: String,
    /// Live postings it owns on the blocked batch's transactions.
    pub postings: u64,
    /// Distinct such transactions.
    pub transactions: u64,
}

/// What discarding a batch would do, computed in a rolled-back transaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DiscardPreview {
    /// The discard can run.
    Ready {
        /// What it would do.
        counts: DiscardCounts,
        /// Whether `backup.auto-pre-discard` will snapshot first.
        snapshot_planned: bool,
    },
    /// Later batches built on this one; discard them first, newest first.
    Blocked {
        /// Those batches, newest first.
        dependants: Vec<DiscardDependant>,
    },
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use core::fmt::Debug;

    use pretty_assertions::assert_eq;
    use rust_decimal_macros::dec;
    use serde::Serialize;
    use serde::de::DeserializeOwned;

    use super::*;

    /// Serialises `dto`, checks it decodes back unchanged, and returns the JSON.
    fn round_trip<T>(dto: &T) -> serde_json::Value
    where
        T: Serialize + DeserializeOwned + PartialEq + Debug,
    {
        let json = serde_json::to_value(dto).expect("ser");
        let back: T = serde_json::from_value(json.clone()).expect("de");
        assert_eq!(&back, dto);
        json
    }

    /// `value` in AUD.
    fn aud(value: rust_decimal::Decimal) -> Amount {
        Amount::new(value, "AUD")
    }

    /// A leg on `account` for `value` AUD.
    fn leg(account: &str, value: rust_decimal::Decimal, fate: LegFateInfo) -> PreviewLeg {
        PreviewLeg {
            account: account.to_owned(),
            amount: Some(aud(value)),
            fate,
        }
    }

    /// A Supermarket row at `location` with one leg.
    fn row(location: &str, fate: RowFateInfo, leg: PreviewLeg) -> PreviewRow {
        PreviewRow {
            location: location.to_owned(),
            date: "2026-01-03".to_owned(),
            description: "Supermarket".to_owned(),
            fate,
            legs: vec![leg],
            diagnostics: Vec::new(),
        }
    }

    /// A preview naming every row and leg fate once.
    fn preview() -> ImportPreview {
        let unresolved = DiagnosticInfo {
            location: "statement.csv row 2".to_owned(),
            cause: "unresolved account".to_owned(),
            detail: "Expenses:Dining".to_owned(),
        };
        let mut created = row(
            "statement.csv row 2",
            RowFateInfo::Create,
            leg("Checking", dec!(-42.00), LegFateInfo::New),
        );
        created.legs.push(PreviewLeg {
            account: "Expenses:Dining".to_owned(),
            amount: None,
            fate: LegFateInfo::Skipped {
                cause: "unresolved account".to_owned(),
            },
        });
        created.diagnostics.push(unresolved);
        ImportPreview {
            profile: "groceries".to_owned(),
            fingerprint: "00000000000000ff".to_owned(),
            new_transactions: 1,
            attached_postings: 1,
            already_imported: 1,
            skipped_postings: 2,
            unresolved_accounts: vec![UnresolvedItem {
                name: "Expenses:Dining".to_owned(),
                postings: 1,
            }],
            unresolved_commodities: vec![UnresolvedItem {
                name: "XYZ".to_owned(),
                postings: 1,
            }],
            skips_by_cause: vec![CauseCount {
                cause: "unresolved account".to_owned(),
                postings: 1,
            }],
            warnings: vec!["Checking is archived".to_owned()],
            would_create_accounts: vec!["Expenses:Snacks".to_owned()],
            would_create_tags: vec!["shop".to_owned()],
            account_totals: vec![
                AccountTotal {
                    account: "Checking".to_owned(),
                    amounts: vec![aud(dec!(-42.00))],
                },
                AccountTotal {
                    account: "Savings".to_owned(),
                    amounts: Vec::new(),
                },
            ],
            rows: vec![
                created,
                row(
                    "statement.csv row 3",
                    RowFateInfo::Attach {
                        owner: "transaction_1".to_owned(),
                    },
                    leg("Groceries", dec!(5.00), LegFateInfo::New),
                ),
                row(
                    "statement.csv row 4",
                    RowFateInfo::AlreadyImported {
                        owner: "transaction_2".to_owned(),
                    },
                    leg("Checking", dec!(-5.00), LegFateInfo::Stored),
                ),
                row(
                    "statement.csv row 5",
                    RowFateInfo::Conflict {
                        owners: vec!["transaction_3".to_owned(), "transaction_4".to_owned()],
                    },
                    leg(
                        "Checking",
                        dec!(-7.00),
                        LegFateInfo::Skipped {
                            cause: "multi-owner conflict".to_owned(),
                        },
                    ),
                ),
                row(
                    "statement.csv row 6",
                    RowFateInfo::Skipped {
                        cause: "unregistered commodity".to_owned(),
                    },
                    leg(
                        "Checking",
                        dec!(-1.00),
                        LegFateInfo::Skipped {
                            cause: "unregistered commodity".to_owned(),
                        },
                    ),
                ),
            ],
            other_diagnostics: vec![DiagnosticInfo {
                location: "statement.csv".to_owned(),
                cause: "ignored declaration".to_owned(),
                detail: "duplicate open for Checking".to_owned(),
            }],
        }
    }

    /// Every discard count distinct, so a swapped field shows.
    fn discard_counts() -> DiscardCounts {
        DiscardCounts {
            removed_postings: 120,
            removed_transactions: 42,
            detached_adopted: 1,
            freed_tombstones: 2,
            other_batch_references_removed: 3,
            other_batch_references_tombstoned: 4,
            edited_postings: 5,
            reconciled_postings: 6,
            flagged_postings: 7,
            removed_tags: 8,
            kept_tags: 9,
            removed_accounts: 10,
            kept_accounts: 11,
            reverted_fields: 12,
        }
    }

    #[test]
    fn import_profiles_wire_shape() {
        let dto = ImportProfiles {
            documents_root_set: true,
            profiles: vec![ImportProfileInfo {
                name: "groceries".to_owned(),
                importer: "csv".to_owned(),
                installed: false,
                config_text: "{\n  \"account\": \"123456789\"\n}".to_owned(),
            }],
        };
        insta::assert_json_snapshot!(round_trip(&dto));
    }

    #[test]
    fn preview_ready_wire_shape() {
        insta::assert_json_snapshot!(round_trip(&PreviewResult::Ready(preview())));
    }

    #[test]
    fn preview_failed_wire_shape() {
        let dto = PreviewResult::Failed(ImportFailure {
            stage: FailureStageInfo::Importer,
            message: "parse error: unreadable statement".to_owned(),
            batch_id: None,
        });
        insta::assert_json_snapshot!(round_trip(&dto));
    }

    #[test]
    fn commit_imported_wire_shape() {
        let dto = CommitResult::Imported(ImportResult {
            batch_id: "import_batch_1".to_owned(),
            new_transactions: 42,
            attached_postings: 3,
            skipped_postings: 7,
            skips_by_cause: vec![CauseCount {
                cause: "unresolved account".to_owned(),
                postings: 7,
            }],
            unresolved_accounts: vec!["Expenses:Dining".to_owned()],
            unresolved_commodities: Vec::new(),
            created_tags: vec!["shop".to_owned()],
            created_accounts: vec!["Expenses:Snacks".to_owned()],
            warnings: vec!["Checking is archived".to_owned()],
            snapshot: Some("/backups/ledger/20260103-090000.pre-import.sqlite".to_owned()),
        });
        insta::assert_json_snapshot!(round_trip(&dto));
    }

    #[test]
    fn commit_changed_wire_shape() {
        insta::assert_json_snapshot!(round_trip(&CommitResult::Changed(preview())));
    }

    #[test]
    fn commit_failed_wire_shape() {
        let dto = CommitResult::Failed(ImportFailure {
            stage: FailureStageInfo::Engine,
            message: "database error: disk full".to_owned(),
            batch_id: Some("import_batch_1".to_owned()),
        });
        insta::assert_json_snapshot!(round_trip(&dto));
    }

    #[test]
    fn batches_wire_shape() {
        let counts = BatchCounts {
            new_transactions: 42,
            attached_postings: 3,
            skipped_postings: 7,
        };
        let dto = vec![
            ImportBatchInfo {
                id: "import_batch_3".to_owned(),
                profile: Some("groceries".to_owned()),
                importer: "csv".to_owned(),
                started_at: "2026-01-05T09:00:00Z".to_owned(),
                finished_at: Some("2026-01-05T09:00:05Z".to_owned()),
                state: BatchState::Complete,
                counts: Some(counts.clone()),
                discard: None,
            },
            ImportBatchInfo {
                id: "import_batch_2".to_owned(),
                profile: None,
                importer: "csv".to_owned(),
                started_at: "2026-01-04T09:00:00Z".to_owned(),
                finished_at: None,
                state: BatchState::Incomplete,
                counts: None,
                discard: None,
            },
            ImportBatchInfo {
                id: "import_batch_1".to_owned(),
                profile: Some("groceries".to_owned()),
                importer: "csv".to_owned(),
                started_at: "2026-01-03T09:00:00Z".to_owned(),
                finished_at: Some("2026-01-03T09:00:05Z".to_owned()),
                state: BatchState::Discarded,
                counts: Some(counts),
                discard: Some(DiscardInfo {
                    counts: discard_counts(),
                    discarded_at: "2026-01-06T10:00:00Z".to_owned(),
                    snapshot: Some("/backups/ledger/20260106-100000.pre-discard.sqlite".to_owned()),
                }),
            },
        ];
        insta::assert_json_snapshot!(round_trip(&dto));
    }

    #[test]
    fn discard_preview_ready_wire_shape() {
        let dto = DiscardPreview::Ready {
            counts: discard_counts(),
            snapshot_planned: true,
        };
        insta::assert_json_snapshot!(round_trip(&dto));
    }

    #[test]
    fn discard_preview_blocked_wire_shape() {
        let dto = DiscardPreview::Blocked {
            dependants: vec![DiscardDependant {
                batch_id: "import_batch_2".to_owned(),
                importer: "csv".to_owned(),
                started_at: "2026-01-04T09:00:00Z".to_owned(),
                postings: 3,
                transactions: 2,
            }],
        };
        insta::assert_json_snapshot!(round_trip(&dto));
    }
}
