//! Import execution: resolve every raw leg's account path, then create
//! transactions and attach per-leg provenance for the legs that are new.
//!
//! The run is a pipeline of six steps, each its own function below:
//!
//! 1. **Resolve** every leg's account path against one snapshot of the account
//!    tree ([`resolve_legs`]).
//! 2. **Validate** each raw transaction's structure — two or more elided legs
//!    leave the residual ambiguous ([`has_ambiguous_residual`]).
//! 3. **Allocate** an occurrence slot per `(account, fingerprint)` across every
//!    leg of the run ([`allocate_occurrences`]).
//! 4. **Match** each planned leg against the legs already stored
//!    ([`Writer::owner_of`]).
//! 5. **Corroborate** a matched candidate — every posting already on it must be
//!    explained by a leg of this document transaction ([`corroborates`]).
//! 6. **Decide and write** per transaction — create, attach, or skip
//!    ([`Writer::write_row`]).

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Balances;
use bc_models::ImportBatchId;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;

use crate::AccountPath;
use crate::AccountResolver;
use crate::BcResult;
use crate::RawPosting;
use crate::RawTransaction;
use crate::Resolution;
use crate::StoredLeg;

/// What an import run did.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportOutcome {
    /// The batch recording this run.
    pub batch_id: ImportBatchId,
    /// Transactions created.
    pub new_transactions: usize,
    /// Postings attached to transactions an earlier run created.
    pub attached_postings: usize,
    /// Postings that could not be persisted, whatever the cause. The sum of
    /// [`Self::unresolved_path_postings`] and [`Self::other_skipped_postings`].
    pub skipped_postings: usize,
    /// Postings skipped because their account path named no existing account.
    ///
    /// These are the legs [`Self::unresolved_paths`] accounts for; creating
    /// those accounts and re-running attaches them.
    pub unresolved_path_postings: usize,
    /// Postings skipped for any other reason — a malformed account path, an
    /// ambiguous residual, legs owned by several transactions, or a candidate
    /// that failed to corroborate. Each was warned about individually.
    pub other_skipped_postings: usize,
    /// Account paths that resolved to no account, deduplicated and sorted.
    ///
    /// This is the actionable output: create these accounts and re-run, and the
    /// next pass attaches the legs this one skipped.
    pub unresolved_paths: Vec<String>,
}

/// Running totals for one import run, with skips attributed to their cause.
#[derive(Debug, Default)]
struct Counts {
    /// Transactions created so far.
    new_transactions: usize,
    /// Postings appended to already-existing transactions so far.
    attached_postings: usize,
    /// Postings skipped because their account path named no existing account.
    unresolved_path_postings: usize,
    /// Postings skipped for any other reason.
    other_skipped_postings: usize,
}

impl Counts {
    /// Records `postings` as unpersistable for a reason other than an
    /// unresolved account path.
    fn skip_other(&mut self, postings: usize) {
        self.other_skipped_postings = self.other_skipped_postings.saturating_add(postings);
    }

    /// Returns the total skipped, whatever the cause.
    fn skipped(&self) -> usize {
        self.unresolved_path_postings
            .saturating_add(self.other_skipped_postings)
    }
}

/// Why one leg — or one whole transaction — could not be persisted this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipCause {
    /// The leg's account path named no existing account. Creating the account
    /// and re-running attaches the leg.
    UnresolvedPath,
    /// Anything else: a malformed account path, an ambiguous residual, legs
    /// owned by several transactions, or a failed corroboration.
    Other,
}

/// One leg whose account path resolved to an existing account.
#[derive(Debug, Clone)]
struct ResolvedLeg {
    /// The account the leg's path named.
    account_id: AccountId,
    /// The leg's amount as the document stated it; `None` for the elided residual.
    amount: Option<Amount>,
    /// The leg's dedup fingerprint, over the document's own values.
    fingerprint: String,
}

/// A resolved leg together with the occurrence slot it claims for this run.
#[derive(Debug, Clone)]
struct LegPlan {
    /// The account the leg's path named.
    account_id: AccountId,
    /// The leg's amount as the document stated it; `None` for the elided residual.
    amount: Option<Amount>,
    /// The leg's dedup fingerprint, over the document's own values.
    fingerprint: String,
    /// The occurrence slot this leg claims within `(account, fingerprint)`.
    occurrence: u32,
}

impl LegPlan {
    /// Builds the posting this leg persists as.
    ///
    /// # Arguments
    ///
    /// * `residual` - Amount to give an elided leg whose derived value would
    ///   otherwise be lost; `None` keeps the leg elided.
    ///
    /// # Returns
    ///
    /// A freshly-identified [`Posting`] on this leg's account.
    fn posting(&self, residual: Option<&Amount>) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(self.account_id.clone())
            .maybe_amount(self.amount.clone().or_else(|| residual.cloned()))
            .build()
    }
}

/// The outcome of the resolution pass over every leg of every raw transaction.
struct Resolved {
    /// Resolved legs per raw transaction, index-aligned with the input slice.
    ///
    /// An entry is shorter than its raw transaction's posting list when some leg
    /// was skipped, and empty when the whole transaction was.
    rows: Vec<Vec<ResolvedLeg>>,
    /// Legs skipped because their account path named no existing account.
    unresolved_path_postings: usize,
    /// Legs skipped for any other reason.
    other_skipped_postings: usize,
    /// Distinct account paths naming no account; sorted and unique by construction.
    unresolved_paths: BTreeSet<String>,
}

