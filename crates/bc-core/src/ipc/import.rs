//! Conversions from the import engine's types into `bc_ipc` import DTOs.
//!
//! Ids cross as their string form, timestamps as RFC 3339, dates as
//! `YYYY-MM-DD`, skip causes as their [`SkipCause::label`], and counts as
//! `u64`.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use super::balances_to_amounts;
use crate::AccountPath;
use crate::Diagnostic;
use crate::DiscardDependant;
use crate::DiscardOutcome;
use crate::DiscardRecord;
use crate::FailureStage;
use crate::ImportBatch;
use crate::ImportBatchCounts;
use crate::ImportOutcome;
use crate::ImportPlan;
use crate::LegFate;
use crate::PlannedLeg;
use crate::PlannedRow;
use crate::ProfileFailure;
use crate::RowFate;
use crate::SkipCause;
use crate::SourceFingerprint;

// MARK: Helpers

/// Widens a count to the wire's `u64`.
fn count(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Converts per-cause charges, keeping their order.
fn cause_counts(charged: &[(SkipCause, usize)]) -> Vec<bc_ipc::CauseCount> {
    charged
        .iter()
        .map(|&(cause, postings)| {
            bc_ipc::CauseCount::builder()
                .cause(cause.label().to_owned())
                .postings(count(postings))
                .build()
        })
        .collect()
}

/// Adds one to `key`'s tally.
fn bump(tally: &mut BTreeMap<String, usize>, key: String) {
    tally
        .entry(key)
        .and_modify(|n| *n = n.saturating_add(1))
        .or_insert(1);
}

/// Counts the skipped legs behind each unresolved account path and
/// commodity code, keyed as the plan's lists spell them.
///
/// A leg keeps the document's spelling. The plan lists the parsed path's
/// rendering and the trimmed code, so each key is normalised the same way.
///
/// # Returns
///
/// The account tally, then the commodity tally.
fn unresolved_leg_counts(
    rows: &[PlannedRow],
) -> (BTreeMap<String, usize>, BTreeMap<String, usize>) {
    let mut accounts = BTreeMap::new();
    let mut commodities = BTreeMap::new();
    for leg in rows.iter().flat_map(|row| &row.legs) {
        match leg.fate {
            LegFate::Skipped(SkipCause::UnresolvedAccount) => {
                let key = AccountPath::parse(&leg.account)
                    .map_or_else(|_| leg.account.clone(), |path| path.to_string());
                bump(&mut accounts, key);
            }
            LegFate::Skipped(SkipCause::UnresolvedCommodity) => {
                if let Some(amount) = &leg.amount {
                    bump(
                        &mut commodities,
                        amount.commodity().as_str().trim().to_owned(),
                    );
                }
            }
            LegFate::New | LegFate::Stored | LegFate::Skipped(_) => {}
        }
    }
    (accounts, commodities)
}

/// Pairs each name with its tally, zero when no leg was counted under it.
fn unresolved_items(
    names: &[String],
    tally: &BTreeMap<String, usize>,
) -> Vec<bc_ipc::UnresolvedItem> {
    names
        .iter()
        .map(|name| {
            bc_ipc::UnresolvedItem::builder()
                .name(name.clone())
                .postings(count(tally.get(name).copied().unwrap_or(0)))
                .build()
        })
        .collect()
}

// MARK: Preview

impl From<&RowFate> for bc_ipc::RowFateInfo {
    fn from(fate: &RowFate) -> Self {
        match fate {
            RowFate::Create => Self::Create,
            RowFate::Attach { owner } => Self::Attach {
                owner: owner.to_string(),
            },
            RowFate::AlreadyImported { owner } => Self::AlreadyImported {
                owner: owner.to_string(),
            },
            RowFate::Conflict { owners } => Self::Conflict {
                owners: owners.iter().map(ToString::to_string).collect(),
            },
            RowFate::Skipped(cause) => Self::Skipped {
                cause: cause.label().to_owned(),
            },
        }
    }
}

impl From<LegFate> for bc_ipc::LegFateInfo {
    fn from(fate: LegFate) -> Self {
        match fate {
            LegFate::New => Self::New,
            LegFate::Stored => Self::Stored,
            LegFate::Skipped(cause) => Self::Skipped {
                cause: cause.label().to_owned(),
            },
        }
    }
}

impl From<&PlannedLeg> for bc_ipc::PreviewLeg {
    fn from(leg: &PlannedLeg) -> Self {
        Self::builder()
            .account(leg.account.clone())
            .maybe_amount(leg.amount.as_ref().map(bc_ipc::Amount::from))
            .fate(leg.fate.into())
            .build()
    }
}

impl From<&Diagnostic> for bc_ipc::DiagnosticInfo {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self::builder()
            .location(diagnostic.location.clone())
            .cause(diagnostic.cause.label().to_owned())
            .detail(diagnostic.detail.clone())
            .build()
    }
}

