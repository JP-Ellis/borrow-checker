//! Applying a planned round trip to the database as one import batch.
//!
//! Each written transaction is one SQLite transaction: an edit commits with
//! its flag, the sources of its new legs and its edit record; a create with
//! the sources of its legs; a delete with its edit record. A run that stops
//! part-way leaves whole transactions written and its batch open, and every
//! written transaction is reachable from that batch for a discard.

use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::ImportBatchId;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;
use sqlx::SqlitePool;

use super::Action;
use super::ApplyPlan;
use super::PreflightError;
use super::Resolver;
use super::RoundTripService;
use super::Sidecar;
use super::SkipReason;
use super::database_id;
use super::plan;
use super::plan::unchanged_since;
use super::sidecar::VERSION;
use crate::BcError;
use crate::BcResult;
use crate::DeleteMode;
use crate::ImportBatchCounts;
use crate::ImportBatchService;
use crate::ImportOutput;
use crate::SlotHolder;
use crate::SourceService;
use crate::TransactionService;
use crate::import::batch::record_edit_in_tx;

/// The importer an apply batch records, and the reference on every source
/// row apply writes. A bank row carries its own reference or none, so its
/// fingerprint never equals an apply leg's.
const APPLY: &str = "bc-apply";

/// What an apply run wrote, skipped and refused.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[expect(
    clippy::module_name_repetitions,
    reason = "the type is re-exported at `roundtrip` where the prefix is needed"
)]
#[non_exhaustive]
pub struct ApplyOutcome {
    /// The batch recording the run; `None` when it had nothing to write.
    pub batch_id: Option<ImportBatchId>,
    /// Transactions edited.
    pub edited: usize,
    /// Transactions created.
    pub created: usize,
    /// Transactions deleted.
    pub deleted: usize,
    /// Transactions left as they are, a create already applied included.
    pub skipped: usize,
    /// Transactions refused, in file order.
    pub refused: Vec<Refusal>,
    /// The plan's warnings, then the advisory warnings each write raised.
    pub warnings: Vec<String>,
}

/// One transaction apply left alone, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Refusal {
    /// The transaction's position among the file's transactions, which is
    /// also its action's position in the plan.
    pub index: usize,
    /// The transaction's handle, when it has one.
    pub handle: Option<String>,
    /// Why.
    pub reason: String,
}

impl Refusal {
    /// Creates a refusal of the file transaction at `index`.
    #[must_use]
    #[inline]
    pub fn new(index: usize, handle: Option<String>, reason: impl Into<String>) -> Self {
        Self {
            index,
            handle,
            reason: reason.into(),
        }
    }
}

impl RoundTripService {
    /// Plans applying `output`, an edited working file, against the stored
    /// state of the transactions `sidecar` exported.
    ///
    /// # Arguments
    ///
    /// * `output` - The parsed working file.
    /// * `sidecar` - The export's sidecar.
    ///
    /// # Returns
    ///
    /// The plan, or the [`PreflightError`] refusing the whole file. A sidecar
    /// of another format version or from another database is refused before
    /// the file is read. A new transaction whose every leg an earlier apply
    /// already stored is planned as a skip, already applied, so a rerun of
    /// the same file plans no write; every other create carries its legs'
    /// positions among identical legs of the file's creates, counted past
    /// the slots of the transactions the file names by handle.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database or data failure.
    #[inline]
    pub async fn plan_file(
        &self,
        output: &ImportOutput,
        sidecar: &Sidecar,
    ) -> BcResult<Result<ApplyPlan, PreflightError>> {
        let mut problems = Vec::new();
        if sidecar.version != VERSION {
            problems.push(format!(
                "sidecar version {} is not supported; this build reads version {VERSION}",
                sidecar.version
            ));
        }
        if sidecar.database_id != database_id(&self.pool).await? {
            problems.push("the sidecar was written from another database".to_owned());
        }
        if !problems.is_empty() {
            return Ok(Err(PreflightError { problems }));
        }

        let resolver = Resolver::load(&self.pool).await?;
        let mut current = HashMap::new();
        for entry in &sidecar.entries {
            let id = entry.transaction.id();
            match self.transactions.find_by_id(id).await {
                Ok(stored) => {
                    current.insert(id.clone(), stored);
                }
                Err(BcError::NotFound(_)) => {}
                Err(error) => return Err(error),
            }
        }
        let ids: Vec<&TransactionId> = current.keys().collect();
        let protected = protected_postings(&self.pool, &ids).await?;
        let mut planned = match plan(output, sidecar, &resolver, &current, &protected) {
            Ok(planned) => planned,
            Err(refused) => return Ok(Err(refused)),
        };
        let sources = SourceService::new(self.pool.clone());
        Slots::load(&sources, &planned.actions)
            .await?
            .settle(&mut planned.actions, sidecar);
        Ok(Ok(planned))
    }

    /// Applies `plan` as one import batch, importer `bc-apply`.
    ///
    /// Actions run in file order. An edit is written only while the stored
    /// transaction still equals its base, reconciliation included, and ends
    /// `Flagged`. A create's legs take the positions the plan gave them while
    /// those slots are free. A delete
    /// keeps the transaction's source references as orphans. Each edit and
    /// delete records the transaction's state before and after against the
    /// batch. A transaction found stale, or refused by validation, is
    /// reported and the rest still applies. A plan with nothing to write
    /// opens no batch.
    ///
    /// # Arguments
    ///
    /// * `plan` - The plan from [`Self::plan_file`].
    ///
    /// # Returns
    ///
    /// What the run wrote, skipped and refused.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on database failure. Transactions already written
    /// stay written under the batch, which is left open.
    #[inline]
    pub async fn apply(&self, plan: ApplyPlan) -> BcResult<ApplyOutcome> {
        let ApplyPlan { actions, warnings } = plan;
        let mut outcome = ApplyOutcome {
            warnings,
            ..ApplyOutcome::default()
        };
        let sources = SourceService::new(self.pool.clone());
        let mut slots = Slots::load(&sources, &actions).await?;
        let writes = actions.iter().any(|action| {
            matches!(
                *action,
                Action::Edit { .. } | Action::Create { .. } | Action::Delete { .. }
            )
        });
        let batch = if writes {
            Some(
                ImportBatchService::new(self.pool.clone())
                    .open(None, APPLY)
                    .await?,
            )
        } else {
            None
        };
        let mut run = Run {
            service: self,
            sources: &sources,
            slots: &mut slots,
            outcome: &mut outcome,
            attached: 0,
        };
        for (index, action) in actions.into_iter().enumerate() {
            if let Err(error) = run.step(batch.as_ref(), index, action).await {
                return Err(left_open(batch.as_ref(), error));
            }
        }
        let attached = run.attached;

        if let Some(ref id) = batch {
            let closed = ImportBatchService::new(self.pool.clone())
                .close(
                    id,
                    ImportBatchCounts {
                        new_transactions: outcome.created,
                        attached_postings: attached,
                        ..ImportBatchCounts::default()
                    },
                )
                .await;
            if let Err(error) = closed {
                return Err(left_open(batch.as_ref(), error));
            }
        }
        outcome.batch_id = batch;
        Ok(outcome)
    }
}