/// Imports raw transactions, persisting every resolvable posting.
///
/// Each leg's account **path** is resolved to an id; a leg naming no existing
/// account is skipped and its path reported, so the user can create the account
/// and re-run — the next pass attaches the leg to the transaction this pass
/// created. Provenance is recorded per leg, which is what makes that possible.
///
/// A transaction is skipped whole only when it is structurally unrepresentable:
/// two or more elided legs leave the residual ambiguous.
///
/// The stored legs are loaded once per run, before any write, so a leg cannot
/// attach to a transaction created earlier in the *same* batch — it defers to
/// the next run. That is what keeps dedup decisions consistent across the run.
///
/// # Arguments
///
/// * `transactions` - Transaction persistence service.
/// * `sources` - Source-reference persistence service.
/// * `accounts` - Account service, snapshotted once for path resolution.
/// * `batches` - Import batch provenance service.
/// * `profile_id` - The driving profile, if the run is profile-driven.
/// * `importer` - Stable importer name, recorded on the batch.
/// * `raws` - Parsed transactions in document order.
///
/// # Returns
///
/// An [`ImportOutcome`] summarising what was written and what was skipped.
///
/// # Errors
///
/// Returns [`crate::BcError`] on query, insert, or batch-record failure.
#[inline]
pub async fn execute_import(
    transactions: &crate::TransactionService,
    sources: &crate::SourceService,
    accounts: &crate::AccountService,
    batches: &crate::ImportBatchService,
    profile_id: Option<&bc_models::ProfileId>,
    importer: &str,
    raws: &[RawTransaction],
) -> BcResult<ImportOutcome> {
    let resolver = crate::AccountResolver::load(accounts).await?;
    let batch_id = batches.open(profile_id, importer).await?;

    let pass = resolve_legs(&resolver, raws);
    let unresolved_paths: Vec<String> = pass.unresolved_paths.into_iter().collect();
    let mut counts = Counts {
        unresolved_path_postings: pass.unresolved_path_postings,
        other_skipped_postings: pass.other_skipped_postings,
        ..Counts::default()
    };

    let planned = allocate_occurrences(pass.rows);
    // One query per touched account for the whole run, not per row.
    let writer = Writer {
        transactions,
        sources,
        existing: sources.existing_legs(&touched_accounts(&planned)).await?,
        batch_id: batch_id.clone(),
    };

    for (raw, legs) in raws.iter().zip(&planned) {
        writer.write_row(raw, legs, &mut counts).await?;
    }

    batches
        .close(
            &batch_id,
            crate::ImportBatchCounts {
                new_transactions: counts.new_transactions,
                attached_postings: counts.attached_postings,
                unresolved_path_postings: counts.unresolved_path_postings,
                other_skipped_postings: counts.other_skipped_postings,
            },
        )
        .await?;

    Ok(ImportOutcome {
        batch_id,
        new_transactions: counts.new_transactions,
        attached_postings: counts.attached_postings,
        skipped_postings: counts.skipped(),
        unresolved_path_postings: counts.unresolved_path_postings,
        other_skipped_postings: counts.other_skipped_postings,
        unresolved_paths,
    })
}

/// Step 1 and 2: resolves every leg's account path, dropping the legs — or the
/// whole transaction — that cannot be persisted this run.
///
/// # Arguments
///
/// * `resolver` - The account-tree snapshot to resolve paths against.
/// * `raws` - Parsed transactions in document order.
///
/// # Returns
///
/// The resolved legs per transaction, the skipped-posting tallies attributed to
/// their causes, and the distinct unresolved paths.
fn resolve_legs(resolver: &AccountResolver, raws: &[RawTransaction]) -> Resolved {
    let mut out = Resolved {
        rows: Vec::with_capacity(raws.len()),
        unresolved_path_postings: 0_usize,
        other_skipped_postings: 0_usize,
        unresolved_paths: BTreeSet::new(),
    };

    for raw in raws {
        if has_ambiguous_residual(raw) {
            tracing::warn!(
                location = location_of(raw),
                postings = raw.postings.len(),
                "two or more elided legs leave the residual ambiguous; skipping the transaction"
            );
            out.other_skipped_postings = out
                .other_skipped_postings
                .saturating_add(raw.postings.len());
            out.rows.push(Vec::new());
            continue;
        }

        let mut legs = Vec::with_capacity(raw.postings.len());
        for posting in &raw.postings {
            match resolve_leg(resolver, raw, posting, &mut out.unresolved_paths) {
                Ok(leg) => legs.push(leg),
                Err(SkipCause::UnresolvedPath) => {
                    out.unresolved_path_postings = out.unresolved_path_postings.saturating_add(1);
                }
                Err(SkipCause::Other) => {
                    out.other_skipped_postings = out.other_skipped_postings.saturating_add(1);
                }
            }
        }
        out.rows.push(legs);
    }

    out
}

/// Step 2: reports whether `raw` leaves its residual ambiguous.
///
/// Two or more elided legs cannot both absorb the residual, so the transaction
/// is unrepresentable rather than merely unbalanced.
///
/// # Arguments
///
/// * `raw` - The transaction to inspect.
///
/// # Returns
///
/// `true` if two or more legs elide their amount.
fn has_ambiguous_residual(raw: &RawTransaction) -> bool {
    raw.postings
        .iter()
        .filter(|posting| posting.amount.is_none())
        .count()
        >= 2_usize
}

/// Resolves one leg, reporting the diagnostics a skipped leg warrants.
///
/// # Arguments
///
/// * `resolver` - The account-tree snapshot to resolve against.
/// * `raw` - The transaction the leg belongs to, for diagnostics.
/// * `posting` - The leg to resolve.
/// * `unresolved` - Accumulator of distinct unresolved paths; also the
///   warn-once guard, since inserting a path reports whether it is new.
///
/// # Returns
///
/// The [`ResolvedLeg`], or the [`SkipCause`] that stopped it being persisted
/// this run.
fn resolve_leg(
    resolver: &AccountResolver,
    raw: &RawTransaction,
    posting: &RawPosting,
    unresolved: &mut BTreeSet<String>,
) -> Result<ResolvedLeg, SkipCause> {
    let path = match AccountPath::parse(&posting.account) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                location = location_of(raw),
                account = posting.account.as_str(),
                %error,
                "malformed account path; skipping this leg"
            );
            return Err(SkipCause::Other);
        }
    };

    let account_id = match resolver.resolve(&path) {
        Resolution::Resolved { id, archived } => {
            if archived {
                tracing::warn!(
                    location = location_of(raw),
                    account = %path,
                    "importing into an archived account"
                );
            }
            id
        }
        Resolution::Missing {
            resolved_prefix,
            missing_segment,
        } => {
            let rendered = path.to_string();
            // Warn once per distinct path: a file naming one missing account in
            // every row should log one line, not one per row.
            if unresolved.insert(rendered.clone()) {
                tracing::warn!(
                    location = location_of(raw),
                    account = rendered.as_str(),
                    resolved_prefix = resolved_prefix.as_str(),
                    missing_segment = missing_segment.as_str(),
                    "account path names no existing account; create it and re-run to \
                     attach the legs skipped now"
                );
            }
            return Err(SkipCause::UnresolvedPath);
        }
    };

    Ok(ResolvedLeg {
        fingerprint: SourceRef::compute_fingerprint(
            raw.date,
            &raw.description,
            posting.amount.as_ref(),
            raw.reference.as_deref(),
        ),
        account_id,
        amount: posting.amount.clone(),
    })
}