impl From<&PlannedRow> for bc_ipc::PreviewRow {
    fn from(row: &PlannedRow) -> Self {
        Self::builder()
            .location(row.location.clone())
            .date(row.date.to_string())
            .description(row.description.clone())
            .fate((&row.fate).into())
            .legs(row.legs.iter().map(bc_ipc::PreviewLeg::from).collect())
            .diagnostics(
                row.diagnostics
                    .iter()
                    .map(bc_ipc::DiagnosticInfo::from)
                    .collect(),
            )
            .build()
    }
}

/// Builds an [`bc_ipc::ImportPreview`] from a dry run's plan.
pub trait ImportPreviewExt {
    /// Converts `plan`, the dry run of `profile` over a source that parsed
    /// to `fingerprint`.
    ///
    /// # Arguments
    ///
    /// * `profile` - The profile's name.
    /// * `fingerprint` - The parsed source's fingerprint.
    /// * `plan` - The dry run's plan.
    ///
    /// # Returns
    ///
    /// The preview DTO.
    fn from_plan(profile: &str, fingerprint: SourceFingerprint, plan: &ImportPlan) -> Self;
}

impl ImportPreviewExt for bc_ipc::ImportPreview {
    fn from_plan(profile: &str, fingerprint: SourceFingerprint, plan: &ImportPlan) -> Self {
        let row_locations: BTreeSet<&str> =
            plan.rows.iter().map(|row| row.location.as_str()).collect();
        let (accounts, commodities) = unresolved_leg_counts(&plan.rows);
        let already_imported = plan
            .rows
            .iter()
            .filter(|row| matches!(row.fate, RowFate::AlreadyImported { .. }))
            .count();
        Self::builder()
            .profile(profile.to_owned())
            .fingerprint(fingerprint.to_string())
            .new_transactions(count(plan.new_transactions))
            .attached_postings(count(plan.attached_postings))
            .already_imported(count(already_imported))
            .skipped_postings(count(plan.skipped_postings))
            .unresolved_accounts(unresolved_items(&plan.unresolved_accounts, &accounts))
            .unresolved_commodities(unresolved_items(&plan.unresolved_commodities, &commodities))
            .skips_by_cause(cause_counts(&plan.charged_by_cause))
            .warnings(plan.warnings.iter().map(ToString::to_string).collect())
            .would_create_accounts(plan.would_create_accounts.clone())
            .would_create_tags(plan.would_create_tags.clone())
            .account_totals(
                plan.account_totals
                    .iter()
                    .map(|(account, balances)| {
                        bc_ipc::AccountTotal::builder()
                            .account(account.clone())
                            .amounts(balances_to_amounts(balances))
                            .build()
                    })
                    .collect(),
            )
            .rows(plan.rows.iter().map(bc_ipc::PreviewRow::from).collect())
            .other_diagnostics(
                plan.diagnostics
                    .iter()
                    .filter(|d| !row_locations.contains(d.location.as_str()))
                    .map(bc_ipc::DiagnosticInfo::from)
                    .collect(),
            )
            .build()
    }
}

// MARK: Run results

impl From<FailureStage> for bc_ipc::FailureStageInfo {
    fn from(stage: FailureStage) -> Self {
        match stage {
            FailureStage::UnknownImporter => Self::UnknownImporter,
            FailureStage::Importer => Self::Importer,
            FailureStage::Engine => Self::Engine,
            FailureStage::SourceChanged => Self::SourceChanged,
        }
    }
}