/// Reports the batch a stopped run leaves open, so the user can discard it,
/// and passes `error` on.
fn left_open(batch: Option<&ImportBatchId>, error: BcError) -> BcError {
    if let Some(id) = batch {
        tracing::warn!(
            batch_id = %id,
            %error,
            "apply stopped part-way; `import discard {id}` takes back what it wrote"
        );
    }
    error
}

// MARK: Execution

/// One apply run in progress.
struct Run<'a> {
    /// The service whose pool and transactions the run writes through.
    service: &'a RoundTripService,
    /// Attaches the sources of apply's legs.
    sources: &'a SourceService,
    /// The occurrence slots of apply's fingerprints.
    slots: &'a mut Slots,
    /// The tallies so far.
    outcome: &'a mut ApplyOutcome,
    /// Legs added to existing transactions.
    attached: usize,
}

/// What one written transaction added to the run.
#[derive(Default)]
struct Written {
    /// Legs added to an existing transaction.
    attached: usize,
    /// Advisory warnings the write raised.
    warnings: Vec<crate::Warning>,
    /// The create was found stored at write time and nothing was written.
    already_applied: bool,
}

impl Run<'_> {
    /// Carries out one action and tallies it.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on a failure that is not one transaction's alone,
    /// such as a database failure.
    async fn step(
        &mut self,
        open: Option<&ImportBatchId>,
        index: usize,
        action: Action,
    ) -> BcResult<()> {
        let (handle, label, result) = match (action, open) {
            (Action::Skip { .. }, _) => {
                self.outcome.skipped = self.outcome.skipped.saturating_add(1);
                return Ok(());
            }
            (Action::Refuse { handle, reason }, _) => {
                self.outcome
                    .refused
                    .push(Refusal::new(index, handle, reason));
                return Ok(());
            }
            (
                Action::Edit {
                    handle,
                    base,
                    edited,
                },
                Some(batch),
            ) => {
                let result = self.edit(batch, &base, &edited).await;
                if result.is_ok() {
                    self.outcome.edited = self.outcome.edited.saturating_add(1);
                }
                (Some(handle.clone()), format!("`{handle}`"), result)
            }
            (
                Action::Create {
                    edited, positions, ..
                },
                Some(batch),
            ) => {
                let result = self.create(batch, &edited, &positions).await;
                match result {
                    Ok(Written {
                        already_applied: true,
                        ..
                    }) => self.outcome.skipped = self.outcome.skipped.saturating_add(1),
                    Ok(_) => self.outcome.created = self.outcome.created.saturating_add(1),
                    Err(_) => {}
                }
                let label = format!("new `{}` on {}", edited.description(), edited.date());
                (None, label, result)
            }
            (Action::Delete { handle, base }, Some(batch)) => {
                let result = self.delete(batch, &base).await;
                if result.is_ok() {
                    self.outcome.deleted = self.outcome.deleted.saturating_add(1);
                }
                (Some(handle.clone()), format!("`{handle}`"), result)
            }
            (Action::Edit { .. } | Action::Create { .. } | Action::Delete { .. }, None) => {
                return Err(BcError::InvalidInput(
                    "apply wrote outside its batch".to_owned(),
                ));
            }
        };
        match result {
            Ok(written) => {
                self.attached = self.attached.saturating_add(written.attached);
                self.outcome.warnings.extend(
                    written
                        .warnings
                        .iter()
                        .map(|warning| format!("{label}: {warning}")),
                );
            }
            Err(error) => self
                .outcome
                .refused
                .push(Refusal::new(index, handle, refusal(error)?)),
        }
        Ok(())
    }

    /// Replaces `base` with `edited`, flags it, sources its new legs and
    /// records it, in one database transaction.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Conflict`] when the stored transaction no longer
    /// equals `base`, reconciliation included, and [`BcError::NotFound`] when
    /// it is gone.
    async fn edit(
        &mut self,
        batch: &ImportBatchId,
        base: &Transaction,
        edited: &Transaction,
    ) -> BcResult<Written> {
        let new_legs: Vec<&Posting> = edited
            .postings()
            .iter()
            .filter(|leg| !base.postings().iter().any(|old| old.id() == leg.id()))
            .collect();
        let mut db_tx = self.service.pool.begin_with("BEGIN IMMEDIATE").await?;
        let before = TransactionService::find_in(&mut db_tx, base.id()).await?;
        if !unchanged_since(&before, base) {
            return Err(BcError::Conflict(base.id().to_string()));
        }
        let warned = self
            .service
            .transactions
            .edit_if_unchanged_in_tx(&mut db_tx, base, edited.clone())
            .await?;
        let written = TransactionService::find_in(&mut db_tx, base.id()).await?;
        TransactionService::reconcile_in_tx(&mut db_tx, &written, Reconciliation::Flagged).await?;
        for leg in &new_legs {
            let occurrence = self.slots.claim(edited, leg, None);
            self.sources
                .attach_in_tx(&mut db_tx, &source_ref(batch, edited, leg, occurrence))
                .await?;
        }
        let after = TransactionService::find_in(&mut db_tx, base.id()).await?;
        record_edit_in_tx(&mut db_tx, batch, base.id(), &before, Some(&after), &[]).await?;
        db_tx.commit().await?;
        Ok(Written {
            attached: new_legs.len(),
            warnings: warned.warnings,
            ..Written::default()
        })
    }

    /// Stores `edited` with a source on every leg, in one database
    /// transaction. Each leg takes its planned position as its occurrence
    /// while that slot is free.
    ///
    /// The plan's verdict is checked again inside the transaction: when every
    /// leg's planned slot is stored by now, as when a plan is applied twice or
    /// another writer applied the same file since planning, nothing is
    /// written and the create reports itself already applied.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::BadData`] when the transaction is unrepresentable.
    async fn create(
        &mut self,
        batch: &ImportBatchId,
        edited: &Transaction,
        positions: &[u32],
    ) -> BcResult<Written> {
        let mut db_tx = self.service.pool.begin_with("BEGIN IMMEDIATE").await?;
        let mut held = Vec::with_capacity(edited.postings().len());
        for (leg, position) in edited.postings().iter().zip(positions) {
            let (account, fingerprint) = slot_key(edited, leg);
            let found: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM transaction_sources \
                 WHERE account_id = ? AND fingerprint = ? AND occurrence = ?",
            )
            .bind(account)
            .bind(fingerprint)
            .bind(i64::from(*position))
            .fetch_optional(&mut *db_tx)
            .await?;
            held.push(found.is_some());
        }
        if held.len() == edited.postings().len() && stored_whole(&held) {
            return Ok(Written {
                already_applied: true,
                ..Written::default()
            });
        }
        let warned = self
            .service
            .transactions
            .create_in_tx(&mut db_tx, edited.clone())
            .await?;
        for (index, leg) in edited.postings().iter().enumerate() {
            let occurrence = self.slots.claim(edited, leg, positions.get(index).copied());
            self.sources
                .attach_in_tx(&mut db_tx, &source_ref(batch, edited, leg, occurrence))
                .await?;
        }
        db_tx.commit().await?;
        Ok(Written {
            warnings: warned.warnings,
            ..Written::default()
        })
    }

    /// Deletes the stored transaction, keeping its provenance, and records
    /// it with the posting each of its sources named, in one database
    /// transaction.
    ///
    /// # Errors
    ///
    /// Returns [`BcError::Conflict`] when the stored transaction no longer
    /// equals `base`, and [`BcError::NotFound`] when it is gone.
    async fn delete(&mut self, batch: &ImportBatchId, base: &Transaction) -> BcResult<Written> {
        let mut db_tx = self.service.pool.begin_with("BEGIN IMMEDIATE").await?;
        let before = TransactionService::find_in(&mut db_tx, base.id()).await?;
        if !unchanged_since(&before, base) {
            return Err(BcError::Conflict(base.id().to_string()));
        }
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, posting_id FROM transaction_sources \
             WHERE transaction_id = ? AND posting_id IS NOT NULL ORDER BY id",
        )
        .bind(base.id().to_string())
        .fetch_all(&mut *db_tx)
        .await?;
        let links = rows
            .into_iter()
            .map(|(source, posting)| {
                Ok((
                    source
                        .parse::<SourceRefId>()
                        .map_err(|e| BcError::BadData(e.to_string()))?,
                    posting
                        .parse::<PostingId>()
                        .map_err(|e| BcError::BadData(e.to_string()))?,
                ))
            })
            .collect::<BcResult<Vec<_>>>()?;
        TransactionService::delete_in_tx(&mut db_tx, &before, DeleteMode::KeepProvenance).await?;
        record_edit_in_tx(&mut db_tx, batch, base.id(), &before, None, &links).await?;
        db_tx.commit().await?;
        Ok(Written::default())
    }
}