/// Step 3: claims an occurrence slot for every resolved leg.
///
/// Slots are allocated per `(account, fingerprint)` across **all** legs of the
/// run, not per transaction, so two genuinely identical legs on one account take
/// distinct slots. Allocation restarts at zero each run, which is what makes a
/// re-import land on the same slots it wrote last time and therefore dedup.
///
/// # Arguments
///
/// * `rows` - Resolved legs per transaction, in document order.
///
/// # Returns
///
/// The same rows with each leg's occurrence assigned.
fn allocate_occurrences(rows: Vec<Vec<ResolvedLeg>>) -> Vec<Vec<LegPlan>> {
    let mut claimed: HashMap<(String, String), u32> = HashMap::new();
    rows.into_iter()
        .map(|legs| {
            legs.into_iter()
                .map(|leg| {
                    let slot = claimed
                        .entry((leg.account_id.to_string(), leg.fingerprint.clone()))
                        .or_insert(0_u32);
                    let occurrence = *slot;
                    *slot = slot.saturating_add(1_u32);
                    LegPlan {
                        account_id: leg.account_id,
                        amount: leg.amount,
                        fingerprint: leg.fingerprint,
                        occurrence,
                    }
                })
                .collect()
        })
        .collect()
}

/// Returns every account named by a planned leg, without duplicates.
///
/// # Arguments
///
/// * `rows` - Planned legs per transaction.
///
/// # Returns
///
/// The distinct accounts, in first-seen order.
fn touched_accounts(rows: &[Vec<LegPlan>]) -> Vec<AccountId> {
    let mut seen: HashSet<AccountId> = HashSet::new();
    let mut accounts: Vec<AccountId> = Vec::new();
    for leg in rows.iter().flatten() {
        if seen.insert(leg.account_id.clone()) {
            accounts.push(leg.account_id.clone());
        }
    }
    accounts
}

/// Step 5: reports whether every posting already on `existing` is explained by
/// a leg of the document transaction.
///
/// This is what makes grafting a leg onto an existing transaction safe. Two
/// distinct document transactions can share an identical leg; if one of them was
/// only partially imported, occurrence ordinals alone could point at the wrong
/// transaction. Requiring the candidate to be fully explained by *this*
/// document transaction rules that out.
///
/// Each existing posting consumes at most one leg, so one leg cannot explain
/// two postings. An exact `(account, amount)` match is preferred. Failing that,
/// an elided leg may explain a posting on its account — an earlier pass may have
/// materialised the residual onto it — but **only** when that posting carries
/// exactly the residual this document determines. Accepting any amount instead
/// would reopen the hole this check exists to close: two same-day, same-narration
/// transactions sharing one identical leg would let an elided leg forgive a
/// posting that belongs to the other one, grafting foreign legs onto it.
///
/// # Arguments
///
/// * `existing` - The candidate transaction already in the database.
/// * `legs` - Every planned leg of the document transaction, matched or not.
/// * `residual` - The residual the document determines, from
///   [`document_residual`]; `None` when it determines none, which admits no
///   materialised posting at all.
///
/// # Returns
///
/// `true` if the candidate is corroborated and may be appended to.
fn corroborates(existing: &Transaction, legs: &[LegPlan], residual: Option<&Amount>) -> bool {
    let mut unconsumed: Vec<&LegPlan> = legs.iter().collect();
    for posting in existing.postings() {
        let exact = unconsumed.iter().position(|leg| {
            leg.account_id == *posting.account_id() && leg.amount.as_ref() == posting.amount()
        });
        let materialised = || {
            unconsumed.iter().position(|leg| {
                leg.amount.is_none()
                    && leg.account_id == *posting.account_id()
                    && posting.amount().is_some()
                    && posting.amount() == residual
            })
        };
        match exact.or_else(materialised) {
            Some(index) => {
                unconsumed.swap_remove(index);
            }
            None => return false,
        }
    }
    true
}

/// Builds the postings of a brand-new transaction, in leg order.
///
/// An elided leg normally persists with no amount, staying derived: fingerprints
/// are recorded over the document's own values, so a residual computed now could
/// never invalidate them later. When *no* concrete leg accompanies it — its
/// siblings named accounts that do not exist — keeping it elided would discard
/// the document's only statement of value, so the residual over the document's
/// concrete legs is materialised onto the posting. Provenance still fingerprints
/// the empty amount, so the dedup key never depends on which siblings resolved.
///
/// # Arguments
///
/// * `raw` - The document transaction, for the residual.
/// * `legs` - The planned legs to persist.
///
/// # Returns
///
/// The postings, or `None` when the residual must be materialised but the
/// document does not determine it.
fn build_postings(raw: &RawTransaction, legs: &[LegPlan]) -> Option<Vec<Posting>> {
    let residual = if legs.iter().all(|leg| leg.amount.is_none()) {
        Some(document_residual(raw)?)
    } else {
        None
    };
    Some(
        legs.iter()
            .map(|leg| leg.posting(residual.as_ref()))
            .collect(),
    )
}

/// Returns the amount an elided leg of `raw` absorbs, per the document itself.
///
/// # Arguments
///
/// * `raw` - The document transaction.
///
/// # Returns
///
/// The negated sum of the concrete legs, or `None` when they are absent, net to
/// zero, or span several commodities — in all three cases the document does not
/// determine a single residual amount.
fn document_residual(raw: &RawTransaction) -> Option<Amount> {
    let mut balances = Balances::new();
    for amount in raw
        .postings
        .iter()
        .filter_map(|posting| posting.amount.as_ref())
    {
        balances.try_sub(amount).ok()?;
    }
    let mut held = balances.into_iter();
    match (held.next(), held.next()) {
        (Some(residual), None) => Some(residual),
        _ => None,
    }
}

/// Human-facing location of `raw`, for diagnostics.
///
/// # Arguments
///
/// * `raw` - The transaction to describe.
///
/// # Returns
///
/// The importer-reported location, or a placeholder when it reported none.
fn location_of(raw: &RawTransaction) -> &str {
    raw.source_location
        .as_ref()
        .map_or("<unknown source>", |location| location.display.as_str())
}