impl From<&ProfileFailure> for bc_ipc::ImportFailure {
    fn from(failure: &ProfileFailure) -> Self {
        Self::builder()
            .stage(failure.stage.into())
            .message(failure.message.clone())
            .maybe_batch_id(failure.batch_id.as_ref().map(ToString::to_string))
            .build()
    }
}

/// Builds an [`bc_ipc::ImportResult`] from a committed run's outcome.
pub trait ImportResultExt {
    /// Converts `outcome`, with the snapshot the sweep took before it.
    ///
    /// # Arguments
    ///
    /// * `outcome` - The committed run's outcome.
    /// * `snapshot` - The pre-import snapshot, if one was taken.
    ///
    /// # Returns
    ///
    /// The result DTO.
    fn from_outcome(outcome: &ImportOutcome, snapshot: Option<&Path>) -> Self;
}

impl ImportResultExt for bc_ipc::ImportResult {
    fn from_outcome(outcome: &ImportOutcome, snapshot: Option<&Path>) -> Self {
        Self::builder()
            .batch_id(outcome.batch_id.to_string())
            .new_transactions(count(outcome.new_transactions))
            .attached_postings(count(outcome.attached_postings))
            .skipped_postings(count(outcome.skipped_postings))
            .skips_by_cause(cause_counts(&outcome.charged_by_cause))
            .unresolved_accounts(outcome.unresolved_accounts.clone())
            .unresolved_commodities(outcome.unresolved_commodities.clone())
            .created_tags(outcome.created_tags.clone())
            .created_accounts(outcome.created_accounts.clone())
            .warnings(outcome.warnings.iter().map(ToString::to_string).collect())
            .maybe_snapshot(snapshot.map(|path| path.display().to_string()))
            .build()
    }
}

// MARK: Batches

impl From<&ImportBatchCounts> for bc_ipc::BatchCounts {
    fn from(counts: &ImportBatchCounts) -> Self {
        Self::builder()
            .new_transactions(count(counts.new_transactions))
            .attached_postings(count(counts.attached_postings))
            .skipped_postings(count(counts.skipped()))
            .build()
    }
}

impl From<&DiscardOutcome> for bc_ipc::DiscardCounts {
    fn from(outcome: &DiscardOutcome) -> Self {
        Self::builder()
            .removed_postings(count(outcome.removed_postings))
            .removed_transactions(count(outcome.removed_transactions))
            .detached_adopted(count(outcome.detached_adopted))
            .freed_tombstones(count(outcome.freed_tombstones))
            .other_batch_references_removed(count(outcome.other_batch_references_removed))
            .other_batch_references_tombstoned(count(outcome.other_batch_references_tombstoned))
            .edited_postings(count(outcome.edited_postings))
            .reconciled_postings(count(outcome.reconciled_postings))
            .flagged_postings(count(outcome.flagged_postings))
            .removed_tags(count(outcome.removed_tags))
            .kept_tags(count(outcome.kept_tags))
            .removed_accounts(count(outcome.removed_accounts))
            .kept_accounts(count(outcome.kept_accounts))
            .reverted_fields(count(outcome.reverted_fields))
            .build()
    }
}

impl From<&DiscardRecord> for bc_ipc::DiscardInfo {
    fn from(record: &DiscardRecord) -> Self {
        Self::builder()
            .counts((&record.outcome).into())
            .discarded_at(record.discarded_at.to_string())
            .maybe_snapshot(
                record
                    .snapshot
                    .as_ref()
                    .map(|path| path.display().to_string()),
            )
            .build()
    }
}

impl From<&DiscardDependant> for bc_ipc::DiscardDependant {
    fn from(dependant: &DiscardDependant) -> Self {
        Self::builder()
            .batch_id(dependant.batch_id.to_string())
            .importer(dependant.importer.clone())
            .started_at(dependant.started_at.to_string())
            .postings(count(dependant.postings))
            .transactions(count(dependant.transactions))
            .build()
    }
}