/// The reason a failed write refuses its transaction, or the error itself
/// when the failure is not that transaction's alone. A refused write was
/// rolled back whole.
///
/// # Errors
///
/// Returns `error` unless it is a stale base, a missing transaction or a
/// validation failure.
fn refusal(error: BcError) -> BcResult<String> {
    match error {
        BcError::Conflict(_) => Ok("changed since export".to_owned()),
        BcError::NotFound(_) => Ok("deleted since export".to_owned()),
        BcError::BadData(reason) | BcError::InvalidInput(reason) => Ok(reason),
        other @ (BcError::DatabaseInUse(_)
        | BcError::AlreadyArchived(_)
        | BcError::AlreadyClosed(_)
        | BcError::NotClosed(_)
        | BcError::TagInUse(_)
        | BcError::MarkerConflict { .. }
        | BcError::CommodityInUse(_)
        | BcError::EmptyCommodityCode
        | BcError::NotMerged(_)
        | BcError::Query(_)
        | BcError::Database(_)
        | BcError::Serialisation(_)
        | BcError::Migration(_)
        | BcError::DiscardBlocked { .. }
        | BcError::InvalidAccountKind { .. }
        | BcError::NotMergeable { .. }
        | BcError::NotUnmergeable { .. }) => Err(other),
    }
}

/// The source row apply writes for `leg` of `tx`.
fn source_ref(
    batch: &ImportBatchId,
    tx: &Transaction,
    leg: &Posting,
    occurrence: u32,
) -> SourceRef {
    SourceRef::builder()
        .id(SourceRefId::new())
        .transaction_id(tx.id().clone())
        .posting_id(Some(leg.id().clone()))
        .account_id(leg.account_id().clone())
        .date(tx.date())
        .narration(tx.description())
        .amount(leg.amount().cloned())
        .reference(Some(APPLY.to_owned()))
        .occurrence(occurrence)
        .import_batch_id(Some(batch.clone()))
        .owns_posting(true)
        .created_at(Timestamp::now())
        .build()
}

// MARK: Slots

/// A source row's dedup scope: account and fingerprint.
type SlotKey = (String, String);

/// Whether a create whose legs' planned slots are `held` was already
/// applied: it has legs and every one is stored. Planning and the write-time
/// check both decide through this.
fn stored_whole(held: &[bool]) -> bool {
    !held.is_empty() && held.iter().all(|&stored| stored)
}

/// The scope apply's source row for `leg` of `tx` occupies.
fn slot_key(tx: &Transaction, leg: &Posting) -> SlotKey {
    (
        leg.account_id().to_string(),
        SourceRef::compute_fingerprint(tx.date(), tx.description(), leg.amount(), Some(APPLY)),
    )
}

/// The occurrence slots of apply's fingerprints.
///
/// A create's legs take their position among identical legs of the file's
/// creates, counted over the slots no transaction the file names by handle
/// holds, as an import's rows do, and are written at that position while it
/// is free, so a rerun finds each one at the slot it took the first time.
/// A slot held by a handled transaction is that transaction's, whatever the
/// file now says of it, so it never stands for a create. Any other row, or a
/// leg whose position is taken, takes the slot after every one held or
/// planned. [`Slots::settle`] is the one place that decides whether an
/// earlier apply already stored a create.
struct Slots {
    /// Occurrences stored before the run, by scope, with the transaction
    /// each belongs to, a deleted one included.
    stored: HashMap<SlotKey, Vec<(u32, TransactionId)>>,
    /// Occurrences held now, by scope: those stored and those the run claimed.
    held: HashMap<SlotKey, HashSet<u32>>,
    /// Occurrences the plan's creates are to take, by scope. A row with no
    /// planned position steps past them, so it never takes a later create's
    /// slot and makes that create look applied.
    planned: HashMap<SlotKey, HashSet<u32>>,
}

impl Slots {
    /// Loads the stored slots on every account the plan writes a leg to, and
    /// the positions its creates carry.
    ///
    /// # Errors
    ///
    /// Returns [`BcError`] on query failure.
    async fn load(sources: &SourceService, actions: &[Action]) -> BcResult<Self> {
        let mut accounts: Vec<AccountId> = actions
            .iter()
            .filter_map(|action| match *action {
                Action::Edit { ref edited, .. } | Action::Create { ref edited, .. } => {
                    Some(edited.postings().iter().map(|leg| leg.account_id().clone()))
                }
                Action::Skip { .. } | Action::Delete { .. } | Action::Refuse { .. } => None,
            })
            .flatten()
            .collect();
        accounts.sort_by_key(ToString::to_string);
        accounts.dedup();
        let existing = sources.existing_legs(&accounts).await?;
        let stored: HashMap<SlotKey, Vec<(u32, TransactionId)>> = existing
            .into_iter()
            .map(|(key, rows)| {
                let owned = rows
                    .into_iter()
                    .map(|leg| {
                        let (SlotHolder::Live(id)
                        | SlotHolder::Tombstone(id)
                        | SlotHolder::Rejected(id)) = leg.holder;
                        (leg.occurrence, id)
                    })
                    .collect();
                (key, owned)
            })
            .collect();
        let held = stored
            .iter()
            .map(|(key, rows)| (key.clone(), rows.iter().map(|&(at, _)| at).collect()))
            .collect();
        let mut planned: HashMap<SlotKey, HashSet<u32>> = HashMap::new();
        for action in actions {
            if let Action::Create {
                ref edited,
                ref positions,
                ..
            } = *action
            {
                for (leg, &position) in edited.postings().iter().zip(positions) {
                    planned
                        .entry(slot_key(edited, leg))
                        .or_default()
                        .insert(position);
                }
            }
        }
        Ok(Self {
            stored,
            held,
            planned,
        })
    }