/// Applies the run's per-row failure policy to one row's write.
///
/// A condition local to this row — input that cannot be represented, or a
/// `UNIQUE` violation showing its slot is already claimed — warns and skips the
/// row, exactly as every other unpersistable-row case in this pipeline does.
/// One bad row among thousands must not abort the run and leave a half-written
/// database behind an unclosed batch. A genuine I/O failure still propagates.
///
/// # Arguments
///
/// * `result` - The outcome of the row's write.
/// * `raw` - The document transaction, for diagnostics.
/// * `action` - What the run was attempting, for the warning.
/// * `postings` - Legs lost if the row is skipped.
/// * `counts` - Run totals to update.
///
/// # Returns
///
/// `true` if the write succeeded, `false` if the row was skipped.
///
/// # Errors
///
/// Returns the original error when it is not row-local.
fn row_local(
    result: BcResult<()>,
    raw: &RawTransaction,
    action: &str,
    postings: usize,
    counts: &mut Counts,
) -> BcResult<bool> {
    match result {
        Ok(()) => Ok(true),
        Err(error) if is_row_local(&error) => {
            tracing::warn!(
                location = location_of(raw),
                action,
                %error,
                "this document transaction cannot be persisted as it stands; skipping the \
                 row and continuing the run"
            );
            counts.skip_other(postings);
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

/// Reports whether `error` describes one row's data rather than a failure of
/// the run itself.
///
/// # Arguments
///
/// * `error` - The error a row's write returned.
///
/// # Returns
///
/// `true` if the run may warn, skip the row, and carry on.
fn is_row_local(error: &crate::BcError) -> bool {
    matches!(
        *error,
        crate::BcError::BadData(_) | crate::BcError::InvalidInput(_)
    ) || matches!(
        *error,
        crate::BcError::Database(sqlx::Error::Database(ref inner))
            if inner.is_unique_violation()
    )
}

/// The write half of an import run: everything the per-row decision needs.
struct Writer<'svc> {
    /// Transaction persistence service.
    transactions: &'svc crate::TransactionService,
    /// Source-reference persistence service.
    sources: &'svc crate::SourceService,
    /// Legs already stored for every account this run touches, keyed by
    /// `(account id string, fingerprint)`.
    existing: HashMap<(String, String), Vec<StoredLeg>>,
    /// The batch stamped onto every reference this run writes.
    batch_id: ImportBatchId,
}

impl Writer<'_> {
    /// Step 6: matches one transaction's legs, then creates, attaches, or
    /// skips.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction.
    /// * `legs` - Its planned legs; empty when nothing resolved.
    /// * `counts` - Run totals to update.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query or insert failure.
    async fn write_row(
        &self,
        raw: &RawTransaction,
        legs: &[LegPlan],
        counts: &mut Counts,
    ) -> BcResult<()> {
        if legs.is_empty() {
            return Ok(());
        }

        let owners: Vec<Option<TransactionId>> =
            legs.iter().map(|leg| self.owner_of(leg)).collect();
        let mut distinct: Vec<&TransactionId> = Vec::new();
        for owner in owners.iter().flatten() {
            if !distinct.contains(&owner) {
                distinct.push(owner);
            }
        }

        match distinct.as_slice() {
            [] => self.create(raw, legs, counts).await,
            [owner] => {
                let missing: Vec<&LegPlan> = legs
                    .iter()
                    .zip(&owners)
                    .filter_map(|(leg, owner_of_leg)| owner_of_leg.is_none().then_some(leg))
                    .collect();
                self.attach(raw, legs, owner, &missing, counts).await
            }
            conflicting => {
                // Only the legs that are not already stored are lost; the ones
                // that matched a stored leg are already in the database.
                let unstored = owners.iter().filter(|owner| owner.is_none()).count();
                tracing::warn!(
                    location = location_of(raw),
                    transactions = ?conflicting,
                    unstored,
                    "the legs of one document transaction already belong to several \
                     transactions; skipping rather than guessing which one owns it"
                );
                counts.skip_other(unstored);
                Ok(())
            }
        }
    }

    /// Step 4: finds the transaction that already owns `leg`, if any.
    ///
    /// # Arguments
    ///
    /// * `leg` - The planned leg to match.
    ///
    /// # Returns
    ///
    /// The owning transaction, or `None` when this slot is unwritten.
    fn owner_of(&self, leg: &LegPlan) -> Option<TransactionId> {
        self.existing
            .get(&(leg.account_id.to_string(), leg.fingerprint.clone()))?
            .iter()
            .find(|stored| stored.occurrence == leg.occurrence)
            .map(|stored| stored.transaction_id.clone())
    }

    /// Creates a transaction from `legs` and records provenance for each.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction.
    /// * `legs` - Its planned legs, none of which is already stored.
    /// * `counts` - Run totals to update.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on insert failure.
    async fn create(
        &self,
        raw: &RawTransaction,
        legs: &[LegPlan],
        counts: &mut Counts,
    ) -> BcResult<()> {
        let Some(postings) = build_postings(raw, legs) else {
            tracing::warn!(
                location = location_of(raw),
                "the elided leg is the only leg that resolved, and the document gives it no \
                 single amount — its concrete legs are absent, net to zero, or span several \
                 commodities; skipping the transaction"
            );
            counts.skip_other(legs.len());
            return Ok(());
        };

        let tx_id = TransactionId::new();
        // A freshly imported transaction may hold fewer legs than the document
        // did, so it can be unbalanced (an accepted state). It stays
        // `Unreconciled` until its remaining legs arrive.
        let tx = Transaction::builder()
            .id(tx_id.clone())
            .date(raw.date)
            .maybe_payee(raw.payee.clone())
            .description(raw.description.clone())
            .postings(postings.clone())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        // Create the transaction and attach its provenance atomically, so a
        // failure can never leave a posting without the reference that stops a
        // later re-import duplicating it.
        let written: BcResult<()> = async {
            let mut db_tx = self.transactions.pool().begin().await?;
            self.transactions.create_in_tx(&mut db_tx, tx).await?;
            for (posting, leg) in postings.iter().zip(legs) {
                let source = self.source_ref(raw, leg, &tx_id, posting.id());
                self.sources.attach_in_tx(&mut db_tx, &source).await?;
            }
            db_tx.commit().await?;
            Ok(())
        }
        .await;

        if !row_local(written, raw, "creating the transaction", legs.len(), counts)? {
            return Ok(());
        }

        counts.new_transactions = counts.new_transactions.saturating_add(1_usize);
        Ok(())
    }

    /// Appends the legs an earlier run could not persist to the transaction it
    /// created.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction.
    /// * `legs` - All its planned legs, used to corroborate the candidate.
    /// * `owner` - The transaction its stored legs belong to.
    /// * `missing` - The legs not yet stored.
    /// * `counts` - Run totals to update.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query or insert failure.
    async fn attach(
        &self,
        raw: &RawTransaction,
        legs: &[LegPlan],
        owner: &TransactionId,
        missing: &[&LegPlan],
        counts: &mut Counts,
    ) -> BcResult<()> {
        if missing.is_empty() {
            return Ok(());
        }

        let candidate = self.transactions.find_by_id(owner).await?;
        let residual = document_residual(raw);
        if !corroborates(&candidate, legs, residual.as_ref()) {
            tracing::warn!(
                location = location_of(raw),
                transaction = %owner,
                "a posting of the matched transaction is not explained by this document \
                 transaction; skipping rather than grafting a leg onto the wrong transaction"
            );
            counts.skip_other(missing.len());
            return Ok(());
        }

        // Appending a leg to a balanced, reconciled transaction unbalances it.
        // That is allowed — a partial import can legitimately have been
        // reconciled — but the user must be told their reconciliation is stale.
        if candidate.reconciliation() != Reconciliation::Unreconciled {
            tracing::warn!(
                location = location_of(raw),
                transaction = %owner,
                reconciliation = ?candidate.reconciliation(),
                postings = missing.len(),
                "attaching a leg to a transaction that is no longer unreconciled; its \
                 reconciliation no longer reflects its postings"
            );
        }

        // The candidate already holds a concrete leg, so a missing elided leg
        // stays elided here — there is a sibling for it to be derived from.
        let postings: Vec<Posting> = missing.iter().map(|leg| leg.posting(None)).collect();

        let written: BcResult<()> = async {
            let mut db_tx = self.transactions.pool().begin().await?;
            self.transactions
                .add_postings_in_tx(&mut db_tx, owner, &postings)
                .await?;
            for (posting, leg) in postings.iter().zip(missing) {
                let source = self.source_ref(raw, leg, owner, posting.id());
                self.sources.attach_in_tx(&mut db_tx, &source).await?;
            }
            db_tx.commit().await?;
            Ok(())
        }
        .await;

        if !row_local(
            written,
            raw,
            "appending the missing legs",
            missing.len(),
            counts,
        )? {
            return Ok(());
        }

        counts.attached_postings = counts.attached_postings.saturating_add(missing.len());
        Ok(())
    }

    /// Builds the provenance record for one persisted leg.
    ///
    /// The recorded amount is the document's, elided included: fingerprints must
    /// depend only on what the document said, never on a value derived from it.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction.
    /// * `leg` - The planned leg.
    /// * `transaction_id` - The transaction the posting belongs to.
    /// * `posting_id` - The posting this leg produced.
    ///
    /// # Returns
    ///
    /// The [`SourceRef`] to persist.
    fn source_ref(
        &self,
        raw: &RawTransaction,
        leg: &LegPlan,
        transaction_id: &TransactionId,
        posting_id: &PostingId,
    ) -> SourceRef {
        SourceRef::builder()
            .id(SourceRefId::new())
            .transaction_id(transaction_id.clone())
            .posting_id(Some(posting_id.clone()))
            .account_id(leg.account_id.clone())
            .date(raw.date)
            .narration(raw.description.clone())
            .amount(leg.amount.clone())
            .reference(raw.reference.clone())
            .occurrence(leg.occurrence)
            .import_batch_id(Some(self.batch_id.clone()))
            .created_at(Timestamp::now())
            .build()
    }
}

#[cfg(test)]
mod tests {
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::Amount;
    use bc_models::CommodityCode;
    use jiff::civil::date;
    use pretty_assertions::assert_eq;
    use rust_decimal::Decimal;
    use sqlx::SqlitePool;

    use super::*;
    use crate::RawPosting;

    /// Builds the service bundle every test needs.
    struct Services {
        transactions: crate::TransactionService,
        sources: crate::SourceService,
        accounts: crate::AccountService,
        batches: crate::ImportBatchService,
    }

    /// Creates the four services over one pool.
    fn services(pool: &SqlitePool) -> Services {
        Services {
            transactions: crate::TransactionService::new(pool.clone()),
            sources: crate::SourceService::new(pool.clone()),
            accounts: crate::AccountService::new(pool.clone()),
            batches: crate::ImportBatchService::new(pool.clone()),
        }
    }

    /// Runs an import with no profile, under the "test" importer name.
    async fn run(svcs: &Services, raws: &[RawTransaction]) -> ImportOutcome {
        execute_import(
            &svcs.transactions,
            &svcs.sources,
            &svcs.accounts,
            &svcs.batches,
            None,
            "test",
            raws,
        )
        .await
        .expect("import")
    }

    /// Creates `Assets:Bank` and `Expenses:Food`, returning their leaf IDs.
    async fn two_account_tree(pool: &SqlitePool) -> (AccountId, AccountId) {
        let svc = crate::AccountService::new(pool.clone());
        let assets = svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Assets");
        let bank = svc
            .create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("Bank");
        let expenses = svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Expenses");
        let food = svc
            .create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .parent_id(&expenses)
            .call()
            .await
            .expect("Food");
        (bank, food)
    }

    /// Creates only `Assets:Bank`, leaving `Expenses:Food` absent.
    async fn bank_only_tree(pool: &SqlitePool) -> AccountId {
        let svc = crate::AccountService::new(pool.clone());
        let assets = svc
            .create()
            .name("Assets")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Assets");
        svc.create()
            .name("Bank")
            .account_type(AccountType::Asset)
            .kind(AccountKind::DepositAccount)
            .parent_id(&assets)
            .call()
            .await
            .expect("Bank")
    }

    /// Creates `Expenses:Food`, returning its leaf ID.
    async fn add_food(pool: &SqlitePool) -> AccountId {
        let svc = crate::AccountService::new(pool.clone());
        let expenses = svc
            .create()
            .name("Expenses")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("Expenses");
        svc.create()
            .name("Food")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .parent_id(&expenses)
            .call()
            .await
            .expect("Food")
    }

    /// Creates an account beside `sibling`, under the same parent.
    async fn sibling_of(
        pool: &SqlitePool,
        sibling: &AccountId,
        name: &str,
        ty: AccountType,
    ) -> AccountId {
        let svc = crate::AccountService::new(pool.clone());
        let existing = svc.find_by_id(sibling).await.expect("sibling account");
        svc.create()
            .name(name)
            .account_type(ty)
            .kind(AccountKind::DepositAccount)
            .maybe_parent_id(existing.parent_id())
            .call()
            .await
            .expect("create sibling")
    }

    /// A posting leg on `account`; `None` elides the amount.
    fn leg(account: &str, amount: Option<i64>) -> RawPosting {
        RawPosting::builder()
            .account(account)
            .maybe_amount(
                amount.map(|value| Amount::new(Decimal::from(value), CommodityCode::new("AUD"))),
            )
            .build()
    }

    /// A transaction dated 2025-06-27, named `description`, with `legs`.
    fn raw_with(description: &str, legs: Vec<RawPosting>) -> RawTransaction {
        RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description(description)
            .postings(legs)
            .build()
    }

    /// Returns the transaction owning the single posting on `account`.
    async fn owner_of_posting(pool: &SqlitePool, account: &AccountId) -> String {
        sqlx::query_scalar("SELECT transaction_id FROM postings WHERE account_id = ?")
            .bind(account.to_string())
            .fetch_one(pool)
            .await
            .expect("posting owner")
    }

    /// Counts the postings of one transaction.
    async fn postings_of(pool: &SqlitePool, transaction: &str) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE transaction_id = ?")
            .bind(transaction)
            .fetch_one(pool)
            .await
            .expect("count postings of transaction")
    }

    /// A two-leg transaction: `Expenses:Food` +50, `Assets:Bank` elided.
    fn split_raw(description: &str) -> RawTransaction {
        RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description(description)
            .postings(vec![
                RawPosting::builder()
                    .account("Expenses:Food")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
                RawPosting::builder().account("Assets:Bank").build(),
            ])
            .build()
    }

    fn raw(desc: &str, amount: i64) -> RawTransaction {
        RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description(desc)
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(amount),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
            ])
            .build()
    }

    async fn tx_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
            .fetch_one(pool)
            .await
            .expect("count transactions")
    }

    async fn posting_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM postings")
            .fetch_one(pool)
            .await
            .expect("count postings")
    }

    /// Counts stored source references.
    async fn source_count(pool: &SqlitePool) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM transaction_sources")
            .fetch_one(pool)
            .await
            .expect("count sources")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_is_idempotent_and_incremental(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        let batch = vec![raw("COFFEE", -5), raw("LUNCH", -20)];

        let first = run(&svcs, &batch).await;
        assert_eq!(first.new_transactions, 2);
        assert_eq!(tx_count(&pool).await, 2);
        assert_eq!(
            posting_count(&pool).await,
            2,
            "each imported transaction has exactly one posting"
        );

        // Re-import the identical batch: nothing new.
        let second = run(&svcs, &batch).await;
        assert_eq!(second.new_transactions, 0);
        assert_eq!(second.attached_postings, 0);
        assert_eq!(tx_count(&pool).await, 2);

        // Append a genuinely new row: only it imports.
        let grown = vec![raw("COFFEE", -5), raw("LUNCH", -20), raw("DINNER", -40)];
        let third = run(&svcs, &grown).await;
        assert_eq!(third.new_transactions, 1);
        assert_eq!(tx_count(&pool).await, 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn identical_rows_both_import_first_run(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        // Two legitimately identical rows (same day, narration, amount, no reference).
        let batch = vec![raw("COFFEE", -5), raw("COFFEE", -5)];
        let imported = run(&svcs, &batch).await;
        assert_eq!(
            imported.new_transactions, 2,
            "both identical rows import at occurrences 0 and 1"
        );
        assert_eq!(tx_count(&pool).await, 2);

        let again = run(&svcs, &batch).await;
        assert_eq!(again.new_transactions, 0, "re-import of both is a no-op");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn rows_without_a_concrete_amount_are_skipped(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        let amountless = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("PENDING")
            .postings(vec![RawPosting::builder().account("Assets:Bank").build()])
            .build();
        let batch = vec![raw("COFFEE", -5), amountless];

        let imported = run(&svcs, &batch).await;
        assert_eq!(
            imported.new_transactions, 1,
            "only the row with a concrete posting amount is imported"
        );
        assert_eq!(tx_count(&pool).await, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn every_leg_of_a_multi_posting_row_is_persisted(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(outcome.skipped_postings, 0);
        assert!(outcome.unresolved_paths.is_empty());
        assert_eq!(tx_count(&pool).await, 1);
        assert_eq!(
            posting_count(&pool).await,
            2,
            "both legs are persisted, not just the first"
        );
        assert_eq!(
            source_count(&pool).await,
            2,
            "provenance is per-leg, including the elided one"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_elided_leg_persists_without_an_amount(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);
        run(&svcs, &[split_raw("SPLIT")]).await;

        let elided_postings: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE amount IS NULL")
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(
            elided_postings, 1,
            "the residual stays derived so later passes cannot invalidate it"
        );

        let elided_sources: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM transaction_sources WHERE amount IS NULL")
                .fetch_one(&pool)
                .await
                .expect("count");
        assert_eq!(elided_sources, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unresolvable_leg_is_skipped_and_the_rest_import(pool: SqlitePool) {
        // Only Assets:Bank exists; Expenses:Food does not.
        bank_only_tree(&pool).await;
        let svcs = services(&pool);

        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        assert_eq!(outcome.new_transactions, 1, "the transaction still imports");
        assert_eq!(outcome.skipped_postings, 1);
        assert_eq!(
            outcome.unresolved_paths,
            vec!["Expenses:Food".to_owned()],
            "the report names exactly what the user must create"
        );
        assert_eq!(
            posting_count(&pool).await,
            1,
            "only the resolvable leg persists"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_pass_attaches_the_previously_missing_leg(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool);
        let batch = vec![split_raw("SPLIT")];

        let first = run(&svcs, &batch).await;
        assert_eq!(first.skipped_postings, 1);
        assert_eq!(posting_count(&pool).await, 1);

        // The user creates the missing account and re-runs.
        add_food(&pool).await;

        let second = run(&svcs, &batch).await;

        assert_eq!(
            second.new_transactions, 0,
            "the transaction already exists; it must not be duplicated"
        );
        assert_eq!(second.attached_postings, 1);
        assert_eq!(tx_count(&pool).await, 1, "still exactly one transaction");
        assert_eq!(
            posting_count(&pool).await,
            2,
            "the missing leg was attached"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_fully_imported_row_is_a_no_op_on_re_import(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);
        let batch = vec![split_raw("SPLIT")];

        run(&svcs, &batch).await;
        let again = run(&svcs, &batch).await;

        assert_eq!(again.new_transactions, 0);
        assert_eq!(again.attached_postings, 0);
        assert_eq!(tx_count(&pool).await, 1);
        assert_eq!(posting_count(&pool).await, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unresolved_paths_are_deduplicated(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool);

        let batch: Vec<RawTransaction> = (0_i32..25_i32)
            .map(|i| split_raw(&format!("SPLIT {i}")))
            .collect();
        let outcome = run(&svcs, &batch).await;

        assert_eq!(outcome.skipped_postings, 25);
        assert_eq!(
            outcome.unresolved_paths,
            vec!["Expenses:Food".to_owned()],
            "one missing account is reported once, not 25 times"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_elided_legs_skip_the_whole_transaction(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        let ambiguous = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("AMBIGUOUS")
            .postings(vec![
                RawPosting::builder().account("Assets:Bank").build(),
                RawPosting::builder().account("Expenses:Food").build(),
            ])
            .build();

        let outcome = run(&svcs, &[ambiguous]).await;

        assert_eq!(
            outcome.new_transactions, 0,
            "two elided legs make the residual ambiguous, so nothing persists"
        );
        assert_eq!(outcome.skipped_postings, 2);
        assert_eq!(tx_count(&pool).await, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_malformed_path_skips_only_its_own_leg(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        let malformed = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("BAD PATH")
            .postings(vec![
                RawPosting::builder()
                    .account("Expenses:Food")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
                RawPosting::builder().account("Assets::Bank").build(),
            ])
            .build();

        let outcome = run(&svcs, &[malformed]).await;

        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(outcome.skipped_postings, 1);
        assert_eq!(posting_count(&pool).await, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn identical_legs_on_one_account_take_distinct_occurrences(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        // Two separate transactions whose Expenses:Food legs are identical.
        let batch = vec![split_raw("SPLIT"), split_raw("SPLIT")];
        let outcome = run(&svcs, &batch).await;

        assert_eq!(outcome.new_transactions, 2);
        let occurrences: Vec<i64> = sqlx::query_scalar(
            "SELECT occurrence FROM transaction_sources \
             WHERE amount IS NOT NULL ORDER BY occurrence",
        )
        .fetch_all(&pool)
        .await
        .expect("occurrences");
        assert_eq!(
            occurrences,
            vec![0, 1],
            "identical legs must occupy distinct slots or the UNIQUE key rejects them"
        );

        let again = run(&svcs, &batch).await;
        assert_eq!(again.new_transactions, 0, "re-import stays a no-op");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_lone_resolvable_residual_carries_the_document_amount(pool: SqlitePool) {
        // Only Assets:Bank exists, and it is the elided leg: with no concrete
        // sibling to derive from, the residual the document states is persisted
        // so the account's balance is right straight away.
        bank_only_tree(&pool).await;
        let svcs = services(&pool);

        run(&svcs, &[split_raw("SPLIT")]).await;

        let amount: Option<String> = sqlx::query_scalar("SELECT amount FROM postings")
            .fetch_one(&pool)
            .await
            .expect("posting amount");
        assert_eq!(
            amount,
            Some("-50".to_owned()),
            "the residual over the document's concrete legs is materialised"
        );

        let source_amount: Option<String> =
            sqlx::query_scalar("SELECT amount FROM transaction_sources")
                .fetch_one(&pool)
                .await
                .expect("source amount");
        assert_eq!(
            source_amount, None,
            "provenance still fingerprints the empty amount, so the dedup key does \
             not depend on which siblings resolved"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_uncorroborated_candidate_is_not_grafted_onto(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let food = add_food(&pool).await;
        let svc = crate::AccountService::new(pool.clone());
        let expenses = svc.find_by_id(&food).await.expect("Food");
        svc.create()
            .name("Fun")
            .account_type(AccountType::Expense)
            .kind(AccountKind::DepositAccount)
            .maybe_parent_id(expenses.parent_id())
            .call()
            .await
            .expect("Fun");
        let svcs = services(&pool);

        // The first document transaction imports whole.
        run(&svcs, &[split_raw("SPLIT")]).await;

        // A *different* document transaction whose Expenses:Food leg is
        // identical, so it matches the stored leg at the same occurrence, but
        // whose counter-leg names another account entirely.
        let lookalike = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("SPLIT")
            .postings(vec![
                RawPosting::builder()
                    .account("Expenses:Food")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .build(),
                RawPosting::builder().account("Expenses:Fun").build(),
            ])
            .build();

        let outcome = run(&svcs, &[lookalike]).await;

        assert_eq!(
            outcome.attached_postings, 0,
            "the candidate holds a posting this document does not explain, so its \
             missing leg must not be grafted on"
        );
        assert_eq!(outcome.new_transactions, 0);
        assert_eq!(outcome.skipped_postings, 1);
        assert_eq!(
            posting_count(&pool).await,
            2,
            "the first transaction is left exactly as it was"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_missing_leg_attaches_to_the_transaction_owning_its_siblings(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool);

        // Two transactions sharing a date and a narration, so their identical
        // Expenses:Food legs collide on fingerprint; only the second names an
        // account that does not exist yet.
        let plain = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );
        let with_fun = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Expenses:Fun", Some(30)),
                leg("Assets:Bank", None),
            ],
        );
        let batch = vec![plain, with_fun];

        let first = run(&svcs, &batch).await;
        assert_eq!(first.new_transactions, 2);
        assert_eq!(first.skipped_postings, 1, "only the Expenses:Fun leg waits");

        let fun = sibling_of(&pool, &food, "Fun", AccountType::Expense).await;
        let second = run(&svcs, &batch).await;

        assert_eq!(second.new_transactions, 0);
        assert_eq!(second.attached_postings, 1);
        assert_eq!(tx_count(&pool).await, 2, "still exactly two transactions");

        // The *elided* Assets:Bank posting belongs to the second transaction
        // alone, so it identifies which of the two candidates the leg had to land
        // on. (The first transaction's Assets:Bank leg is concrete.)
        let expected: String = sqlx::query_scalar(
            "SELECT transaction_id FROM postings WHERE account_id = ? AND amount IS NULL",
        )
        .bind(bank.to_string())
        .fetch_one(&pool)
        .await
        .expect("elided leg owner");
        assert_eq!(
            owner_of_posting(&pool, &fun).await,
            expected,
            "the leg must attach to the transaction owning its siblings, not merely \
             to a transaction that happens to share a leg"
        );
        assert_eq!(postings_of(&pool, &expected).await, 3);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_elided_leg_does_not_excuse_a_foreign_posting(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        sibling_of(&pool, &bank, "Cash", AccountType::Asset).await;
        let svcs = services(&pool);

        // One statement imports whole, leaving Assets:Bank at -50.
        let imported = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );
        run(&svcs, &[imported]).await;

        // A genuinely different transaction that happens to share the date, the
        // narration and the Expenses:Food leg, so that leg matches the stored one
        // and names the first transaction as the candidate. Its residual is -30,
        // not the -50 the candidate holds, so the candidate is not this
        // transaction and must not be appended to.
        let lookalike = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Cash", Some(-20)),
                leg("Assets:Bank", None),
            ],
        );
        let outcome = run(&svcs, &[lookalike]).await;

        assert_eq!(
            outcome.attached_postings, 0,
            "an elided leg may only explain a posting carrying the residual this \
             document determines"
        );
        assert_eq!(outcome.new_transactions, 0);
        assert_eq!(outcome.skipped_postings, 2);
        assert_eq!(tx_count(&pool).await, 1);
        assert_eq!(
            posting_count(&pool).await,
            2,
            "the candidate keeps exactly the two legs it was imported with"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn legs_owned_by_several_transactions_are_skipped(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        sibling_of(&pool, &bank, "Cash", AccountType::Asset).await;
        let svcs = services(&pool);

        // Two single-leg statements import as two separate transactions.
        let separate = vec![
            raw_with("ACME", vec![leg("Expenses:Food", Some(50))]),
            raw_with("ACME", vec![leg("Assets:Cash", Some(-20))]),
        ];
        let first = run(&svcs, &separate).await;
        assert_eq!(first.new_transactions, 2);

        // A document pairing those two legs: each is already owned, by a
        // different transaction, so which one it belongs to is unknowable.
        let paired = raw_with(
            "ACME",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Cash", Some(-20)),
            ],
        );
        let outcome = run(&svcs, &[paired]).await;

        assert_eq!(
            outcome.new_transactions, 0,
            "the legs already exist, so the document must not be duplicated"
        );
        assert_eq!(
            outcome.attached_postings, 0,
            "neither owner may absorb the other's leg"
        );
        assert_eq!(
            outcome.skipped_postings, 0,
            "both legs are already stored, so nothing was lost — only the pairing \
             was refused"
        );
        assert_eq!(tx_count(&pool).await, 2);
        assert_eq!(posting_count(&pool).await, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_multi_owner_conflict_charges_only_the_unstored_legs(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;
        sibling_of(&pool, &bank, "Cash", AccountType::Asset).await;
        sibling_of(&pool, &food, "Fun", AccountType::Expense).await;
        let svcs = services(&pool);

        // Two single-leg statements import as two separate transactions.
        let separate = vec![
            raw_with("ACME", vec![leg("Expenses:Food", Some(50))]),
            raw_with("ACME", vec![leg("Assets:Cash", Some(-20))]),
        ];
        assert_eq!(run(&svcs, &separate).await.new_transactions, 2);

        // A document pairing those two legs plus one that is not stored anywhere.
        let paired = raw_with(
            "ACME",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Cash", Some(-20)),
                leg("Expenses:Fun", Some(-30)),
            ],
        );
        let outcome = run(&svcs, &[paired]).await;

        assert_eq!(outcome.attached_postings, 0);
        assert_eq!(
            outcome.skipped_postings, 1,
            "only the Expenses:Fun leg is lost; the other two are already stored"
        );
        assert_eq!(outcome.other_skipped_postings, 1);
        assert_eq!(outcome.unresolved_path_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_unrepresentable_row_skips_itself_without_aborting_the_run(pool: SqlitePool) {
        // Only Expenses:Food exists, so the document's elided Assets:Bank leg
        // waits for a later pass.
        let food = add_food(&pool).await;
        let svcs = services(&pool);

        let document = raw_with(
            "COFFEE",
            vec![leg("Expenses:Food", Some(50)), leg("Assets:Bank", None)],
        );
        assert_eq!(
            run(&svcs, core::slice::from_ref(&document))
                .await
                .new_transactions,
            1
        );

        // Assets:Bank appears, and the user separately hand-adds an elided leg on
        // it. That leg carries no provenance, so the next pass still sees the
        // document's elided leg as missing — and appending it would give the
        // transaction two elided legs, which is unrepresentable.
        let bank = bank_only_tree(&pool).await;
        let owner: TransactionId = owner_of_posting(&pool, &food)
            .await
            .parse()
            .expect("owning transaction id");
        let stored = svcs
            .transactions
            .find_by_id(&owner)
            .await
            .expect("stored transaction");
        let mut postings = stored.postings().to_vec();
        postings.push(
            Posting::builder()
                .id(PostingId::new())
                .account_id(bank.clone())
                .build(),
        );
        svcs.transactions
            .edit(
                Transaction::builder()
                    .id(owner.clone())
                    .date(stored.date())
                    .description(stored.description())
                    .postings(postings)
                    .reconciliation(stored.reconciliation())
                    .created_at(*stored.created_at())
                    .build(),
            )
            .await
            .expect("hand-add an elided leg");

        // The bad row comes first, so a run that aborted on it would never reach
        // the good one.
        let good = raw_with(
            "LUNCH",
            vec![
                leg("Expenses:Food", Some(20)),
                leg("Assets:Bank", Some(-20)),
            ],
        );
        let outcome = run(&svcs, &[document, good]).await;

        assert_eq!(
            outcome.new_transactions, 1,
            "the run completes and still imports the good row"
        );
        assert_eq!(outcome.attached_postings, 0);
        assert_eq!(
            outcome.other_skipped_postings, 1,
            "the unrepresentable row costs only its own missing leg"
        );

        let batch = svcs
            .batches
            .find_by_id(&outcome.batch_id)
            .await
            .expect("the batch is still closed with its final counts");
        assert_eq!(batch.new_transactions, 1);
        assert_eq!(batch.attached_postings, 0);
        assert_eq!(batch.skipped_postings, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deleted_leg_is_not_recreated_by_a_re_import(pool: SqlitePool) {
        let (_bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool);

        let document = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );

        let first = run(&svcs, core::slice::from_ref(&document)).await;
        assert_eq!(first.new_transactions, 1);
        assert_eq!(posting_count(&pool).await, 2);

        // The user deliberately deletes the Assets:Bank leg.
        let owner: TransactionId = owner_of_posting(&pool, &food)
            .await
            .parse()
            .expect("owning transaction id");
        let stored = svcs
            .transactions
            .find_by_id(&owner)
            .await
            .expect("stored transaction");
        let kept: Vec<Posting> = stored
            .postings()
            .iter()
            .filter(|posting| *posting.account_id() == food)
            .cloned()
            .collect();
        assert_eq!(kept.len(), 1, "exactly the Expenses:Food leg is kept");
        let edited = Transaction::builder()
            .id(owner.clone())
            .date(stored.date())
            .description(stored.description())
            .postings(kept)
            .reconciliation(stored.reconciliation())
            .created_at(*stored.created_at())
            .build();
        svcs.transactions.edit(edited).await.expect("edit");
        assert_eq!(posting_count(&pool).await, 1);

        // Re-importing the unchanged document must respect that deletion: the
        // tombstoned reference still claims the leg's occurrence slot, so the
        // leg is never seen as missing.
        let second = run(&svcs, &[document]).await;

        assert_eq!(
            second.attached_postings, 0,
            "a leg the user deleted must not be resurrected by a re-import"
        );
        assert_eq!(second.new_transactions, 0);
        assert_eq!(
            posting_count(&pool).await,
            1,
            "the deleted leg stays deleted"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn the_outcome_records_a_batch(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool);

        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        let batch = svcs
            .batches
            .find_by_id(&outcome.batch_id)
            .await
            .expect("batch recorded");
        assert_eq!(batch.importer, "test");
        assert_eq!(batch.new_transactions, 1);

        let stamped: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM transaction_sources WHERE import_batch_id = ?",
        )
        .bind(outcome.batch_id.to_string())
        .fetch_one(&pool)
        .await
        .expect("count");
        assert_eq!(
            stamped, 2,
            "every leg's provenance names the run that wrote it"
        );
    }
}