/// Builds an [`bc_ipc::ImportBatchInfo`] from a stored batch.
pub trait ImportBatchInfoExt {
    /// Converts `batch`, naming its profile and attaching its discard.
    ///
    /// # Arguments
    ///
    /// * `batch` - The stored batch.
    /// * `profile` - The name of the profile that drove it, if it still exists.
    /// * `discard` - The batch's discard record, if it was discarded.
    ///
    /// # Returns
    ///
    /// The history row DTO.
    fn from_batch(
        batch: &ImportBatch,
        profile: Option<&str>,
        discard: Option<&DiscardRecord>,
    ) -> Self;
}

impl ImportBatchInfoExt for bc_ipc::ImportBatchInfo {
    fn from_batch(
        batch: &ImportBatch,
        profile: Option<&str>,
        discard: Option<&DiscardRecord>,
    ) -> Self {
        let state = if batch.discarded_at.is_some() {
            bc_ipc::BatchState::Discarded
        } else if batch.finished_at.is_some() {
            bc_ipc::BatchState::Complete
        } else {
            bc_ipc::BatchState::Incomplete
        };
        Self::builder()
            .id(batch.id.to_string())
            .maybe_profile(profile.map(str::to_owned))
            .importer(batch.importer.clone())
            .started_at(batch.started_at.to_string())
            .maybe_finished_at(batch.finished_at.map(|at| at.to_string()))
            .state(state)
            .maybe_counts(batch.counts.as_ref().map(bc_ipc::BatchCounts::from))
            .maybe_discard(discard.map(bc_ipc::DiscardInfo::from))
            .build()
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::Balances;
    use bc_models::CommodityCode;
    use bc_models::ImportBatchId;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use super::*;
    use crate::Warning;

    /// `value` in AUD.
    fn aud(value: Decimal) -> Amount {
        Amount::new(value, CommodityCode::new("AUD"))
    }

    /// A leg on `account`.
    fn leg(account: &str, amount: Option<Amount>, fate: LegFate) -> PlannedLeg {
        PlannedLeg {
            account: account.to_owned(),
            amount,
            fate,
        }
    }

    /// A Supermarket row dated 2026-01-03 at `location`.
    fn row(
        location: &str,
        fate: RowFate,
        legs: Vec<PlannedLeg>,
        diagnostics: Vec<Diagnostic>,
    ) -> PlannedRow {
        PlannedRow {
            location: location.to_owned(),
            date: date(2026, 1, 3),
            description: "Supermarket".to_owned(),
            fate,
            legs,
            diagnostics,
        }
    }

    /// A diagnostic at `location`.
    fn diagnostic(location: &str, cause: SkipCause, detail: &str) -> Diagnostic {
        Diagnostic {
            location: location.to_owned(),
            cause,
            detail: detail.to_owned(),
        }
    }

    /// Parses an RFC 3339 instant.
    fn at(text: &str) -> Timestamp {
        text.parse().expect("timestamp")
    }

    /// A plan over three rows: one created with an unresolved leg the
    /// document spells with a stray space, one already imported, one
    /// skipped for a commodity code with a stray space; plus a diagnostic
    /// matching no row.
    fn plan() -> ImportPlan {
        let unresolved = diagnostic(
            "statement.csv row 2",
            SkipCause::UnresolvedAccount,
            "Expenses:Dining (resolved as far as 'Expenses', missing 'Dining')",
        );
        ImportPlan {
            new_transactions: 1,
            attached_postings: 0,
            skipped_postings: 2,
            unresolved_account_postings: 1,
            unresolved_commodity_postings: 1,
            other_skipped_postings: 0,
            unresolved_accounts: vec!["Expenses:Dining".to_owned()],
            unresolved_commodities: vec!["XYZ".to_owned()],
            would_create_tags: vec!["shop".to_owned()],
            would_create_accounts: Vec::new(),
            account_totals: vec![(
                "Checking".to_owned(),
                [aud(dec!(-42.00))].into_iter().collect::<Balances>(),
            )],
            charged_by_cause: vec![
                (SkipCause::UnresolvedAccount, 1),
                (SkipCause::UnresolvedCommodity, 1),
            ],
            diagnostics: vec![
                unresolved.clone(),
                diagnostic(
                    "statement.csv",
                    SkipCause::IgnoredDeclaration,
                    "duplicate open for Checking",
                ),
            ],
            warnings: vec![Warning::PostingIntoArchivedAccount {
                account_id: AccountId::new(),
                account_path: "Checking".to_owned(),
            }],
            rows: vec![
                row(
                    "statement.csv row 2",
                    RowFate::Create,
                    vec![
                        leg("Checking", Some(aud(dec!(-42.00))), LegFate::New),
                        leg(
                            "Expenses: Dining",
                            Some(aud(dec!(42.00))),
                            LegFate::Skipped(SkipCause::UnresolvedAccount),
                        ),
                    ],
                    vec![unresolved],
                ),
                row(
                    "statement.csv row 3",
                    RowFate::AlreadyImported {
                        owner: TransactionId::new(),
                    },
                    vec![leg("Checking", Some(aud(dec!(-5.00))), LegFate::Stored)],
                    Vec::new(),
                ),
                row(
                    "statement.csv row 4",
                    RowFate::Skipped(SkipCause::UnresolvedCommodity),
                    vec![leg(
                        "Checking",
                        Some(Amount::new(dec!(5), CommodityCode::new(" XYZ"))),
                        LegFate::Skipped(SkipCause::UnresolvedCommodity),
                    )],
                    Vec::new(),
                ),
            ],
        }
    }

    /// Every discard count distinct, so a swapped field shows.
    fn outcome(batch_id: &ImportBatchId) -> DiscardOutcome {
        DiscardOutcome {
            batch_id: batch_id.clone(),
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

    /// The DTO [`outcome`] crosses as.
    fn crossed_counts() -> bc_ipc::DiscardCounts {
        bc_ipc::DiscardCounts::builder()
            .removed_postings(120)
            .removed_transactions(42)
            .detached_adopted(1)
            .freed_tombstones(2)
            .other_batch_references_removed(3)
            .other_batch_references_tombstoned(4)
            .edited_postings(5)
            .reconciled_postings(6)
            .flagged_postings(7)
            .removed_tags(8)
            .kept_tags(9)
            .removed_accounts(10)
            .kept_accounts(11)
            .reverted_fields(12)
            .build()
    }

    /// A csv batch started 2026-01-03 09:00, finished and discarded as asked.
    fn batch(finished: bool, discarded: bool) -> ImportBatch {
        ImportBatch {
            id: ImportBatchId::new(),
            profile_id: None,
            importer: "csv".to_owned(),
            started_at: at("2026-01-03T09:00:00Z"),
            finished_at: finished.then(|| at("2026-01-03T09:00:05Z")),
            discarded_at: discarded.then(|| at("2026-01-04T10:00:00Z")),
            counts: finished.then_some(ImportBatchCounts {
                new_transactions: 42,
                attached_postings: 3,
                unresolved_account_postings: 4,
                unresolved_commodity_postings: 2,
                other_skipped_postings: 1,
            }),
        }
    }

    /// An unresolved item.
    fn item(name: &str, postings: u64) -> bc_ipc::UnresolvedItem {
        bc_ipc::UnresolvedItem::builder()
            .name(name.to_owned())
            .postings(postings)
            .build()
    }

    /// A cause count.
    fn cause(label: &str, postings: u64) -> bc_ipc::CauseCount {
        bc_ipc::CauseCount::builder()
            .cause(label.to_owned())
            .postings(postings)
            .build()
    }

    /// The fingerprint every preview test uses.
    fn fingerprint() -> SourceFingerprint {
        "00000000000000ff".parse().expect("fingerprint")
    }

    #[test]
    fn a_preview_counts_unresolved_legs_under_the_plans_spelling() {
        let preview = bc_ipc::ImportPreview::from_plan("groceries", fingerprint(), &plan());

        assert_eq!(preview.profile, "groceries");
        assert_eq!(preview.fingerprint, "00000000000000ff");
        assert_eq!(
            preview.unresolved_accounts,
            vec![item("Expenses:Dining", 1)]
        );
        assert_eq!(preview.unresolved_commodities, vec![item("XYZ", 1)]);
        assert_eq!(preview.already_imported, 1);
        assert_eq!(
            preview.skips_by_cause,
            vec![
                cause("unresolved account", 1),
                cause("unregistered commodity", 1)
            ]
        );
        assert_eq!(preview.warnings, vec!["Checking is archived".to_owned()]);
        assert_eq!(
            preview.account_totals,
            vec![
                bc_ipc::AccountTotal::builder()
                    .account("Checking".to_owned())
                    .amounts(vec![bc_ipc::Amount::new(dec!(-42.00), "AUD")])
                    .build()
            ]
        );
    }

    #[test]
    fn diagnostics_split_between_their_rows_and_the_rest() {
        let preview = bc_ipc::ImportPreview::from_plan("groceries", fingerprint(), &plan());

        let per_row: Vec<usize> = preview.rows.iter().map(|r| r.diagnostics.len()).collect();
        assert_eq!(per_row, vec![1, 0, 0]);
        assert_eq!(
            preview.other_diagnostics,
            vec![
                bc_ipc::DiagnosticInfo::builder()
                    .location("statement.csv".to_owned())
                    .cause("ignored declaration".to_owned())
                    .detail("duplicate open for Checking".to_owned())
                    .build()
            ]
        );
    }

    #[test]
    fn a_row_crosses_with_its_date_legs_and_amounts() {
        let preview = bc_ipc::ImportPreview::from_plan("groceries", fingerprint(), &plan());

        let first = preview.rows.first().expect("a row");
        assert_eq!(first.date, "2026-01-03");
        assert_eq!(first.fate, bc_ipc::RowFateInfo::Create);
        assert_eq!(
            first.legs,
            vec![
                bc_ipc::PreviewLeg::builder()
                    .account("Checking".to_owned())
                    .amount(bc_ipc::Amount::new(dec!(-42.00), "AUD"))
                    .fate(bc_ipc::LegFateInfo::New)
                    .build(),
                bc_ipc::PreviewLeg::builder()
                    .account("Expenses: Dining".to_owned())
                    .amount(bc_ipc::Amount::new(dec!(42.00), "AUD"))
                    .fate(bc_ipc::LegFateInfo::Skipped {
                        cause: "unresolved account".to_owned(),
                    })
                    .build(),
            ]
        );
    }

    #[test]
    fn every_row_fate_crosses_with_string_ids() {
        let owner = TransactionId::new();
        let other = TransactionId::new();
        let fates = [
            RowFate::Create,
            RowFate::Attach {
                owner: owner.clone(),
            },
            RowFate::AlreadyImported {
                owner: owner.clone(),
            },
            RowFate::Conflict {
                owners: vec![owner.clone(), other.clone()],
            },
            RowFate::Skipped(SkipCause::MultiOwnerConflict),
        ];

        let crossed: Vec<bc_ipc::RowFateInfo> =
            fates.iter().map(bc_ipc::RowFateInfo::from).collect();

        assert_eq!(
            crossed,
            vec![
                bc_ipc::RowFateInfo::Create,
                bc_ipc::RowFateInfo::Attach {
                    owner: owner.to_string()
                },
                bc_ipc::RowFateInfo::AlreadyImported {
                    owner: owner.to_string()
                },
                bc_ipc::RowFateInfo::Conflict {
                    owners: vec![owner.to_string(), other.to_string()]
                },
                bc_ipc::RowFateInfo::Skipped {
                    cause: "multi-owner conflict".to_owned()
                },
            ]
        );
    }

    #[rstest]
    #[case::unknown_importer(
        FailureStage::UnknownImporter,
        bc_ipc::FailureStageInfo::UnknownImporter
    )]
    #[case::importer(FailureStage::Importer, bc_ipc::FailureStageInfo::Importer)]
    #[case::engine(FailureStage::Engine, bc_ipc::FailureStageInfo::Engine)]
    #[case::source_changed(FailureStage::SourceChanged, bc_ipc::FailureStageInfo::SourceChanged)]
    fn every_failure_stage_crosses(
        #[case] stage: FailureStage,
        #[case] expected: bc_ipc::FailureStageInfo,
    ) {
        assert_eq!(bc_ipc::FailureStageInfo::from(stage), expected);
    }

    #[test]
    fn a_failure_keeps_the_batch_it_left_open() {
        let batch_id = ImportBatchId::new();
        let mut failure = ProfileFailure::new(FailureStage::Engine, "disk full");
        failure.batch_id = Some(batch_id.clone());

        assert_eq!(
            bc_ipc::ImportFailure::from(&failure),
            bc_ipc::ImportFailure::builder()
                .stage(bc_ipc::FailureStageInfo::Engine)
                .message("disk full".to_owned())
                .batch_id(batch_id.to_string())
                .build()
        );
    }

    #[test]
    fn a_result_carries_its_snapshot_path() {
        let batch_id = ImportBatchId::new();
        let outcome = ImportOutcome {
            batch_id: batch_id.clone(),
            new_transactions: 42,
            attached_postings: 3,
            skipped_postings: 4,
            unresolved_account_postings: 4,
            unresolved_commodity_postings: 0,
            other_skipped_postings: 0,
            unresolved_accounts: vec!["Expenses:Dining".to_owned()],
            unresolved_commodities: Vec::new(),
            created_tags: vec!["shop".to_owned()],
            created_accounts: Vec::new(),
            charged_by_cause: vec![(SkipCause::UnresolvedAccount, 4)],
            diagnostics: Vec::new(),
            warnings: Vec::new(),
        };
        let path = Path::new("/backups/ledger/20260103-090000.pre-import.sqlite");

        let result = bc_ipc::ImportResult::from_outcome(&outcome, Some(path));

        assert_eq!(result.batch_id, batch_id.to_string());
        assert_eq!(result.skips_by_cause, vec![cause("unresolved account", 4)]);
        assert_eq!(
            result.snapshot.as_deref(),
            Some("/backups/ledger/20260103-090000.pre-import.sqlite")
        );
    }

    #[rstest]
    #[case::complete(true, false, bc_ipc::BatchState::Complete)]
    #[case::incomplete(false, false, bc_ipc::BatchState::Incomplete)]
    #[case::discarded(true, true, bc_ipc::BatchState::Discarded)]
    fn a_batch_state_follows_its_timestamps(
        #[case] finished: bool,
        #[case] discarded: bool,
        #[case] expected: bc_ipc::BatchState,
    ) {
        let info = bc_ipc::ImportBatchInfo::from_batch(&batch(finished, discarded), None, None);
        assert_eq!(info.state, expected);
    }

    #[test]
    fn a_discarded_batch_carries_its_profile_counts_and_discard() {
        let discarded = batch(true, true);
        let record = DiscardRecord {
            outcome: outcome(&discarded.id),
            discarded_at: at("2026-01-04T10:00:00Z"),
            snapshot: Some(PathBuf::from(
                "/backups/ledger/20260104-100000.pre-discard.sqlite",
            )),
        };

        let info =
            bc_ipc::ImportBatchInfo::from_batch(&discarded, Some("groceries"), Some(&record));

        assert_eq!(
            info,
            bc_ipc::ImportBatchInfo::builder()
                .id(discarded.id.to_string())
                .profile("groceries".to_owned())
                .importer("csv".to_owned())
                .started_at("2026-01-03T09:00:00Z".to_owned())
                .finished_at("2026-01-03T09:00:05Z".to_owned())
                .state(bc_ipc::BatchState::Discarded)
                .counts(
                    bc_ipc::BatchCounts::builder()
                        .new_transactions(42)
                        .attached_postings(3)
                        .skipped_postings(7)
                        .build()
                )
                .discard(
                    bc_ipc::DiscardInfo::builder()
                        .counts(crossed_counts())
                        .discarded_at("2026-01-04T10:00:00Z".to_owned())
                        .snapshot("/backups/ledger/20260104-100000.pre-discard.sqlite".to_owned())
                        .build()
                )
                .build()
        );
    }

    #[test]
    fn a_dependant_crosses_as_strings() {
        let batch_id = ImportBatchId::new();
        let dependant = DiscardDependant {
            batch_id: batch_id.clone(),
            importer: "csv".to_owned(),
            started_at: at("2026-01-04T09:00:00Z"),
            postings: 3,
            transactions: 2,
        };

        assert_eq!(
            bc_ipc::DiscardDependant::from(&dependant),
            bc_ipc::DiscardDependant::builder()
                .batch_id(batch_id.to_string())
                .importer("csv".to_owned())
                .started_at("2026-01-04T09:00:00Z".to_owned())
                .postings(3)
                .transactions(2)
                .build()
        );
    }
}