    /// Gives each create in `actions` its legs' positions among identical
    /// legs of the file's creates, in file order, and turns a create whose
    /// every leg an earlier apply stored at those positions into a skip.
    ///
    /// Positions count over the slots that no transaction `sidecar` lists
    /// under a handle in `actions` holds. A create stays handle-less across
    /// a rerun of its file and so keeps its position; a transaction the file
    /// names, as one an earlier apply created and a re-export since gave a
    /// handle, keeps its slots out of the count.
    fn settle(&self, actions: &mut [Action], sidecar: &Sidecar) {
        let named: HashSet<&TransactionId> = actions
            .iter()
            .filter_map(|action| match *action {
                Action::Skip {
                    handle: Some(ref handle),
                    ..
                }
                | Action::Refuse {
                    handle: Some(ref handle),
                    ..
                }
                | Action::Edit { ref handle, .. }
                | Action::Delete { ref handle, .. } => sidecar.entry(handle),
                Action::Skip { handle: None, .. }
                | Action::Refuse { handle: None, .. }
                | Action::Create { .. } => None,
            })
            .map(|entry| entry.transaction.id())
            .collect();
        let claimed: HashMap<&SlotKey, HashSet<u32>> = self
            .stored
            .iter()
            .map(|(key, legs)| {
                let theirs = legs
                    .iter()
                    .filter(|&(_, id)| named.contains(id))
                    .map(|&(at, _)| at)
                    .collect();
                (key, theirs)
            })
            .collect();
        let mut next: HashMap<SlotKey, u32> = HashMap::new();
        for action in actions.iter_mut() {
            let Action::Create {
                ref edited,
                ref mut positions,
                ..
            } = *action
            else {
                continue;
            };
            let mut held = Vec::with_capacity(edited.postings().len());
            let mut at = Vec::with_capacity(edited.postings().len());
            for leg in edited.postings() {
                let key = slot_key(edited, leg);
                let taken = claimed.get(&key);
                let cursor = next.entry(key.clone()).or_insert(0);
                let mut position = *cursor;
                while taken.is_some_and(|slots| slots.contains(&position)) {
                    position = position.saturating_add(1);
                }
                *cursor = position.saturating_add(1);
                held.push(
                    self.stored
                        .get(&key)
                        .is_some_and(|legs| legs.iter().any(|&(slot, _)| slot == position)),
                );
                at.push(position);
            }
            if stored_whole(&held) {
                *action = Action::Skip {
                    handle: None,
                    reason: SkipReason::AlreadyApplied,
                };
            } else {
                *positions = at;
            }
        }
    }

    /// Claims an occurrence for `leg` of `tx`: `preferred` when it is free,
    /// otherwise the slot after every one held or planned.
    fn claim(&mut self, tx: &Transaction, leg: &Posting, preferred: Option<u32>) -> u32 {
        let key = slot_key(tx, leg);
        let planned = self.planned.get(&key);
        let held = self.held.entry(key).or_default();
        let at = match preferred {
            Some(position) if !held.contains(&position) => position,
            Some(_) | None => held
                .iter()
                .chain(planned.into_iter().flatten())
                .max()
                .map_or(0, |&last| last.saturating_add(1)),
        };
        held.insert(at);
        at
    }
}

// MARK: Loading

/// Every posting of `ids` that carries a live source reference other than
/// apply's own.
///
/// Apply's legs are drafts whose source rows exist only to dedupe a rerun,
/// so a later apply may re-categorise or remove them. A posting with any
/// other live source stays protected.
///
/// # Errors
///
/// Returns [`BcError`] on query failure or an unparsable stored ID.
async fn protected_postings(
    pool: &SqlitePool,
    ids: &[&TransactionId],
) -> BcResult<HashSet<PostingId>> {
    let ids_json = serde_json::to_string(&ids.iter().map(ToString::to_string).collect::<Vec<_>>())?;
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT posting_id FROM transaction_sources \
         WHERE posting_id IS NOT NULL \
           AND reference IS NOT ? \
           AND transaction_id IN (SELECT value FROM json_each(?))",
    )
    .bind(APPLY)
    .bind(ids_json)
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|raw| {
            raw.parse::<PostingId>()
                .map_err(|e| BcError::BadData(e.to_string()))
        })
        .collect()
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_models::AccountId;
    use bc_models::Amount;
    use bc_models::Posting;
    use bc_models::PostingId;
    use bc_models::Reconciliation;
    use bc_models::Transaction;
    use bc_models::TransactionId;
    use jiff::Timestamp;
    use jiff::civil::Date;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;
    use serde_json::Value;
    use sqlx::SqlitePool;

    use super::*;
    use crate::AccountPath;
    use crate::AccountService;
    use crate::CommodityService;
    use crate::Directive;
    use crate::ImportBatchService;
    use crate::NativeTransaction;
    use crate::PathSpec;
    use crate::RawPosting;
    use crate::RawTransaction;
    use crate::SourceService;
    use crate::TagService;
    use crate::TransactionService;
    use crate::execute_import;
    use crate::export::Level;
    use crate::search::TransactionQuery;

    // MARK: Fixtures

    /// The accounts every test books against.
    struct Ledger {
        bank: AccountId,
        food: AccountId,
        fees: AccountId,
    }

    async fn account(pool: &SqlitePool, path: &str) -> AccountId {
        AccountService::new(pool.clone())
            .create_path(
                &PathSpec::builder()
                    .path(AccountPath::parse(path).expect("path"))
                    .build(),
            )
            .await
            .expect("account")
    }

    async fn ledger(pool: &SqlitePool) -> Ledger {
        CommodityService::new(pool.clone())
            .seed_defaults()
            .await
            .expect("seed commodities");
        Ledger {
            bank: account(pool, "Assets:Bank").await,
            food: account(pool, "Expenses:Food").await,
            fees: account(pool, "Expenses:Fees").await,
        }
    }

    fn day() -> Date {
        date(2026, 3, 4)
    }

    fn aud(value: Decimal) -> Amount {
        Amount::new(value, "AUD")
    }

    fn leg(account: &AccountId, value: Decimal) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(account.clone())
            .amount(aud(value))
            .build()
    }

    /// Stores a 50 grocery purchase from the bank.
    async fn purchase(pool: &SqlitePool, l: &Ledger) -> TransactionId {
        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date(day())
            .description("Woolworths")
            .postings(vec![leg(&l.bank, dec!(-50.00)), leg(&l.food, dec!(50.00))])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        TransactionService::new(pool.clone())
            .create(tx)
            .await
            .expect("create")
            .value
    }

    async fn sidecar(pool: &SqlitePool) -> Sidecar {
        RoundTripService::new(pool.clone())
            .export(&TransactionQuery::default(), Level::Working)
            .await
            .expect("export")
            .sidecar
            .expect("a working export has a sidecar")
    }

    fn handle(sidecar: &Sidecar, id: &TransactionId) -> String {
        sidecar
            .entries
            .iter()
            .find(|entry| entry.transaction.id() == id)
            .map(|entry| entry.handle.clone())
            .expect("exported")
    }

    /// A file transaction on [`day`].
    fn raw(handle: Option<&str>, description: &str, legs: &[(&str, Decimal)]) -> RawTransaction {
        RawTransaction::builder()
            .date(day())
            .description(description)
            .postings(
                legs.iter()
                    .map(|&(account, value)| {
                        RawPosting::builder()
                            .account(account)
                            .amount(aud(value))
                            .build()
                    })
                    .collect(),
            )
            .native(
                NativeTransaction::builder()
                    .maybe_handle(handle.map(str::to_owned))
                    .build(),
            )
            .build()
    }

    fn output(raws: Vec<RawTransaction>) -> ImportOutput {
        ImportOutput::from(
            raws.into_iter()
                .map(Directive::Transaction)
                .collect::<Vec<_>>(),
        )
    }

    async fn run(pool: &SqlitePool, raws: Vec<RawTransaction>, sidecar: &Sidecar) -> ApplyOutcome {
        let svc = RoundTripService::new(pool.clone());
        let plan = svc
            .plan_file(&output(raws), sidecar)
            .await
            .expect("plans")
            .expect("passes pre-flight");
        svc.apply(plan).await.expect("applies")
    }

    async fn find(pool: &SqlitePool, id: &TransactionId) -> Transaction {
        TransactionService::new(pool.clone())
            .find_by_id(id)
            .await
            .expect("stored")
    }

    async fn count(pool: &SqlitePool, table: &str) -> i64 {
        sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT COUNT(*) FROM {table}")))
            .fetch_one(pool)
            .await
            .expect("count")
    }

    /// A `transaction_sources` row: posting, reference, batch, owns, occurrence.
    type SourceRow = (Option<String>, Option<String>, Option<String>, i64, i64);

    async fn sources_of(pool: &SqlitePool, id: &TransactionId) -> Vec<SourceRow> {
        sqlx::query_as(
            "SELECT posting_id, reference, import_batch_id, owns_posting, occurrence \
             FROM transaction_sources WHERE transaction_id = ? ORDER BY posting_id",
        )
        .bind(id.to_string())
        .fetch_all(pool)
        .await
        .expect("sources")
    }

    /// An `import_batch_edits` row: before, after and links, as JSON.
    async fn edit_record(
        pool: &SqlitePool,
        batch: &ImportBatchId,
        id: &TransactionId,
    ) -> (Value, Option<Value>, Value) {
        let (before, after, links): (String, Option<String>, String) = sqlx::query_as(
            "SELECT before, after, source_links FROM import_batch_edits \
             WHERE import_batch_id = ? AND transaction_id = ?",
        )
        .bind(batch.to_string())
        .bind(id.to_string())
        .fetch_one(pool)
        .await
        .expect("edit recorded");
        let parse = |text: &str| serde_json::from_str::<Value>(text).expect("json");
        (parse(&before), after.as_deref().map(parse), parse(&links))
    }

    fn json(tx: &Transaction) -> Value {
        serde_json::to_value(tx).expect("encode")
    }

    fn posting_ids(tx: &Transaction) -> Vec<PostingId> {
        tx.postings().iter().map(|p| p.id().clone()).collect()
    }

    /// The edit every edit test makes: food down to 45, a 5 fee added.
    fn split_fee(handle: &str) -> RawTransaction {
        raw(
            Some(handle),
            "Woolworths",
            &[
                ("Assets:Bank", dec!(-50.00)),
                ("Expenses:Food", dec!(45.00)),
                ("Expenses:Fees", dec!(5.00)),
            ],
        )
    }

    fn coles() -> RawTransaction {
        raw(
            None,
            "Coles",
            &[
                ("Assets:Bank", dec!(-12.00)),
                ("Expenses:Food", dec!(12.00)),
            ],
        )
    }

    /// Imports a 50 grocery purchase as a bank statement would, with a
    /// source on each leg, and returns it.
    async fn import_woolworths(pool: &SqlitePool) -> Transaction {
        let statement_row = RawTransaction::builder()
            .date(day())
            .description("Woolworths")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .amount(aud(dec!(-50.00)))
                    .build(),
                RawPosting::builder()
                    .account("Expenses:Food")
                    .amount(aud(dec!(50.00)))
                    .build(),
            ])
            .build();
        execute_import(
            &TransactionService::new(pool.clone()),
            &SourceService::new(pool.clone()),
            &AccountService::new(pool.clone()),
            &CommodityService::new(pool.clone()),
            &TagService::new(pool.clone()),
            &ImportBatchService::new(pool.clone()),
            None,
            "csv",
            &[],
            &[statement_row],
        )
        .await
        .expect("import");
        let all = TransactionService::new(pool.clone())
            .list()
            .await
            .expect("list");
        all.first().cloned().expect("imported")
    }

    // MARK: Edit

    #[sqlx::test(migrations = "./migrations")]
    async fn apply_edits_flags_and_records_before_after(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let id = purchase(&pool, &l).await;
        let original = find(&pool, &id).await;
        let sidecar = sidecar(&pool).await;

        let outcome = run(&pool, vec![split_fee(&handle(&sidecar, &id))], &sidecar).await;

        assert_eq!(outcome.edited, 1);
        assert_eq!(outcome.refused, vec![]);
        let batch = outcome.batch_id.clone().expect("a batch");
        let stored = find(&pool, &id).await;
        assert_eq!(stored.reconciliation(), Reconciliation::Flagged);
        let ids = posting_ids(&stored);
        assert_eq!(ids.get(..2), Some(posting_ids(&original).as_slice()));
        let fee = stored.postings().get(2).expect("fee leg");
        assert_eq!(fee.account_id(), &l.fees);

        let (before, after, links) = edit_record(&pool, &batch, &id).await;
        assert_eq!(before, json(&original));
        assert_eq!(after, Some(json(&stored)));
        assert_eq!(links, serde_json::json!([]));

        assert_eq!(
            sources_of(&pool, &id).await,
            vec![(
                Some(fee.id().to_string()),
                Some("bc-apply".to_owned()),
                Some(batch.to_string()),
                1,
                0,
            )]
        );
        let closed = ImportBatchService::new(pool.clone())
            .find_by_id(&batch)
            .await
            .expect("batch");
        assert_eq!(closed.importer, "bc-apply");
        let counts = closed.counts.expect("closed");
        assert_eq!(counts.new_transactions, 0);
        assert_eq!(counts.attached_postings, 1);
    }

    // MARK: Create

    #[sqlx::test(migrations = "./migrations")]
    async fn apply_creates_with_bc_apply_sources(pool: SqlitePool) {
        ledger(&pool).await;
        let sidecar = sidecar(&pool).await;

        let outcome = run(&pool, vec![coles()], &sidecar).await;

        assert_eq!(outcome.created, 1);
        let batch = outcome.batch_id.clone().expect("a batch");
        let all = TransactionService::new(pool.clone())
            .list()
            .await
            .expect("list");
        let [created] = all.as_slice() else {
            panic!("one transaction: {all:#?}");
        };
        assert_eq!(created.description(), "Coles");
        assert_eq!(created.reconciliation(), Reconciliation::Flagged);
        let mut expected: Vec<SourceRow> = created
            .postings()
            .iter()
            .map(|p| {
                (
                    Some(p.id().to_string()),
                    Some("bc-apply".to_owned()),
                    Some(batch.to_string()),
                    1,
                    0,
                )
            })
            .collect();
        expected.sort();
        assert_eq!(sources_of(&pool, created.id()).await, expected);
        let counts = ImportBatchService::new(pool.clone())
            .find_by_id(&batch)
            .await
            .expect("batch")
            .counts
            .expect("closed");
        assert_eq!(counts.new_transactions, 1);
        assert_eq!(count(&pool, "import_batch_edits").await, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rerun_creates_nothing_new(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let id = purchase(&pool, &l).await;
        let sidecar = sidecar(&pool).await;
        let h = handle(&sidecar, &id);

        let first = run(&pool, vec![split_fee(&h), coles(), coles()], &sidecar).await;
        assert_eq!((first.edited, first.created), (1, 2));
        let transactions = count(&pool, "transactions").await;
        let sources = count(&pool, "transaction_sources").await;

        let second = run(&pool, vec![split_fee(&h), coles(), coles()], &sidecar).await;

        assert_eq!(
            second,
            ApplyOutcome {
                skipped: 3,
                ..ApplyOutcome::default()
            }
        );
        assert_eq!(count(&pool, "transactions").await, transactions);
        assert_eq!(count(&pool, "transaction_sources").await, sources);
        assert_eq!(count(&pool, "import_batches").await, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rerun_plans_stored_creates_as_skips(pool: SqlitePool) {
        ledger(&pool).await;
        let sidecar = sidecar(&pool).await;
        run(&pool, vec![coles()], &sidecar).await;

        let plan = RoundTripService::new(pool.clone())
            .plan_file(&output(vec![coles(), coles()]), &sidecar)
            .await
            .expect("plans")
            .expect("passes pre-flight");

        let [first, second] = plan.actions.as_slice() else {
            panic!("expected two actions, got {:#?}", plan.actions);
        };
        assert!(
            matches!(
                *first,
                Action::Skip {
                    handle: None,
                    reason: SkipReason::AlreadyApplied
                }
            ),
            "{first:#?}"
        );
        let Action::Create { ref positions, .. } = *second else {
            panic!("expected a create, got {second:#?}");
        };
        assert_eq!(positions, &vec![1, 1]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unwritable_create_is_refused_at_its_file_position(pool: SqlitePool) {
        ledger(&pool).await;
        let sidecar = sidecar(&pool).await;
        let elided = |account: &str| RawPosting::builder().account(account).build();
        let unwritable = RawTransaction::builder()
            .date(day())
            .description("Two elided legs")
            .postings(vec![elided("Assets:Bank"), elided("Expenses:Food")])
            .build();

        let outcome = run(&pool, vec![coles(), unwritable], &sidecar).await;

        assert_eq!(outcome.created, 1);
        let [ref refusal] = *outcome.refused.as_slice() else {
            panic!("expected one refusal, got {:#?}", outcome.refused);
        };
        assert_eq!((refusal.index, refusal.handle.as_deref()), (1, None));
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn applying_one_plan_twice_creates_once(pool: SqlitePool) {
        ledger(&pool).await;
        let sidecar = sidecar(&pool).await;
        let svc = RoundTripService::new(pool.clone());
        let plan = svc
            .plan_file(&output(vec![coles(), coles()]), &sidecar)
            .await
            .expect("plans")
            .expect("passes pre-flight");

        let first = svc.apply(plan.clone()).await.expect("applies");
        let transactions = count(&pool, "transactions").await;
        let sources = count(&pool, "transaction_sources").await;
        let second = svc.apply(plan).await.expect("applies again");

        assert_eq!((first.created, first.skipped), (2, 0));
        assert_eq!((second.created, second.skipped), (0, 2));
        assert_eq!(second.refused, vec![]);
        assert_eq!(count(&pool, "transactions").await, transactions);
        assert_eq!(count(&pool, "transaction_sources").await, sources);
    }

    /// Two plans of one file made before either applies, as two writers
    /// would make them: the second finds the first's creates stored.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plan_overtaken_by_another_apply_creates_nothing(pool: SqlitePool) {
        ledger(&pool).await;
        let sidecar = sidecar(&pool).await;
        let svc = RoundTripService::new(pool.clone());
        let plan = || async {
            svc.plan_file(&output(vec![coles(), coles()]), &sidecar)
                .await
                .expect("plans")
                .expect("passes pre-flight")
        };
        let (first_plan, second_plan) = (plan().await, plan().await);

        let first = svc.apply(first_plan).await.expect("applies");
        let transactions = count(&pool, "transactions").await;
        let sources = count(&pool, "transaction_sources").await;
        let second = svc.apply(second_plan).await.expect("applies again");

        assert_eq!((first.created, first.skipped), (2, 0));
        assert_eq!((second.created, second.skipped), (0, 2));
        assert_eq!(second.refused, vec![]);
        assert_eq!(count(&pool, "transactions").await, transactions);
        assert_eq!(count(&pool, "transaction_sources").await, sources);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn freed_slot_is_refilled_once(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let empty = sidecar(&pool).await;
        assert_eq!(run(&pool, vec![coles(), coles()], &empty).await.created, 2);
        let (first,): (String,) = sqlx::query_as(
            "SELECT transaction_id FROM transaction_sources \
             WHERE account_id = ? AND occurrence = 0",
        )
        .bind(l.bank.to_string())
        .fetch_one(&pool)
        .await
        .expect("slot 0");
        TransactionService::new(pool.clone())
            .delete(
                &first.parse().expect("transaction id"),
                DeleteMode::ForgetProvenance,
            )
            .await
            .expect("delete, freeing slot 0");
        assert_eq!(count(&pool, "transactions").await, 1);

        let refill = run(&pool, vec![coles(), coles()], &empty).await;
        let rerun = run(&pool, vec![coles(), coles()], &empty).await;

        assert_eq!((refill.created, refill.skipped), (1, 1));
        assert_eq!((rerun.created, rerun.skipped), (0, 2));
        assert_eq!(count(&pool, "transactions").await, 2);
        let mut occurrences: Vec<i64> =
            sqlx::query_scalar("SELECT occurrence FROM transaction_sources WHERE account_id = ?")
                .bind(l.bank.to_string())
                .fetch_all(&pool)
                .await
                .expect("occurrences");
        occurrences.sort_unstable();
        assert_eq!(occurrences, vec![0, 1]);
    }

    /// A 4.50 coffee from the bank, as a file line under `handle`.
    fn coffee(handle: Option<&str>) -> RawTransaction {
        raw(
            handle,
            "Cafe",
            &[("Assets:Bank", dec!(-4.50)), ("Expenses:Food", dec!(4.50))],
        )
    }

    /// The one stored transaction other than `known`.
    async fn other_than(pool: &SqlitePool, known: &TransactionId) -> TransactionId {
        let all = TransactionService::new(pool.clone())
            .list()
            .await
            .expect("list");
        let [ref first, ref second] = *all.as_slice() else {
            panic!("two transactions: {all:#?}");
        };
        if first.id() == known {
            second.id().clone()
        } else {
            first.id().clone()
        }
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_re_exported_create_leaves_a_new_twin_to_be_created(pool: SqlitePool) {
        ledger(&pool).await;
        let empty = sidecar(&pool).await;
        assert_eq!(run(&pool, vec![coffee(None)], &empty).await.created, 1);
        let all = TransactionService::new(pool.clone())
            .list()
            .await
            .expect("list");
        let first = all.first().map(|tx| tx.id().clone()).expect("created");
        let exported = sidecar(&pool).await;
        let file = || vec![coffee(Some(&handle(&exported, &first))), coffee(None)];

        let twin = run(&pool, file(), &exported).await;
        let rerun = run(&pool, file(), &exported).await;
        let again = run(&pool, file(), &exported).await;

        assert_eq!((twin.created, twin.skipped), (1, 1));
        assert_eq!((rerun.created, rerun.skipped), (0, 2));
        assert_eq!((again.created, again.skipped), (0, 2));
        assert_eq!(count(&pool, "transactions").await, 2);
        let second = other_than(&pool, &first).await;
        let occurrences =
            |rows: Vec<SourceRow>| -> Vec<i64> { rows.into_iter().map(|(.., at)| at).collect() };
        assert_eq!(occurrences(sources_of(&pool, &first).await), vec![0, 0]);
        assert_eq!(occurrences(sources_of(&pool, &second).await), vec![1, 1]);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unexported_rerun_still_creates_once(pool: SqlitePool) {
        ledger(&pool).await;
        let empty = sidecar(&pool).await;

        let first = run(&pool, vec![coffee(None), coffee(None)], &empty).await;
        let second = run(&pool, vec![coffee(None), coffee(None)], &empty).await;
        let third = run(&pool, vec![coffee(None), coffee(None)], &empty).await;

        assert_eq!((first.created, first.skipped), (2, 0));
        assert_eq!((second.created, second.skipped), (0, 2));
        assert_eq!((third.created, third.skipped), (0, 2));
        assert_eq!(count(&pool, "transactions").await, 2);
    }

    /// An edit adding legs identical to a later create's must not take the
    /// slots the plan gave that create, or the write-time check would find
    /// them stored and skip a create the plan reported.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_edit_leaves_a_planned_create_its_slots(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let stored = Transaction::builder()
            .id(TransactionId::new())
            .date(day())
            .description("Cafe")
            .postings(vec![leg(&l.bank, dec!(-4.50)), leg(&l.food, dec!(4.50))])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();
        let id = TransactionService::new(pool.clone())
            .create(stored)
            .await
            .expect("create")
            .value;
        let exported = sidecar(&pool).await;
        let doubled = raw(
            Some(&handle(&exported, &id)),
            "Cafe",
            &[
                ("Assets:Bank", dec!(-4.50)),
                ("Expenses:Food", dec!(4.50)),
                ("Assets:Bank", dec!(-4.50)),
                ("Expenses:Food", dec!(4.50)),
            ],
        );
        let file = || vec![doubled.clone(), coffee(None)];

        let svc = RoundTripService::new(pool.clone());
        let plan = svc
            .plan_file(&output(file()), &exported)
            .await
            .expect("plans")
            .expect("passes pre-flight");
        assert!(
            matches!(
                plan.actions.as_slice(),
                [Action::Edit { .. }, Action::Create { .. }]
            ),
            "{plan:#?}"
        );
        let first = svc.apply(plan).await.expect("applies");
        let rerun = run(&pool, file(), &exported).await;

        assert_eq!((first.edited, first.created, first.skipped), (1, 1, 0));
        assert_eq!((rerun.created, rerun.skipped), (0, 2));
        assert_eq!(count(&pool, "transactions").await, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rerun_after_partial_refuses_only_the_stale_one(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let fresh = purchase(&pool, &l).await;
        let stale = purchase(&pool, &l).await;
        let sidecar = sidecar(&pool).await;
        let (fresh_h, stale_h) = (handle(&sidecar, &fresh), handle(&sidecar, &stale));
        let raws = || vec![split_fee(&fresh_h), split_fee(&stale_h)];

        let svc = RoundTripService::new(pool.clone());
        let plan = svc
            .plan_file(&output(raws()), &sidecar)
            .await
            .expect("plans")
            .expect("passes pre-flight");
        let before_ui = find(&pool, &stale).await;
        let ui_edit = Transaction::builder()
            .id(stale.clone())
            .date(day())
            .description("Woolworths")
            .postings(vec![
                before_ui.postings().first().cloned().expect("bank leg"),
                leg(&l.food, dec!(49.00)),
            ])
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(*before_ui.created_at())
            .build();
        TransactionService::new(pool.clone())
            .edit(ui_edit)
            .await
            .expect("edit in the UI");
        let after_ui = find(&pool, &stale).await;

        let first = svc.apply(plan).await.expect("applies");
        assert_eq!(first.edited, 1);
        assert_eq!(
            first.refused,
            vec![Refusal::new(
                1,
                Some(stale_h.clone()),
                "changed since export"
            )]
        );
        assert_eq!(json(&find(&pool, &stale).await), json(&after_ui));
        assert_eq!(
            find(&pool, &fresh).await.reconciliation(),
            Reconciliation::Flagged
        );

        let second = run(&pool, raws(), &sidecar).await;
        assert_eq!(
            second,
            ApplyOutcome {
                skipped: 1,
                refused: vec![Refusal::new(1, Some(stale_h), "changed since export")],
                ..ApplyOutcome::default()
            }
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn bank_import_does_not_dedupe_against_apply_leg(pool: SqlitePool) {
        ledger(&pool).await;
        let sidecar = sidecar(&pool).await;
        let applied = run(&pool, vec![coles()], &sidecar).await;
        assert_eq!(applied.created, 1);

        let statement_row = RawTransaction::builder()
            .date(day())
            .description("Coles")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .amount(aud(dec!(-12.00)))
                    .build(),
                RawPosting::builder()
                    .account("Expenses:Food")
                    .amount(aud(dec!(12.00)))
                    .build(),
            ])
            .build();
        let imported = execute_import(
            &TransactionService::new(pool.clone()),
            &SourceService::new(pool.clone()),
            &AccountService::new(pool.clone()),
            &CommodityService::new(pool.clone()),
            &TagService::new(pool.clone()),
            &ImportBatchService::new(pool.clone()),
            None,
            "csv",
            &[],
            &[statement_row],
        )
        .await
        .expect("import");

        assert_eq!(imported.new_transactions, 1);
        assert_eq!(count(&pool, "transactions").await, 2);
    }

    // MARK: Protection

    #[sqlx::test(migrations = "./migrations")]
    async fn apply_leg_is_not_protected(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let empty = sidecar(&pool).await;
        assert_eq!(run(&pool, vec![coles()], &empty).await.created, 1);
        let all = TransactionService::new(pool.clone())
            .list()
            .await
            .expect("list");
        let created = all.first().cloned().expect("created");
        let draft = created.postings().get(1).cloned().expect("food leg");
        assert_eq!(draft.account_id(), &l.food);
        let (source_id,): (String,) =
            sqlx::query_as("SELECT id FROM transaction_sources WHERE posting_id = ?")
                .bind(draft.id().to_string())
                .fetch_one(&pool)
                .await
                .expect("draft source");
        let sidecar = sidecar(&pool).await;
        let recategorised = raw(
            Some(&handle(&sidecar, created.id())),
            "Coles",
            &[
                ("Assets:Bank", dec!(-12.00)),
                ("Expenses:Fees", dec!(12.00)),
            ],
        );

        let svc = RoundTripService::new(pool.clone());
        let plan = svc
            .plan_file(&output(vec![recategorised]), &sidecar)
            .await
            .expect("plans")
            .expect("passes pre-flight");
        assert!(
            matches!(plan.actions.as_slice(), [Action::Edit { .. }]),
            "{plan:#?}"
        );
        let outcome = svc.apply(plan).await.expect("applies");

        assert_eq!(outcome.edited, 1);
        let stored = find(&pool, created.id()).await;
        assert_eq!(posting_ids(&stored), posting_ids(&created));
        assert_eq!(
            stored.postings().get(1).map(Posting::account_id),
            Some(&l.fees)
        );
        let (posting, account): (Option<String>, String) =
            sqlx::query_as("SELECT posting_id, account_id FROM transaction_sources WHERE id = ?")
                .bind(&source_id)
                .fetch_one(&pool)
                .await
                .expect("the draft source survives");
        assert_eq!(
            (posting, account),
            (Some(draft.id().to_string()), l.food.to_string()),
            "the re-categorised leg keeps its posting, so its source stays live"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn statement_leg_stays_protected(pool: SqlitePool) {
        ledger(&pool).await;
        let original = import_woolworths(&pool).await;
        let sidecar = sidecar(&pool).await;
        let changed = raw(
            Some(&handle(&sidecar, original.id())),
            "Woolworths",
            &[
                ("Assets:Bank", dec!(-55.00)),
                ("Expenses:Food", dec!(55.00)),
            ],
        );

        let plan = RoundTripService::new(pool.clone())
            .plan_file(&output(vec![changed]), &sidecar)
            .await
            .expect("plans")
            .expect("passes pre-flight");

        let [Action::Refuse { reason, .. }] = plan.actions.as_slice() else {
            panic!("expected a refusal: {plan:#?}");
        };
        assert_eq!(
            reason,
            "statement leg changed or removed: Assets:Bank -50.00 AUD"
        );
    }

    // MARK: Delete

    #[sqlx::test(migrations = "./migrations")]
    async fn delete_keeps_provenance_and_records_links(pool: SqlitePool) {
        ledger(&pool).await;
        let original = import_woolworths(&pool).await;
        let id = original.id().clone();
        let mut links: Vec<(String, String)> = sqlx::query_as(
            "SELECT id, posting_id FROM transaction_sources WHERE transaction_id = ?",
        )
        .bind(id.to_string())
        .fetch_all(&pool)
        .await
        .expect("links");
        links.sort();
        assert_eq!(links.len(), 2);
        let sidecar = sidecar(&pool).await;
        let mut marked = raw(
            Some(&handle(&sidecar, &id)),
            "Woolworths",
            &[
                ("Assets:Bank", dec!(-50.00)),
                ("Expenses:Food", dec!(50.00)),
            ],
        );
        if let Some(native) = marked.native.as_mut() {
            native.delete = true;
        }

        let outcome = run(&pool, vec![marked], &sidecar).await;

        assert_eq!(outcome.deleted, 1);
        let batch = outcome.batch_id.clone().expect("a batch");
        assert!(matches!(
            TransactionService::new(pool.clone()).find_by_id(&id).await,
            Err(BcError::NotFound(_))
        ));
        let kept: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transaction_sources \
             WHERE deleted_transaction_id = ? AND transaction_id IS NULL",
        )
        .bind(id.to_string())
        .fetch_one(&pool)
        .await
        .expect("orphans");
        assert_eq!(kept, 2);
        let (before, after, raw_links) = edit_record(&pool, &batch, &id).await;
        assert_eq!(before, json(&original));
        assert_eq!(after, None);
        let mut recorded: Vec<(String, String)> = serde_json::from_value(raw_links).expect("pairs");
        recorded.sort();
        assert_eq!(recorded, links);
    }

    // MARK: Whole file

    #[sqlx::test(migrations = "./migrations")]
    async fn wrong_database_refused(pool: SqlitePool) {
        ledger(&pool).await;
        let mut sidecar = sidecar(&pool).await;
        sidecar.database_id = "0123456789abcdef0123456789abcdef".to_owned();

        let refused = RoundTripService::new(pool.clone())
            .plan_file(&output(vec![coles()]), &sidecar)
            .await
            .expect("plans")
            .expect_err("refused");

        assert_eq!(
            refused.problems,
            vec!["the sidecar was written from another database".to_owned()]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unsupported_sidecar_version_refused(pool: SqlitePool) {
        ledger(&pool).await;
        let mut sidecar = sidecar(&pool).await;
        sidecar.version = 2;

        let refused = RoundTripService::new(pool.clone())
            .plan_file(&output(vec![coles()]), &sidecar)
            .await
            .expect("plans")
            .expect_err("refused");

        assert_eq!(
            refused.problems,
            vec!["sidecar version 2 is not supported; this build reads version 1".to_owned()]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn no_op_opens_no_batch(pool: SqlitePool) {
        let l = ledger(&pool).await;
        let id = purchase(&pool, &l).await;
        let sidecar = sidecar(&pool).await;
        let unchanged = raw(
            Some(&handle(&sidecar, &id)),
            "Woolworths",
            &[
                ("Assets:Bank", dec!(-50.00)),
                ("Expenses:Food", dec!(50.00)),
            ],
        );

        let outcome = run(&pool, vec![unchanged], &sidecar).await;

        assert_eq!(
            outcome,
            ApplyOutcome {
                skipped: 1,
                ..ApplyOutcome::default()
            }
        );
        assert_eq!(count(&pool, "import_batches").await, 0);
        assert_eq!(
            find(&pool, &id).await.reconciliation(),
            Reconciliation::Unreconciled
        );
    }
}
