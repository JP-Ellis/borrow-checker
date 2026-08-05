//! Import execution: resolve every raw leg's account path and commodity code,
//! then create transactions and attach per-leg provenance for the legs that
//! are new.
//!
//! The run is a pipeline of six steps, each its own function below:
//!
//! 1. **Resolve** every leg's account path and commodity code against one
//!    snapshot of the account tree and commodity registry ([`resolve_legs`]).
//! 2. **Validate** each raw transaction's structure — two or more elided legs
//!    leave the residual ambiguous ([`has_ambiguous_residual`]).
//! 3. **Allocate** an occurrence slot per `(account, fingerprint)` across every
//!    leg of the run ([`allocate_occurrences`]).
//! 4. **Match** each planned leg against the legs already stored
//!    ([`Writer::owner_of`]).
//! 5. **Corroborate** a matched candidate — every posting already on it must be
//!    explained by a leg of this document transaction ([`corroborate`]).
//! 6. **Decide and write** per transaction — create, attach, or skip
//!    ([`Writer::write_row`]).

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Balances;
use bc_models::CommodityCode;
use bc_models::ImportBatchId;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;
use jiff::civil::Date;

use crate::AccountPath;
use crate::AccountResolver;
use crate::BcResult;
use crate::CommodityResolver;
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
    /// Legs of the document booked onto transactions an earlier run created.
    ///
    /// Counts legs *adopted* as well as legs inserted: where the user had
    /// already added the missing leg by hand, this run records provenance
    /// against their posting rather than appending a second one, so the
    /// transaction's own posting count does not always move with this number.
    pub attached_postings: usize,
    /// Postings that could not be persisted, whatever the cause. The sum of
    /// [`Self::unresolved_account_postings`],
    /// [`Self::unresolved_commodity_postings`] and
    /// [`Self::other_skipped_postings`].
    pub skipped_postings: usize,
    /// Postings skipped because their account path named no existing account.
    ///
    /// These are the legs [`Self::unresolved_accounts`] accounts for; creating
    /// those accounts and re-running attaches them.
    pub unresolved_account_postings: usize,
    /// Postings skipped because their commodity code named no registered commodity.
    ///
    /// These are the legs [`Self::unresolved_commodities`] accounts for;
    /// registering those commodities and re-running attaches them.
    pub unresolved_commodity_postings: usize,
    /// Postings skipped for any other reason — a malformed account path, a blank
    /// commodity code, an ambiguous residual, legs owned by several
    /// transactions, or a candidate that failed to corroborate. Each was warned
    /// about individually.
    pub other_skipped_postings: usize,
    /// Account paths that resolved to no account, deduplicated and sorted.
    ///
    /// This is the actionable output: create these accounts and re-run, and the
    /// next pass attaches the legs this one skipped.
    pub unresolved_accounts: Vec<String>,
    /// The distinct unregistered codes encountered, in sorted order. This is the
    /// register-these-then-re-run worklist.
    pub unresolved_commodities: Vec<String>,
}

/// Running totals for one import run, with skips attributed to their cause.
#[derive(Debug, Default)]
struct Counts {
    /// Transactions created so far.
    new_transactions: usize,
    /// Postings appended to already-existing transactions so far.
    attached_postings: usize,
    /// Postings skipped because their account path named no existing account.
    unresolved_account_postings: usize,
    /// Postings skipped because their commodity code named no registered commodity.
    unresolved_commodity_postings: usize,
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
        self.unresolved_account_postings
            .saturating_add(self.unresolved_commodity_postings)
            .saturating_add(self.other_skipped_postings)
    }
}

/// Why one leg — or one whole transaction — could not be persisted this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SkipCause {
    /// The leg's account path named no existing account. Creating the account
    /// and re-running attaches the leg.
    UnresolvedAccount,
    /// The leg's commodity code named no registered commodity. Registering the
    /// commodity and re-running attaches the leg.
    UnresolvedCommodity,
    /// Anything else: a malformed account path, a blank commodity code, an
    /// ambiguous residual, legs owned by several transactions, or a failed
    /// corroboration.
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
    /// The leg's free-text note, as the document stated it.
    note: Option<String>,
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
    /// The leg's free-text note, as the document stated it.
    note: Option<String>,
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
            .maybe_note(self.note.clone())
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
    unresolved_account_postings: usize,
    /// Legs skipped because their commodity code named no registered commodity.
    unresolved_commodity_postings: usize,
    /// Legs skipped for any other reason.
    other_skipped_postings: usize,
    /// Distinct account paths naming no account; sorted and unique by construction.
    unresolved_accounts: BTreeSet<String>,
    /// Distinct codes naming no registered commodity; sorted and unique by
    /// construction.
    unresolved_commodities: BTreeSet<String>,
}

/// Imports raw transactions, persisting every resolvable posting.
///
/// Each leg's account **path** is resolved to an id; a leg naming no existing
/// account is skipped and its path reported, so the user can create the account
/// and re-run — the next pass attaches the leg to the transaction this pass
/// created. Provenance is recorded per leg, which is what makes that possible.
///
/// A leg's **commodity code** is resolved the same way and for the same reason:
/// a code naming no registered commodity is skipped and reported, so the user
/// can register it and re-run. Resolution also canonicalises the code, so `btc`
/// and `BTC` become the one registered commodity rather than two.
///
/// A transaction is skipped whole when the document does not determine what it
/// would persist: two or more elided legs leave the residual ambiguous, or the
/// only leg that resolved is the elided one and the document fixes no single
/// amount for it. Every other failure costs one leg, not the row.
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
/// * `commodities` - Commodity service, snapshotted once for code resolution.
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
#[expect(
    clippy::too_many_arguments,
    reason = "one parameter per service the run reads or writes; bundling them \
              into a struct would only move the same list one level out"
)]
#[inline]
pub async fn execute_import(
    transactions: &crate::TransactionService,
    sources: &crate::SourceService,
    accounts: &crate::AccountService,
    commodities: &crate::CommodityService,
    batches: &crate::ImportBatchService,
    profile_id: Option<&bc_models::ProfileId>,
    importer: &str,
    raws: &[RawTransaction],
) -> BcResult<ImportOutcome> {
    let resolver = crate::AccountResolver::load(accounts).await?;
    let commodity_resolver = CommodityResolver::load(commodities).await?;
    let batch_id = batches.open(profile_id, importer).await?;

    let pass = resolve_legs(&resolver, &commodity_resolver, raws);
    let unresolved_accounts: Vec<String> = pass.unresolved_accounts.into_iter().collect();
    let unresolved_commodities: Vec<String> = pass.unresolved_commodities.into_iter().collect();
    let mut counts = Counts {
        unresolved_account_postings: pass.unresolved_account_postings,
        unresolved_commodity_postings: pass.unresolved_commodity_postings,
        other_skipped_postings: pass.other_skipped_postings,
        ..Counts::default()
    };

    let planned = allocate_occurrences(pass.rows);
    // One query per touched account for the whole run, not per row.
    let writer = Writer {
        transactions,
        sources,
        commodities: &commodity_resolver,
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
                unresolved_account_postings: counts.unresolved_account_postings,
                unresolved_commodity_postings: counts.unresolved_commodity_postings,
                other_skipped_postings: counts.other_skipped_postings,
            },
        )
        .await?;

    Ok(ImportOutcome {
        batch_id,
        new_transactions: counts.new_transactions,
        attached_postings: counts.attached_postings,
        skipped_postings: counts.skipped(),
        unresolved_account_postings: counts.unresolved_account_postings,
        unresolved_commodity_postings: counts.unresolved_commodity_postings,
        other_skipped_postings: counts.other_skipped_postings,
        unresolved_accounts,
        unresolved_commodities,
    })
}

/// Step 1 and 2: resolves every leg's account path, dropping the legs — or the
/// whole transaction — that cannot be persisted this run.
///
/// # Arguments
///
/// * `resolver` - The account-tree snapshot to resolve paths against.
/// * `commodities` - The registry snapshot to resolve commodity codes against.
/// * `raws` - Parsed transactions in document order.
///
/// # Returns
///
/// The resolved legs per transaction, the skipped-posting tallies attributed to
/// their causes, and the distinct unresolved accounts and commodities.
fn resolve_legs(
    resolver: &AccountResolver,
    commodities: &CommodityResolver,
    raws: &[RawTransaction],
) -> Resolved {
    let mut out = Resolved {
        rows: Vec::with_capacity(raws.len()),
        unresolved_account_postings: 0_usize,
        unresolved_commodity_postings: 0_usize,
        other_skipped_postings: 0_usize,
        unresolved_accounts: BTreeSet::new(),
        unresolved_commodities: BTreeSet::new(),
    };
    // Warn-once guard only: which archived accounts have already been reported.
    // Unlike an unresolved account this is not part of the outcome — importing
    // into an archived account succeeds.
    let mut archived: BTreeSet<String> = BTreeSet::new();

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
            match resolve_leg(
                resolver,
                commodities,
                raw,
                posting,
                &mut out.unresolved_accounts,
                &mut out.unresolved_commodities,
                &mut archived,
            ) {
                Ok(leg) => legs.push(leg),
                Err(SkipCause::UnresolvedAccount) => {
                    out.unresolved_account_postings =
                        out.unresolved_account_postings.saturating_add(1);
                }
                Err(SkipCause::UnresolvedCommodity) => {
                    out.unresolved_commodity_postings =
                        out.unresolved_commodity_postings.saturating_add(1);
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
/// * `commodities` - The registry snapshot to resolve commodity codes against.
/// * `raw` - The transaction the leg belongs to, for diagnostics.
/// * `posting` - The leg to resolve.
/// * `unresolved` - Accumulator of distinct unresolved accounts; also the
///   warn-once guard, since inserting a path reports whether it is new.
/// * `unresolved_commodities` - Accumulator of distinct unregistered commodity
///   codes, and likewise their warn-once guard.
/// * `archived` - Warn-once guard for archived accounts already reported.
///
/// # Returns
///
/// The [`ResolvedLeg`], or the [`SkipCause`] that stopped it being persisted
/// this run.
fn resolve_leg(
    resolver: &AccountResolver,
    commodities: &CommodityResolver,
    raw: &RawTransaction,
    posting: &RawPosting,
    unresolved: &mut BTreeSet<String>,
    unresolved_commodities: &mut BTreeSet<String>,
    archived_seen: &mut BTreeSet<String>,
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
            // Warn once per distinct account, for the same reason the unresolved
            // path below does: one archived account named by every row of a file
            // should log one line, not one per row.
            if archived && archived_seen.insert(path.to_string()) {
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
            return Err(SkipCause::UnresolvedAccount);
        }
    };

    let amount = match canonicalise(commodities, posting.amount.as_ref()) {
        Canonical::Resolved(amount) => amount,
        Canonical::Unregistered(code) => {
            // Warn once per distinct code, for the same reason an unresolved
            // account path does: one unregistered commodity named by every row
            // of a file should log one line, not one per row.
            if unresolved_commodities.insert(code.clone()) {
                tracing::warn!(
                    location = location_of(raw),
                    commodity = code.as_str(),
                    "commodity code names no registered commodity; register it and \
                     re-run to attach the legs skipped now"
                );
            }
            return Err(SkipCause::UnresolvedCommodity);
        }
        Canonical::Blank => {
            tracing::warn!(
                location = location_of(raw),
                "posting has a blank commodity code; skipping this leg"
            );
            return Err(SkipCause::Other);
        }
    };

    // A balance is corroboration, not the posting itself: an unresolved
    // commodity on it costs the balance, not the leg. Nothing persists the
    // reported balance yet, so dropping it is exactly the diagnostic below.
    if let Canonical::Unregistered(code) = canonicalise(commodities, posting.balance.as_ref()) {
        tracing::warn!(
            location = location_of(raw),
            commodity = code.as_str(),
            "reported balance names an unregistered commodity; dropping the balance"
        );
    }

    Ok(ResolvedLeg {
        // Fingerprinted over the *canonical* code, so a file stating `btc` and a
        // later one stating `BTC` produce one fingerprint, not two, and the
        // second re-import dedups rather than duplicating the posting.
        fingerprint: SourceRef::compute_fingerprint(
            raw.date,
            &raw.description,
            amount.as_ref(),
            raw.reference.as_deref(),
        ),
        account_id,
        amount,
        note: posting.note.clone(),
    })
}

/// The outcome of canonicalising one optional amount's commodity code.
enum Canonical {
    /// Resolved — `None` when the amount was elided to begin with.
    Resolved(Option<Amount>),
    /// The code named no registered commodity.
    Unregistered(String),
    /// The code was empty.
    Blank,
}

/// Rewrites an amount's commodity code to its registered spelling.
///
/// # Arguments
///
/// * `commodities` - The registry snapshot to resolve against.
/// * `amount` - The amount to canonicalise; `None` for an elided leg.
///
/// # Returns
///
/// The canonicalised amount, or why it could not be resolved.
fn canonicalise(commodities: &CommodityResolver, amount: Option<&Amount>) -> Canonical {
    let Some(stated) = amount else {
        return Canonical::Resolved(None);
    };
    let trimmed = stated.commodity().as_str().trim();
    if trimmed.is_empty() {
        return Canonical::Blank;
    }
    match commodities.resolve(&CommodityCode::new(trimmed)) {
        Some(code) => {
            Canonical::Resolved(Some(Amount::new(stated.value(), CommodityCode::new(code))))
        }
        None => Canonical::Unregistered(trimmed.to_owned()),
    }
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
                        note: leg.note,
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

/// A leg of the document, paired with whether it is already stored.
struct Candidate<'leg> {
    /// The planned leg.
    leg: &'leg LegPlan,
    /// Whether a stored reference already claims this leg's occurrence slot.
    stored: bool,
}

/// What corroborating a candidate transaction established.
struct Corroborated<'leg> {
    /// Legs whose posting the user already wrote by hand: adopt that posting by
    /// recording provenance against it, rather than inserting a second one.
    adoptions: Vec<(PostingId, &'leg LegPlan)>,
    /// Legs with no posting at all, which must be inserted.
    insertions: Vec<&'leg LegPlan>,
}

/// Step 5: explains every posting already on `existing` with a leg of the
/// document transaction, or reports that one cannot be explained.
///
/// This is what makes appending a leg to an existing transaction safe. Two
/// distinct document transactions can share an identical leg; if one of them was
/// only partially imported, occurrence ordinals alone could point at the wrong
/// transaction. Requiring the candidate to be fully explained by *this* document
/// transaction rules that out. Each posting consumes at most one leg, so one leg
/// cannot explain two postings.
///
/// A posting is explained one of two ways, and which one applies turns on
/// whether it carries provenance:
///
/// - **By its reference.** A posting an import wrote is explained when its
///   reference's `(account, fingerprint, occurrence)` matches a leg. Every
///   component comes from the reference, which records what the *document*
///   said, so an edit to the posting — a corrected amount, a recategorisation —
///   moves the posting but never its reference, and the document's remaining
///   legs can still arrive. Matching on the posting's current amount or account
///   instead would strand them permanently.
/// - **By adoption.** A posting with no provenance is one the user wrote, in all
///   likelihood the very leg an earlier pass could not resolve. It is explained
///   by an unstored leg on its account carrying the same amount, and that leg is
///   then *adopted*: provenance is recorded against the existing posting instead
///   of a duplicate being inserted.
///
/// A posting explained neither way belongs to some other document, and the
/// candidate is refused.
///
/// # Arguments
///
/// * `existing` - The candidate transaction already in the database.
/// * `legs` - Every leg of the document transaction, stored or not.
/// * `provenance` - Provenance of `existing`'s postings, keyed by posting id,
///   from [`crate::SourceService::provenance_by_posting`].
///
/// # Returns
///
/// The legs to adopt and to insert, or `None` when a posting is unexplained.
fn corroborate<'leg>(
    existing: &Transaction,
    legs: &[Candidate<'leg>],
    provenance: &HashMap<String, crate::PostingProvenance>,
) -> Option<Corroborated<'leg>> {
    let mut unconsumed: Vec<&Candidate<'leg>> = legs.iter().collect();
    let mut adoptions = Vec::new();

    for posting in existing.postings() {
        let index = if let Some(recorded) = provenance.get(&posting.id().to_string()) {
            // Written by an import: match what the document said, not what the
            // posting now holds — account included, since an edit can
            // recategorise the posting away from the account it was booked to.
            unconsumed.iter().position(|candidate| {
                candidate.leg.account_id == recorded.account_id
                    && candidate.leg.fingerprint == recorded.fingerprint
                    && candidate.leg.occurrence == recorded.occurrence
            })?
        } else {
            // Written by the user: match on the value, and adopt it.
            let found = unconsumed.iter().position(|candidate| {
                !candidate.stored
                    && candidate.leg.account_id == *posting.account_id()
                    && candidate.leg.amount.as_ref() == posting.amount()
            })?;
            let adopted = unconsumed.get(found).map(|candidate| candidate.leg)?;
            adoptions.push((posting.id().clone(), adopted));
            found
        };
        unconsumed.swap_remove(index);
    }

    // Whatever no posting accounted for, and is not already stored elsewhere,
    // has no posting yet.
    let insertions = unconsumed
        .into_iter()
        .filter(|candidate| !candidate.stored)
        .map(|candidate| candidate.leg)
        .collect();

    Some(Corroborated {
        adoptions,
        insertions,
    })
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
/// * `commodities` - The registry snapshot the residual's commodity is
///   canonicalised against.
///
/// # Returns
///
/// The postings, or `None` when the residual must be materialised but the
/// document does not determine it.
fn build_postings(
    raw: &RawTransaction,
    legs: &[LegPlan],
    commodities: &CommodityResolver,
) -> BcResult<Option<Vec<Posting>>> {
    let residual = if legs.iter().all(|leg| leg.amount.is_none()) {
        match document_residual(raw, commodities)? {
            Some(residual) => Some(residual),
            None => return Ok(None),
        }
    } else {
        None
    };
    Ok(Some(
        legs.iter()
            .map(|leg| leg.posting(residual.as_ref()))
            .collect(),
    ))
}

/// Returns the amount an elided leg of `raw` absorbs, per the document itself.
///
/// # Arguments
///
/// * `raw` - The document transaction.
/// * `commodities` - The registry snapshot each concrete leg's code is
///   canonicalised against before summing, so two spellings of one commodity
///   contribute to a single balance rather than two.
///
/// # Returns
///
/// The negated sum of the concrete legs' canonical amounts, or `None` when they
/// are absent, net to zero, name a code that resolves to no registered
/// commodity, or span several commodities once canonicalised — in all four
/// cases the document does not determine a single residual amount.
///
/// # Errors
///
/// Returns [`crate::BcError::BadData`] if the concrete legs sum out of
/// [`rust_decimal::Decimal`]'s range. That is a defect in the row rather than an
/// undetermined residual, and reporting it as the latter would name a cause the
/// document does not have.
fn document_residual(
    raw: &RawTransaction,
    commodities: &CommodityResolver,
) -> BcResult<Option<Amount>> {
    let mut balances = Balances::new();
    for posting in raw
        .postings
        .iter()
        .filter(|posting| posting.amount.is_some())
    {
        let amount = match canonicalise(commodities, posting.amount.as_ref()) {
            Canonical::Resolved(Some(amount)) => amount,
            // An amount whose commodity does not resolve leaves the document
            // without a residual it determines, exactly as an absent, net-zero,
            // or multi-commodity residual does.
            Canonical::Resolved(None) | Canonical::Unregistered(_) | Canonical::Blank => {
                return Ok(None);
            }
        };
        balances.try_sub(&amount).map_err(|e| {
            crate::BcError::BadData(format!("summing this row's amounts overflowed: {e}"))
        })?;
    }
    let mut held = balances.into_iter();
    Ok(match (held.next(), held.next()) {
        (Some(residual), None) => Some(residual),
        _ => None,
    })
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
    Ok(row_local_value(result, raw, action, postings, counts)?.is_some())
}

/// As [`row_local`], for a step that produces a value.
///
/// # Arguments
///
/// * `result` - The outcome of the row's step.
/// * `raw` - The document transaction, for diagnostics.
/// * `action` - What the run was attempting, for the warning.
/// * `postings` - Legs lost if the row is skipped.
/// * `counts` - Run totals to update.
///
/// # Returns
///
/// The value on success, or `None` if the row was skipped.
///
/// # Errors
///
/// Returns the original error when it is not row-local.
fn row_local_value<T>(
    result: BcResult<T>,
    raw: &RawTransaction,
    action: &str,
    postings: usize,
    counts: &mut Counts,
) -> BcResult<Option<T>> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_row_local(&error) => {
            tracing::warn!(
                location = location_of(raw),
                action,
                %error,
                "this document transaction cannot be persisted as it stands; skipping the \
                 row and continuing the run"
            );
            counts.skip_other(postings);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Drops the labelled dates a row states more than once, keeping the first.
///
/// `transaction_dates` is keyed by `(transaction_id, label)`, so a repeated
/// label would raise a unique violation and cost the row every one of its
/// postings. A label stated twice is a defect in the document's metadata, not
/// grounds to discard the transaction it decorates.
///
/// # Arguments
///
/// * `raw` - The document transaction, for its dates and for diagnostics.
///
/// # Returns
///
/// The row's labelled dates in document order, one entry per label.
fn distinct_extra_dates(raw: &RawTransaction) -> Vec<(String, Date)> {
    let mut seen = HashSet::new();
    raw.extra_dates
        .iter()
        .filter(|(label, date)| {
            if seen.insert(label.clone()) {
                return true;
            }
            tracing::warn!(
                location = location_of(raw),
                label,
                %date,
                "the row states this date label more than once; keeping the first and \
                 dropping this one"
            );
            false
        })
        .cloned()
        .collect()
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
///
/// [`crate::BcError::InvalidInput`] is deliberately **not** row-local. Every
/// per-row rejection in this pipeline raises [`crate::BcError::BadData`]; the one
/// thing that raises `InvalidInput` here is
/// [`crate::SourceService::attach_in_tx`] refusing a reference whose posting does
/// not belong to the named transaction and account. That is an internal
/// invariant this module is responsible for upholding, so it must surface as a
/// failure rather than be absorbed into the skip count.
fn is_row_local(error: &crate::BcError) -> bool {
    matches!(*error, crate::BcError::BadData(_))
        || matches!(
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
    /// Commodity registry snapshot, for canonicalising a materialised
    /// residual's commodity code.
    commodities: &'svc CommodityResolver,
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
                let candidates: Vec<Candidate<'_>> = legs
                    .iter()
                    .zip(&owners)
                    .map(|(leg, owner_of_leg)| Candidate {
                        leg,
                        stored: owner_of_leg.is_some(),
                    })
                    .collect();
                self.attach(raw, &candidates, owner, counts).await
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
        // An overflow is this row's own defect, so it warns and skips like any
        // other unpersistable row rather than aborting the run.
        let built = row_local_value(
            build_postings(raw, legs, self.commodities),
            raw,
            "summing the row's amounts",
            legs.len(),
            counts,
        )?;
        let Some(Some(postings)) = built else {
            if built.is_some() {
                tracing::warn!(
                    location = location_of(raw),
                    "the elided leg is the only leg that resolved, and the document gives it no \
                     single amount — its concrete legs are absent, net to zero, name a \
                     commodity that resolves to nothing registered, or span several \
                     commodities; skipping the transaction"
                );
                counts.skip_other(legs.len());
            }
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
            .maybe_note(raw.note.clone())
            .extra_dates(distinct_extra_dates(raw))
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
                let source = self.source_ref(raw, leg, &tx_id, posting.id(), true);
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
    /// * `candidates` - Every leg of the document, flagged as stored or not.
    /// * `owner` - The transaction its stored legs belong to.
    /// * `counts` - Run totals to update.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query or insert failure.
    async fn attach(
        &self,
        raw: &RawTransaction,
        candidates: &[Candidate<'_>],
        owner: &TransactionId,
        counts: &mut Counts,
    ) -> BcResult<()> {
        let unstored = candidates.iter().filter(|c| !c.stored).count();
        if unstored == 0 {
            return Ok(());
        }

        // Both lookups precede any write, and a failure of either is treated as
        // row-local: one unreadable candidate must not abort a run whose other
        // rows are fine.
        let looked_up: BcResult<(Transaction, HashMap<String, crate::PostingProvenance>)> = async {
            let candidate = self.transactions.find_by_id(owner).await?;
            let provenance = self.sources.provenance_by_posting(owner).await?;
            Ok((candidate, provenance))
        }
        .await;
        let Some((candidate, provenance)) = row_local_value(
            looked_up,
            raw,
            "reading the matched transaction",
            unstored,
            counts,
        )?
        else {
            return Ok(());
        };

        let Some(Corroborated {
            adoptions,
            insertions,
        }) = corroborate(&candidate, candidates, &provenance)
        else {
            tracing::warn!(
                location = location_of(raw),
                transaction = %owner,
                "a posting of the matched transaction is not explained by this document \
                 transaction; skipping rather than grafting a leg onto the wrong transaction"
            );
            counts.skip_other(unstored);
            return Ok(());
        };

        // Appending a leg to a balanced, reconciled transaction unbalances it.
        // That is allowed — a partial import can legitimately have been
        // reconciled — but the user must be told their reconciliation is stale.
        if !insertions.is_empty() && candidate.reconciliation() != Reconciliation::Unreconciled {
            tracing::warn!(
                location = location_of(raw),
                transaction = %owner,
                reconciliation = ?candidate.reconciliation(),
                postings = insertions.len(),
                "attaching a leg to a transaction that is no longer unreconciled; its \
                 reconciliation no longer reflects its postings"
            );
        }

        for (posting_id, _leg) in &adoptions {
            tracing::info!(
                location = location_of(raw),
                transaction = %owner,
                posting = %posting_id,
                "this leg is already present as a posting with no provenance; recording the \
                 import against it rather than adding a second copy"
            );
        }

        // The candidate already holds a concrete leg, so an unstored elided leg
        // stays elided here — there is a sibling for it to be derived from.
        let postings: Vec<Posting> = insertions.iter().map(|leg| leg.posting(None)).collect();

        let written: BcResult<()> = async {
            let mut db_tx = self.transactions.pool().begin().await?;
            if !postings.is_empty() {
                self.transactions
                    .add_postings_in_tx(&mut db_tx, owner, &postings)
                    .await?;
            }
            for (posting, leg) in postings.iter().zip(&insertions) {
                let source = self.source_ref(raw, leg, owner, posting.id(), true);
                self.sources.attach_in_tx(&mut db_tx, &source).await?;
            }
            for (posting_id, leg) in &adoptions {
                let source = self.source_ref(raw, leg, owner, posting_id, false);
                self.sources.attach_in_tx(&mut db_tx, &source).await?;
            }
            db_tx.commit().await?;
            Ok(())
        }
        .await;

        if !row_local(
            written,
            raw,
            "appending the unstored legs",
            unstored,
            counts,
        )? {
            return Ok(());
        }

        counts.attached_postings = counts.attached_postings.saturating_add(unstored);
        Ok(())
    }

    /// Builds the provenance record for one persisted leg.
    ///
    /// The recorded amount is the *canonicalised* amount — the commodity code
    /// resolved to its registered spelling, elided still included — not the
    /// document's raw text. This is deliberate: fingerprinting the raw code
    /// would make a file importing `btc` and a later one importing `BTC`
    /// produce different fingerprints for the same posting and duplicate it.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction.
    /// * `leg` - The planned leg.
    /// * `transaction_id` - The transaction the posting belongs to.
    /// * `posting_id` - The posting this leg produced.
    /// * `owns_posting` - `true` when this run inserted that posting, `false`
    ///   when it adopted a posting the user had already written.
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
        owns_posting: bool,
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
            .owns_posting(owns_posting)
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
    use rust_decimal_macros::dec;
    use sqlx::SqlitePool;

    use super::*;
    use crate::RawPosting;

    /// Builds the service bundle every test needs.
    struct Services {
        transactions: crate::TransactionService,
        sources: crate::SourceService,
        accounts: crate::AccountService,
        commodities: crate::CommodityService,
        batches: crate::ImportBatchService,
    }

    /// Creates the five services over one pool, with the default commodity set
    /// seeded — every leg's code is now resolved against the registry, so a run
    /// over an empty one would skip every leg for want of a commodity.
    async fn services(pool: &SqlitePool) -> Services {
        let commodities = crate::CommodityService::new(pool.clone());
        commodities
            .seed_defaults()
            .await
            .expect("seed the default commodities");
        Services {
            transactions: crate::TransactionService::new(pool.clone()),
            sources: crate::SourceService::new(pool.clone()),
            accounts: crate::AccountService::new(pool.clone()),
            commodities,
            batches: crate::ImportBatchService::new(pool.clone()),
        }
    }

    /// Runs an import with no profile, under the "test" importer name.
    async fn run(svcs: &Services, raws: &[RawTransaction]) -> ImportOutcome {
        execute_import(
            &svcs.transactions,
            &svcs.sources,
            &svcs.accounts,
            &svcs.commodities,
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

    /// Counts the postings booked to one account.
    async fn postings_of_account(pool: &SqlitePool, account: &AccountId) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM postings WHERE account_id = ?")
            .bind(account.to_string())
            .fetch_one(pool)
            .await
            .expect("count postings of account")
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

    /// A transaction with `concrete` legs — each `(account, amount, code)` —
    /// plus an elided leg on `Assets:Bank`.
    fn row_with_elided_bank(description: &str, concrete: &[(&str, i64, &str)]) -> RawTransaction {
        let mut postings: Vec<RawPosting> = concrete
            .iter()
            .map(|(account, amount, code)| {
                RawPosting::builder()
                    .account(*account)
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(*amount),
                        CommodityCode::new(*code),
                    )))
                    .build()
            })
            .collect();
        postings.push(RawPosting::builder().account("Assets:Bank").build());
        raw_with(description, postings)
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

    /// Reads the `note` column of the single posting booked to `account`.
    async fn note_of_posting(pool: &SqlitePool, account: &AccountId) -> Option<String> {
        sqlx::query_scalar("SELECT note FROM postings WHERE account_id = ?")
            .bind(account.to_string())
            .fetch_one(pool)
            .await
            .expect("posting note")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn posting_note_is_persisted(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = raw_with(
            "coffee",
            vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .note("paid by card")
                    .build(),
            ],
        );

        run(&svcs, &[raw]).await;

        assert_eq!(
            note_of_posting(&pool, &bank).await,
            Some("paid by card".to_owned())
        );
    }

    /// Reads the `note` column of the single stored transaction.
    async fn note_of_transaction(pool: &SqlitePool) -> Option<String> {
        sqlx::query_scalar("SELECT note FROM transactions")
            .fetch_one(pool)
            .await
            .expect("transaction note")
    }

    /// Reads every `(label, date)` pair stored for the single transaction.
    async fn dates_of_transaction(pool: &SqlitePool) -> Vec<(String, String)> {
        sqlx::query_as("SELECT label, date FROM transaction_dates ORDER BY label")
            .fetch_all(pool)
            .await
            .expect("transaction dates")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_note_and_extra_dates_are_persisted(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .note("split with flatmate")
            .extra_dates(vec![
                ("settled".to_owned(), date(2025, 6, 29)),
                ("posted".to_owned(), date(2025, 6, 28)),
            ])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        run(&svcs, &[raw]).await;

        assert_eq!(
            note_of_transaction(&pool).await,
            Some("split with flatmate".to_owned())
        );
        assert_eq!(
            dates_of_transaction(&pool).await,
            vec![
                ("posted".to_owned(), "2025-06-28".to_owned()),
                ("settled".to_owned(), "2025-06-29".to_owned()),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_repeated_date_label_costs_the_date_not_the_row(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .extra_dates(vec![
                ("posted".to_owned(), date(2025, 6, 28)),
                ("posted".to_owned(), date(2025, 6, 30)),
                ("settled".to_owned(), date(2025, 6, 29)),
            ])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        // The row still persists: a duplicated label is a defect in the
        // document's metadata, not grounds to discard the transaction.
        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(
            dates_of_transaction(&pool).await,
            vec![
                ("posted".to_owned(), "2025-06-28".to_owned()),
                ("settled".to_owned(), "2025-06-29".to_owned()),
            ]
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn attaching_a_leg_does_not_revise_transaction_metadata(pool: SqlitePool) {
        // First run: only Assets:Bank exists, so the Expenses:Food leg is skipped
        // and the transaction is created carrying the document's note.
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;
        let first = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .note("original note")
            .postings(vec![
                leg("Assets:Bank", Some(-50_i64)),
                leg("Expenses:Food", Some(50_i64)),
            ])
            .build();
        run(&svcs, &[first]).await;

        // Second run: the account now exists and the document's note has changed.
        // The missing leg attaches; the note must not follow it.
        add_food(&pool).await;
        let second = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .note("revised note")
            .postings(vec![
                leg("Assets:Bank", Some(-50_i64)),
                leg("Expenses:Food", Some(50_i64)),
            ])
            .build();
        let outcome = run(&svcs, &[second]).await;

        assert_eq!(outcome.attached_postings, 1);
        assert_eq!(
            note_of_transaction(&pool).await,
            Some("original note".to_owned())
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn import_is_idempotent_and_incremental(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(outcome.skipped_postings, 0);
        assert!(outcome.unresolved_accounts.is_empty());
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
        let svcs = services(&pool).await;
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
        let svcs = services(&pool).await;

        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        assert_eq!(outcome.new_transactions, 1, "the transaction still imports");
        assert_eq!(outcome.skipped_postings, 1);
        assert_eq!(
            outcome.unresolved_accounts,
            vec!["Expenses:Food".to_owned()],
            "the report names exactly what the user must create"
        );
        assert_eq!(
            posting_count(&pool).await,
            1,
            "only the resolvable leg persists"
        );

        // The batch record must attribute the skip to the cause it had, since a
        // later discard reasons from these numbers.
        let batch = svcs
            .batches
            .find_by_id(&outcome.batch_id)
            .await
            .expect("the batch record");
        let counts = batch.counts.expect("a closed batch reports its counts");
        assert_eq!(counts.skipped(), 1);
        assert_eq!(
            counts.unresolved_account_postings, 1,
            "the skip is attributed to the missing account, not to some other cause"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_second_pass_attaches_the_previously_missing_leg(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;
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
        let svcs = services(&pool).await;
        let batch = vec![split_raw("SPLIT")];

        run(&svcs, &batch).await;
        let again = run(&svcs, &batch).await;

        assert_eq!(again.new_transactions, 0);
        assert_eq!(again.attached_postings, 0);
        assert_eq!(tx_count(&pool).await, 1);
        assert_eq!(posting_count(&pool).await, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unresolved_accounts_are_deduplicated(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;

        let batch: Vec<RawTransaction> = (0_i32..25_i32)
            .map(|i| {
                raw_with(
                    &format!("SPLIT {i}"),
                    vec![
                        leg("Expenses:Food", Some(50)),
                        leg("Bills:Rent", Some(20)),
                        leg("Assets:Bank", None),
                    ],
                )
            })
            .collect();
        let outcome = run(&svcs, &batch).await;

        assert_eq!(outcome.skipped_postings, 50);
        assert_eq!(
            outcome.unresolved_accounts,
            vec!["Bills:Rent".to_owned(), "Expenses:Food".to_owned()],
            "each missing account is reported once, in sorted order, not 25 times each"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn two_elided_legs_skip_the_whole_transaction(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
    async fn a_leg_naming_an_archived_account_still_imports(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;
        crate::AccountService::new(pool.clone())
            .archive(&food)
            .await
            .expect("archive Expenses:Food");
        let svcs = services(&pool).await;

        // Archiving records that an account is no longer in use; it does not make
        // history unimportable, so the leg persists and is only warned about.
        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(
            outcome.skipped_postings, 0,
            "an archived account resolves; it is not a missing one"
        );
        assert!(outcome.unresolved_accounts.is_empty());
        assert_eq!(postings_of_account(&pool, &food).await, 1);
        assert_eq!(postings_of_account(&pool, &bank).await, 1);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn identical_legs_on_different_accounts_share_occurrence_zero(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        sibling_of(&pool, &bank, "Cash", AccountType::Asset).await;
        let svcs = services(&pool).await;

        // Two legs of one transaction that fingerprint identically — same date,
        // narration, amount and (absent) reference — on different accounts. Slots
        // are allocated per account, so both take occurrence 0; keying on the
        // fingerprint alone would push the second to 1 and shift the ordinals a
        // later pass matches against.
        let document = raw_with(
            "TRANSFER",
            vec![
                leg("Assets:Bank", Some(-25)),
                leg("Assets:Cash", Some(-25)),
                leg("Expenses:Food", Some(50)),
            ],
        );
        let outcome = run(&svcs, core::slice::from_ref(&document)).await;
        assert_eq!(outcome.new_transactions, 1);

        let slots: Vec<(String, i64)> = sqlx::query_as(
            "SELECT account_id, occurrence FROM transaction_sources \
             WHERE amount = '-25' ORDER BY account_id",
        )
        .fetch_all(&pool)
        .await
        .expect("the two identical legs");
        assert_eq!(slots.len(), 2, "both legs are recorded");
        assert!(
            slots.iter().all(|(_account, occurrence)| *occurrence == 0),
            "each account's slots are counted independently, got {slots:?}"
        );

        let again = run(&svcs, &[document]).await;
        assert_eq!(
            again.new_transactions, 0,
            "and a re-import still recognises both legs"
        );
        assert_eq!(again.attached_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_lone_resolvable_residual_carries_the_document_amount(pool: SqlitePool) {
        // Only Assets:Bank exists, and it is the elided leg: with no concrete
        // sibling to derive from, the residual the document states is persisted
        // so the account's balance is right straight away.
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;

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

    /// The residual is derived from the *raw* concrete legs, so its commodity
    /// code must be canonicalised exactly as any other leg's is — a
    /// materialised residual must never carry a non-canonical spelling into
    /// the database.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_materialised_residual_stores_the_canonical_spelling(pool: SqlitePool) {
        // Only Assets:Bank exists; Expenses:Food does not, so the elided Bank
        // leg is the only one that resolves and its residual is materialised.
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;

        run(
            &svcs,
            &[row_with_elided_bank(
                "SPLIT",
                &[("Expenses:Food", 50, "aud")],
            )],
        )
        .await;

        let stored: (Option<String>, Option<String>) =
            sqlx::query_as("SELECT amount, commodity FROM postings")
                .fetch_one(&pool)
                .await
                .expect("posting");
        assert_eq!(stored.0, Some("-50".to_owned()));
        assert_eq!(
            stored.1,
            Some("AUD".to_owned()),
            "the materialised residual is stored in the registry's canonical spelling, \
             not the document's own"
        );
    }

    /// Two spellings of one commodity among the concrete legs must sum as one
    /// commodity, not two — summing the raw spellings would leave `Balances`
    /// holding two entries and `document_residual` would report the row as
    /// spanning several commodities, when it in fact spans exactly one.
    #[sqlx::test(migrations = "./migrations")]
    async fn two_spellings_of_one_commodity_sum_as_one(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;

        let outcome = run(
            &svcs,
            &[row_with_elided_bank(
                "SPLIT",
                &[("Expenses:Food", 30, "aud"), ("Expenses:Fun", 20, "AUD")],
            )],
        )
        .await;

        assert_eq!(
            outcome.new_transactions, 1,
            "the residual is determined once the two spellings are merged"
        );
        let stored: (Option<String>, Option<String>) =
            sqlx::query_as("SELECT amount, commodity FROM postings")
                .fetch_one(&pool)
                .await
                .expect("posting");
        assert_eq!(
            stored.0,
            Some("-50".to_owned()),
            "the two spellings summed as one commodity, not two"
        );
        assert_eq!(stored.1, Some("AUD".to_owned()));
    }

    /// A residual whose only concrete leg names an unregistered commodity
    /// must not persist an unregistered code — the row is skipped, exactly as
    /// an absent, net-zero, or multi-commodity residual already is.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unresolvable_residual_commodity_skips_the_transaction(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;

        let outcome = run(
            &svcs,
            &[row_with_elided_bank(
                "SPLIT",
                &[("Expenses:Food", 50, "DOGE")],
            )],
        )
        .await;

        assert_eq!(
            outcome.new_transactions, 0,
            "the document does not determine a persistable residual"
        );
        assert_eq!(tx_count(&pool).await, 0);
        assert_eq!(posting_count(&pool).await, 0);
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
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
            "a posting is explained by the reference it carries, and this document \
             fixes no leg matching the candidate's stored Assets:Bank reference"
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
    async fn a_residual_coinciding_with_a_foreign_posting_does_not_corroborate(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;
        sibling_of(&pool, &bank, "Cash", AccountType::Asset).await;
        sibling_of(&pool, &food, "Fun", AccountType::Expense).await;
        let svcs = services(&pool).await;

        // One statement imports whole, leaving Assets:Bank at -50.
        let imported = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );
        run(&svcs, &[imported]).await;

        // A different transaction sharing the date, the narration and the
        // Expenses:Food leg, so that leg names the first transaction as the
        // candidate. Its extra legs net to zero, so its residual is -50 — exactly
        // what the candidate's Assets:Bank posting already holds. Matching on that
        // amount would corroborate a foreign transaction; matching on the stored
        // reference (which fingerprints -50, not an absent amount) refuses it.
        let lookalike = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Cash", Some(10)),
                leg("Expenses:Fun", Some(-10)),
                leg("Assets:Bank", None),
            ],
        );
        let outcome = run(&svcs, &[lookalike]).await;

        assert_eq!(
            outcome.attached_postings, 0,
            "a residual that coincides with a foreign posting's amount must not \
             corroborate the transaction holding it"
        );
        assert_eq!(outcome.new_transactions, 0);
        assert_eq!(outcome.skipped_postings, 3);
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
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

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
        assert_eq!(outcome.unresolved_account_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_hand_added_elided_leg_is_adopted_not_duplicated(pool: SqlitePool) {
        // Only Expenses:Food exists, so the document's elided Assets:Bank leg
        // waits for a later pass.
        let food = add_food(&pool).await;
        let svcs = services(&pool).await;

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
        // it — the obvious response to the partial-import warning. That leg
        // carries no provenance, so the document's elided leg is still unstored;
        // it must be recorded against the posting the user wrote rather than
        // appended beside it, which would leave two elided legs.
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

        // A second, unrelated row proves the run carries on either way.
        let good = raw_with(
            "LUNCH",
            vec![
                leg("Expenses:Food", Some(20)),
                leg("Assets:Bank", Some(-20)),
            ],
        );
        let outcome = run(&svcs, &[document.clone(), good]).await;

        assert_eq!(
            outcome.new_transactions, 1,
            "the run completes and still imports the unrelated row"
        );
        assert_eq!(
            outcome.attached_postings, 1,
            "the document's elided leg is accounted for by the posting the user added"
        );
        assert_eq!(outcome.skipped_postings, 0);
        assert_eq!(
            postings_of(&pool, &owner.to_string()).await,
            2,
            "the hand-added leg is adopted, not duplicated"
        );

        // Adoption claims the occurrence slot, so the leg cannot reappear later.
        let third = run(&svcs, &[document]).await;
        assert_eq!(third.attached_postings, 0);
        assert_eq!(third.new_transactions, 0);
        assert_eq!(
            postings_of(&pool, &owner.to_string()).await,
            2,
            "a further re-import adds nothing"
        );

        let batch = svcs
            .batches
            .find_by_id(&outcome.batch_id)
            .await
            .expect("the batch is still closed with its final counts");
        let counts = batch.counts.expect("a closed batch reports its counts");
        assert_eq!(counts.new_transactions, 1);
        assert_eq!(counts.attached_postings, 1);
        assert_eq!(counts.skipped(), 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_hand_added_concrete_leg_is_adopted_not_duplicated(pool: SqlitePool) {
        // Only Expenses:Food exists, so the document's Assets:Bank leg waits.
        let food = add_food(&pool).await;
        let svcs = services(&pool).await;

        let document = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );
        let first = run(&svcs, core::slice::from_ref(&document)).await;
        assert_eq!(first.new_transactions, 1);
        assert_eq!(first.unresolved_account_postings, 1);

        // The user creates the account and adds the leg themselves, exactly as
        // the warning invited them to.
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
                .amount(Amount::new(
                    Decimal::from(-50_i64),
                    CommodityCode::new("AUD"),
                ))
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
            .expect("hand-add the missing leg");

        let second = run(&svcs, core::slice::from_ref(&document)).await;
        assert_eq!(
            postings_of(&pool, &owner.to_string()).await,
            2,
            "the leg the user added must not be duplicated, which would unbalance \
             the transaction"
        );
        assert_eq!(second.new_transactions, 0, "no second transaction either");
        assert_eq!(second.attached_postings, 1);

        // Provenance now names the user's posting, so the slot is claimed.
        assert_eq!(source_count(&pool).await, 2);
        let third = run(&svcs, &[document]).await;
        assert_eq!(third.attached_postings, 0);
        assert_eq!(postings_of(&pool, &owner.to_string()).await, 2);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn editing_an_imported_leg_does_not_strand_its_siblings(pool: SqlitePool) {
        // Only Expenses:Food exists, so Assets:Bank waits for a later pass.
        let food = add_food(&pool).await;
        let svcs = services(&pool).await;

        let document = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );
        assert_eq!(
            run(&svcs, core::slice::from_ref(&document))
                .await
                .new_transactions,
            1
        );

        // The user corrects the imported leg's amount. Its posting id survives,
        // so its source reference does too — and the reference still records what
        // the *document* said, which is what the next pass matches on.
        let owner: TransactionId = owner_of_posting(&pool, &food)
            .await
            .parse()
            .expect("owning transaction id");
        let stored = svcs
            .transactions
            .find_by_id(&owner)
            .await
            .expect("stored transaction");
        let corrected: Vec<Posting> = stored
            .postings()
            .iter()
            .map(|posting| {
                Posting::builder()
                    .id(posting.id().clone())
                    .account_id(posting.account_id().clone())
                    .amount(Amount::new(
                        Decimal::from(60_i64),
                        CommodityCode::new("AUD"),
                    ))
                    .build()
            })
            .collect();
        svcs.transactions
            .edit(
                Transaction::builder()
                    .id(owner.clone())
                    .date(stored.date())
                    .description(stored.description())
                    .postings(corrected)
                    .reconciliation(stored.reconciliation())
                    .created_at(*stored.created_at())
                    .build(),
            )
            .await
            .expect("correct the amount");

        let bank = bank_only_tree(&pool).await;
        let second = run(&svcs, &[document]).await;

        assert_eq!(
            second.attached_postings, 1,
            "an edit to one leg must not strand the document's remaining legs"
        );
        assert_eq!(second.new_transactions, 0, "and must not fork a duplicate");
        assert_eq!(postings_of(&pool, &owner.to_string()).await, 2);
        assert_eq!(
            owner_of_posting(&pool, &bank).await,
            owner.to_string(),
            "the leg lands on the transaction its sibling belongs to"
        );

        // The user's correction is left alone.
        let amount: Option<String> =
            sqlx::query_scalar("SELECT amount FROM postings WHERE account_id = ?")
                .bind(food.to_string())
                .fetch_one(&pool)
                .await
                .expect("edited posting");
        assert_eq!(amount.as_deref(), Some("60"));
    }

    #[test]
    fn a_bad_row_is_skipped_and_charged_to_the_run() {
        let raw = split_raw("COFFEE");
        let mut counts = Counts::default();

        let outcome = row_local_value::<()>(
            Err(crate::BcError::BadData(
                "two or more elided postings".into(),
            )),
            &raw,
            "creating the transaction",
            2,
            &mut counts,
        )
        .expect("a bad row must not abort the run");

        assert!(outcome.is_none(), "the row is skipped");
        assert_eq!(counts.other_skipped_postings, 2, "and its legs are charged");
        assert_eq!(counts.new_transactions, 0);
    }

    #[test]
    fn a_broken_invariant_aborts_the_run() {
        let raw = split_raw("COFFEE");
        let mut counts = Counts::default();

        // `attach_in_tx` raises this when handed a reference whose posting does
        // not belong to the named transaction — this module's own invariant, not
        // a defect in the document.
        let result = row_local_value::<()>(
            Err(crate::BcError::InvalidInput(
                "posting is not on this tx".into(),
            )),
            &raw,
            "attaching the source reference",
            2,
            &mut counts,
        );

        assert!(
            matches!(result, Err(crate::BcError::InvalidInput(_))),
            "an internal invariant violation must surface, got {result:?}"
        );
        assert_eq!(
            counts.other_skipped_postings, 0,
            "and must not be absorbed into the skip count"
        );
    }

    #[test]
    fn a_successful_step_yields_its_value_and_charges_nothing() {
        let raw = split_raw("COFFEE");
        let mut counts = Counts::default();

        let outcome = row_local_value(Ok(7_u8), &raw, "creating the transaction", 2, &mut counts)
            .expect("success");

        assert_eq!(outcome, Some(7_u8));
        assert_eq!(counts.other_skipped_postings, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_duplicate_slot_is_row_local_but_a_check_violation_is_not(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let document = raw("COFFEE", -5);
        run(&svcs, core::slice::from_ref(&document)).await;

        // Re-inserting a reference over an occupied (account, fingerprint,
        // occurrence) slot is a document-level collision: row-local.
        let (id, tx_id, fingerprint): (String, String, String) = sqlx::query_as(
            "SELECT id, transaction_id, fingerprint FROM transaction_sources LIMIT 1",
        )
        .fetch_one(&pool)
        .await
        .expect("the stored reference");
        let duplicate = sqlx::query(
            "INSERT INTO transaction_sources \
             (id, transaction_id, account_id, date, narration, amount, commodity, occurrence, \
              fingerprint, created_at) \
             VALUES (?, ?, ?, '2025-06-27', 'COFFEE', '-5', 'AUD', 0, ?, '2025-06-27T00:00:00Z')",
        )
        .bind(format!("{id}x"))
        .bind(&tx_id)
        .bind(bank.to_string())
        .bind(&fingerprint)
        .execute(&pool)
        .await
        .expect_err("the unique slot key must reject this");
        assert!(
            is_row_local(&crate::BcError::from(duplicate)),
            "a slot collision must warn and skip the row"
        );

        // A half-populated amount pair violates a CHECK: the data is malformed in
        // a way no document can cause, so the run must not absorb it.
        let malformed = sqlx::query(
            "INSERT INTO transaction_sources \
             (id, transaction_id, account_id, date, narration, amount, commodity, occurrence, \
              fingerprint, created_at) \
             VALUES (?, ?, ?, '2025-06-27', 'COFFEE', '-5', NULL, 9, 'other', \
              '2025-06-27T00:00:00Z')",
        )
        .bind(format!("{id}y"))
        .bind(&tx_id)
        .bind(bank.to_string())
        .execute(&pool)
        .await
        .expect_err("the CHECK must reject an amount without a commodity");
        assert!(
            !is_row_local(&crate::BcError::from(malformed)),
            "a constraint violation that no document can cause must abort the run"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn recategorising_an_imported_leg_does_not_strand_its_siblings(pool: SqlitePool) {
        // Only Expenses:Food exists, so Assets:Bank waits for a later pass.
        let food = add_food(&pool).await;
        let svcs = services(&pool).await;

        let document = raw_with(
            "COFFEE",
            vec![
                leg("Expenses:Food", Some(50)),
                leg("Assets:Bank", Some(-50)),
            ],
        );
        assert_eq!(
            run(&svcs, core::slice::from_ref(&document))
                .await
                .new_transactions,
            1
        );

        // The user recategorises the imported leg onto a different account. The
        // posting id survives, so its reference does too, and the reference keeps
        // the account the *document* named — which is what the next pass matches.
        let dining = sibling_of(&pool, &food, "Dining", AccountType::Expense).await;
        let owner: TransactionId = owner_of_posting(&pool, &food)
            .await
            .parse()
            .expect("owning transaction id");
        let stored = svcs
            .transactions
            .find_by_id(&owner)
            .await
            .expect("stored transaction");
        let moved: Vec<Posting> = stored
            .postings()
            .iter()
            .map(|posting| {
                Posting::builder()
                    .id(posting.id().clone())
                    .account_id(dining.clone())
                    .maybe_amount(posting.amount().cloned())
                    .build()
            })
            .collect();
        svcs.transactions
            .edit(
                Transaction::builder()
                    .id(owner.clone())
                    .date(stored.date())
                    .description(stored.description())
                    .postings(moved)
                    .reconciliation(stored.reconciliation())
                    .created_at(*stored.created_at())
                    .build(),
            )
            .await
            .expect("recategorise the leg");

        let bank = bank_only_tree(&pool).await;
        let second = run(&svcs, &[document]).await;

        assert_eq!(
            second.attached_postings, 1,
            "recategorising one leg must not strand the document's remaining legs"
        );
        assert_eq!(second.new_transactions, 0, "and must not fork a duplicate");
        assert_eq!(postings_of(&pool, &owner.to_string()).await, 2);
        assert_eq!(
            owner_of_posting(&pool, &bank).await,
            owner.to_string(),
            "the leg lands on the transaction its sibling belongs to"
        );

        // The recategorisation is left alone, and no leg reappears on the account
        // the document named.
        assert_eq!(postings_of_account(&pool, &dining).await, 1);
        assert_eq!(postings_of_account(&pool, &food).await, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_deleted_leg_is_not_recreated_by_a_re_import(pool: SqlitePool) {
        let (_bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;

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
        let svcs = services(&pool).await;

        let outcome = run(&svcs, &[split_raw("SPLIT")]).await;

        let batch = svcs
            .batches
            .find_by_id(&outcome.batch_id)
            .await
            .expect("batch recorded");
        assert_eq!(batch.importer, "test");
        assert_eq!(
            batch
                .counts
                .expect("a closed batch reports its counts")
                .new_transactions,
            1
        );

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

    /// Hand-adds a posting on `account` for `amount`, mirroring the way a user
    /// responds to a partial-import warning by writing the missing leg
    /// themselves.
    ///
    /// Loads the single stored transaction — the only one an import that has
    /// run so far could have produced — rather than looking one up by
    /// posting, since `account` has no posting on it yet.
    async fn add_posting_by_hand(svcs: &Services, account: &AccountId, amount: i64) {
        let mut stored_transactions = svcs.transactions.list().await.expect("list transactions");
        let stored = stored_transactions
            .pop()
            .expect("exactly one stored transaction");
        assert!(
            stored_transactions.is_empty(),
            "add_posting_by_hand expects a single stored transaction"
        );
        let mut postings = stored.postings().to_vec();
        postings.push(
            Posting::builder()
                .id(PostingId::new())
                .account_id(account.clone())
                .amount(Amount::new(
                    Decimal::from(amount),
                    CommodityCode::new("AUD"),
                ))
                .build(),
        );
        svcs.transactions
            .edit(
                Transaction::builder()
                    .id(stored.id().clone())
                    .date(stored.date())
                    .description(stored.description())
                    .postings(postings)
                    .reconciliation(stored.reconciliation())
                    .created_at(*stored.created_at())
                    .build(),
            )
            .await
            .expect("hand-add the missing leg");
    }

    /// Every source reference in the database, in insertion order.
    async fn all_refs(pool: &SqlitePool) -> Vec<(bool, String)> {
        sqlx::query_as(
            "SELECT owns_posting, account_id FROM transaction_sources ORDER BY created_at",
        )
        .fetch_all(pool)
        .await
        .expect("query refs")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_inserted_leg_is_recorded_as_owned(pool: SqlitePool) {
        let svcs = services(&pool).await;
        two_account_tree(&pool).await;
        let raw = raw_with(
            "ACME",
            vec![
                leg("Assets:Bank", Some(-50)),
                leg("Expenses:Food", Some(50)),
            ],
        );

        run(&svcs, &[raw]).await;

        let refs = all_refs(&pool).await;
        assert_eq!(refs.len(), 2);
        assert!(
            refs.iter().all(|(owns, _)| *owns),
            "every leg this run inserted is owned by it"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_adopted_leg_is_not_recorded_as_owned(pool: SqlitePool) {
        // Pass one: only Assets:Bank exists, so the food leg is skipped and the
        // transaction lands one-sided.
        let svcs = services(&pool).await;
        bank_only_tree(&pool).await;
        let raw = raw_with(
            "ACME",
            vec![
                leg("Assets:Bank", Some(-50)),
                leg("Expenses:Food", Some(50)),
            ],
        );
        run(&svcs, core::slice::from_ref(&raw)).await;

        // The user creates the account and adds the missing leg by hand — the
        // obvious response to the partial-import warning.
        let food = add_food(&pool).await;
        add_posting_by_hand(&svcs, &food, 50).await;

        // Pass two adopts that posting rather than inserting a second one.
        run(&svcs, core::slice::from_ref(&raw)).await;

        let refs = all_refs(&pool).await;
        let adopted: Vec<&(bool, String)> = refs.iter().filter(|(owns, _)| !owns).collect();
        assert_eq!(
            adopted.len(),
            1,
            "the hand-written leg was adopted, not created"
        );
        assert_eq!(
            adopted.first().expect("one adopted reference").1,
            food.to_string()
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_wrong_run_can_be_discarded_and_redone(pool: SqlitePool) {
        let svcs = services(&pool).await;
        two_account_tree(&pool).await;

        // The wrong run: amounts sign-flipped, as an inverted convention gives.
        let wrong = vec![
            raw_with(
                "ACME",
                vec![
                    leg("Assets:Bank", Some(50)),
                    leg("Expenses:Food", Some(-50)),
                ],
            ),
            raw_with(
                "BETA",
                vec![
                    leg("Assets:Bank", Some(75)),
                    leg("Expenses:Food", Some(-75)),
                ],
            ),
        ];
        let bad = run(&svcs, &wrong).await;
        assert_eq!(bad.new_transactions, 2);

        let outcome = svcs.batches.discard(&bad.batch_id).await.expect("discard");
        assert_eq!(outcome.removed_postings, 4);
        assert_eq!(outcome.removed_transactions, 2);

        // The corrected run. These rows fingerprint differently from the wrong
        // ones, so nothing here depends on dedup — only on the wrong rows being
        // gone.
        let right = vec![
            raw_with(
                "ACME",
                vec![
                    leg("Assets:Bank", Some(-50)),
                    leg("Expenses:Food", Some(50)),
                ],
            ),
            raw_with(
                "BETA",
                vec![
                    leg("Assets:Bank", Some(-75)),
                    leg("Expenses:Food", Some(75)),
                ],
            ),
        ];
        let good = run(&svcs, &right).await;

        assert_eq!(good.new_transactions, 2);
        let remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(remaining, 2, "only the corrected rows remain");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn re_importing_the_identical_document_lands_after_a_discard(pool: SqlitePool) {
        // The sharper case: the *same* rows, so every fingerprint matches what the
        // discarded run held. Only genuinely freed slots let this import anything.
        let svcs = services(&pool).await;
        two_account_tree(&pool).await;
        let rows = vec![raw_with(
            "ACME",
            vec![
                leg("Assets:Bank", Some(-50)),
                leg("Expenses:Food", Some(50)),
            ],
        )];

        let first = run(&svcs, &rows).await;
        svcs.batches
            .discard(&first.batch_id)
            .await
            .expect("discard");
        let second = run(&svcs, &rows).await;

        assert_eq!(
            second.new_transactions, 1,
            "the freed slot lets the identical row import again"
        );
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transaction_sources")
            .fetch_one(&pool)
            .await
            .expect("count");
        assert_eq!(refs, 2, "the second run's references replaced the first's");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_freed_tombstone_slot_lets_the_leg_reappear(pool: SqlitePool) {
        // The discard module's own tests prove a tombstone's reference row is
        // gone once discarded; this proves a later import actually repopulates
        // the slot, through the real import path rather than the database
        // directly.
        let (_bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;

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

        // The user deletes the Assets:Bank leg, leaving its reference a
        // tombstone: gone as a posting, still holding its occurrence slot.
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
        svcs.transactions
            .edit(
                Transaction::builder()
                    .id(owner.clone())
                    .date(stored.date())
                    .description(stored.description())
                    .postings(kept)
                    .reconciliation(stored.reconciliation())
                    .created_at(*stored.created_at())
                    .build(),
            )
            .await
            .expect("delete the Assets:Bank leg");
        assert_eq!(posting_count(&pool).await, 1);

        // Discarding the whole run must free the tombstoned slot along with
        // the surviving leg's — not just the postings that still exist.
        svcs.batches
            .discard(&first.batch_id)
            .await
            .expect("discard");
        assert_eq!(
            source_count(&pool).await,
            0,
            "discard clears every reference the run held, tombstoned or not"
        );

        let second = run(&svcs, &[document]).await;

        assert_eq!(
            second.new_transactions, 1,
            "the freed tombstone slot lets the whole row import again"
        );
        assert_eq!(
            posting_count(&pool).await,
            2,
            "the leg the user deleted is back, imported fresh"
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_surviving_transactions_leg_is_freed_by_discard(pool: SqlitePool) {
        // The other round-trip tests all discard a batch that owns every leg
        // of its transaction(s), so the transaction ends up empty and
        // `ON DELETE CASCADE` on `transactions` sweeps its references away
        // regardless of whether discard itself hard-deleted or merely
        // tombstoned them — that cascade would paper over the exact bug this
        // feature exists to prevent. Here the transaction survives its
        // batch's discard, so nothing but a genuine hard delete can free the
        // leg's slot.
        let (_bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;

        // Batch 1: COFFEE lands as a lone, unbalanced Assets:Bank leg — not
        // owned by the batch under test.
        let solo = raw_with("COFFEE", vec![leg("Assets:Bank", Some(-50))]);
        let batch1 = run(&svcs, core::slice::from_ref(&solo)).await;
        assert_eq!(batch1.new_transactions, 1);
        assert_eq!(posting_count(&pool).await, 1);

        // Batch 2, under a wrong configuration: alongside COFFEE's own
        // Expenses:Food leg (fine on its own), RENT imports with its sign
        // backwards. Import batches are discarded as a whole, so fixing RENT
        // means redoing the batch — taking COFFEE's correct leg down with it.
        let full = raw_with(
            "COFFEE",
            vec![
                leg("Assets:Bank", Some(-50)),
                leg("Expenses:Food", Some(50)),
            ],
        );
        let sign_wrong = raw_with(
            "RENT",
            vec![
                leg("Assets:Bank", Some(120)),
                leg("Expenses:Food", Some(-120)),
            ],
        );
        let batch2 = run(&svcs, &[full.clone(), sign_wrong]).await;
        assert_eq!(
            batch2.new_transactions, 1,
            "RENT lands as its own (wrong) transaction"
        );
        assert_eq!(
            batch2.attached_postings, 1,
            "COFFEE's Food leg attaches to the transaction batch 1 started"
        );
        assert_eq!(posting_count(&pool).await, 4);

        let outcome = svcs
            .batches
            .discard(&batch2.batch_id)
            .await
            .expect("discard");
        assert_eq!(
            outcome.removed_postings, 3,
            "COFFEE's Food leg and both of RENT's legs were batch 2's own"
        );
        assert_eq!(
            outcome.removed_transactions, 1,
            "RENT's transaction is left with nothing and is swept"
        );
        assert_eq!(
            posting_count(&pool).await,
            1,
            "only batch 1's Assets:Bank leg on COFFEE remains"
        );

        // Corrected configuration: COFFEE is resubmitted byte-for-byte
        // unchanged (its Food leg was never wrong; the batch-wide discard
        // simply wiped its reference), RENT with its sign fixed.
        let sign_right = raw_with(
            "RENT",
            vec![
                leg("Assets:Bank", Some(-120)),
                leg("Expenses:Food", Some(120)),
            ],
        );
        let batch3 = run(&svcs, &[full, sign_right]).await;

        assert_eq!(
            batch3.attached_postings, 1,
            "freeing batch 2's slot lets COFFEE's Food leg attach again"
        );
        assert_eq!(
            batch3.new_transactions, 1,
            "RENT imports cleanly under the corrected sign"
        );
        assert_eq!(
            posting_count(&pool).await,
            4,
            "COFFEE (2 legs) and the corrected RENT (2 legs)"
        );
        assert_eq!(
            postings_of_account(&pool, &food).await,
            2,
            "COFFEE's and RENT's Food legs both landed"
        );
    }

    // MARK: Commodity resolution

    /// Creates every segment of `path` that does not already exist, returning
    /// the leaf. Idempotent, so a test may re-run an import over the same tree.
    async fn ensure_path(pool: &SqlitePool, path: &str) -> AccountId {
        let svc = crate::AccountService::new(pool.clone());
        let account_type = match path.split(':').next() {
            Some("Liabilities") => AccountType::Liability,
            Some("Income") => AccountType::Income,
            Some("Expenses") => AccountType::Expense,
            Some("Equity") => AccountType::Equity,
            _ => AccountType::Asset,
        };
        let mut parent: Option<AccountId> = None;
        for segment in path.split(':') {
            let existing: Option<String> =
                sqlx::query_scalar("SELECT id FROM accounts WHERE name = ? AND parent_id IS ?")
                    .bind(segment)
                    .bind(parent.as_ref().map(ToString::to_string))
                    .fetch_optional(pool)
                    .await
                    .expect("look up an account by parent and name");
            parent = Some(match existing {
                Some(id) => id.parse::<AccountId>().expect("a stored account id"),
                None => svc
                    .create()
                    .name(segment)
                    .account_type(account_type)
                    .kind(AccountKind::DepositAccount)
                    .maybe_parent_id(parent.as_ref())
                    .call()
                    .await
                    .expect("create the account"),
            });
        }
        parent.expect("a non-empty path names at least one account")
    }

    /// A one-leg transaction on `account`, stating `amount` in `code`.
    fn coded_leg(account: &str, amount: Decimal, code: &str) -> RawPosting {
        RawPosting::builder()
            .account(account)
            .maybe_amount(Some(Amount::new(amount, CommodityCode::new(code))))
            .build()
    }

    /// Imports one row holding a single leg on `account` for `amount` `code`.
    async fn import_one_leg(
        pool: &SqlitePool,
        account: &str,
        amount: Decimal,
        code: &str,
    ) -> ImportOutcome {
        ensure_path(pool, account).await;
        let svcs = services(pool).await;
        let row = raw_with("PAYMENT", vec![coded_leg(account, amount, code)]);
        run(&svcs, &[row]).await
    }

    /// Imports `rows` distinct one-leg rows, every one naming `code`.
    async fn import_n_legs(
        pool: &SqlitePool,
        account: &str,
        code: &str,
        rows: usize,
    ) -> ImportOutcome {
        ensure_path(pool, account).await;
        let svcs = services(pool).await;
        let batch: Vec<RawTransaction> = (0..rows)
            .map(|i| {
                raw_with(
                    &format!("PAYMENT {i}"),
                    vec![coded_leg(account, Decimal::from(5_i64), code)],
                )
            })
            .collect();
        run(&svcs, &batch).await
    }

    /// Imports one row holding a leg on each of `first` and `second`, each
    /// stated in its own commodity code.
    async fn import_two_legs(
        pool: &SqlitePool,
        first: (&str, &str),
        second: (&str, &str),
    ) -> ImportOutcome {
        ensure_path(pool, first.0).await;
        ensure_path(pool, second.0).await;
        let svcs = services(pool).await;
        let row = raw_with(
            "SPLIT",
            vec![
                coded_leg(first.0, Decimal::from(5_i64), first.1),
                coded_leg(second.0, Decimal::from(-5_i64), second.1),
            ],
        );
        run(&svcs, &[row]).await
    }

    /// Imports one row whose single leg states its amount in `codes.0` and the
    /// running balance the source reported in `codes.1`.
    async fn import_leg_with_balance(
        pool: &SqlitePool,
        account: &str,
        codes: (&str, &str),
    ) -> ImportOutcome {
        ensure_path(pool, account).await;
        let svcs = services(pool).await;
        let leg = RawPosting::builder()
            .account(account)
            .maybe_amount(Some(Amount::new(
                Decimal::from(5_i64),
                CommodityCode::new(codes.0),
            )))
            .maybe_balance(Some(Amount::new(
                Decimal::from(100_i64),
                CommodityCode::new(codes.1),
            )))
            .build();
        run(&svcs, &[raw_with("PAYMENT", vec![leg])]).await
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn unregistered_commodity_skips_the_leg_and_reports_it(pool: sqlx::SqlitePool) {
        // Registry holds only the seeded defaults; DOGE is not among them.
        let outcome = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "DOGE").await;
        assert_eq!(outcome.unresolved_commodity_postings, 1);
        assert_eq!(outcome.unresolved_commodities, vec!["DOGE".to_owned()]);
        assert_eq!(outcome.new_transactions, 0);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_lower_case_code_resolves_and_stores_the_registered_spelling(pool: sqlx::SqlitePool) {
        let outcome = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "aud").await;
        assert_eq!(outcome.unresolved_commodity_postings, 0);
        let stored: String = sqlx::query_scalar("SELECT commodity FROM postings")
            .fetch_one(&pool)
            .await
            .expect("one posting");
        assert_eq!(stored, "AUD");
    }

    /// A code padded with whitespace is not "blank" and must still resolve —
    /// otherwise it is reported as unregistered under a code the user cannot
    /// usefully register (registering `AUD` would not fix `" AUD "`).
    #[sqlx::test(migrations = "./migrations")]
    async fn a_padded_code_resolves_to_the_canonical_code(pool: sqlx::SqlitePool) {
        let outcome = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), " AUD ").await;
        assert_eq!(outcome.unresolved_commodity_postings, 0);
        assert!(outcome.unresolved_commodities.is_empty());
        let stored: String = sqlx::query_scalar("SELECT commodity FROM postings")
            .fetch_one(&pool)
            .await
            .expect("one posting");
        assert_eq!(stored, "AUD");
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_alias_resolves_to_the_canonical_code(pool: sqlx::SqlitePool) {
        let outcome = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "AU$").await;
        assert_eq!(outcome.unresolved_commodity_postings, 0);
        let stored: String = sqlx::query_scalar("SELECT commodity FROM postings")
            .fetch_one(&pool)
            .await
            .expect("one posting");
        assert_eq!(stored, "AUD");
    }

    /// One unregistered code named by many rows warns once and is listed once,
    /// exactly as an unknown account path already is.
    #[sqlx::test(migrations = "./migrations")]
    async fn one_unregistered_code_is_listed_once_however_many_rows_name_it(
        pool: sqlx::SqlitePool,
    ) {
        let outcome = import_n_legs(&pool, "Assets:Bank:Checking", "DOGE", 5).await;
        assert_eq!(outcome.unresolved_commodity_postings, 5);
        assert_eq!(outcome.unresolved_commodities, vec!["DOGE".to_owned()]);
    }

    /// A sibling leg that resolves still persists; the run is not lost.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_resolvable_sibling_leg_persists(pool: sqlx::SqlitePool) {
        let outcome = import_two_legs(
            &pool,
            ("Assets:Bank:Checking", "AUD"),
            ("Assets:Crypto:Wallet", "DOGE"),
        )
        .await;
        assert_eq!(outcome.unresolved_commodity_postings, 1);
        assert_eq!(
            outcome
                .attached_postings
                .saturating_add(outcome.new_transactions),
            1
        );
    }

    /// Registering the commodity and re-running attaches what was skipped —
    /// the same recovery path an unknown account already has.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_skipped_leg_attaches_after_the_commodity_is_registered(pool: sqlx::SqlitePool) {
        let first = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "DOGE").await;
        assert_eq!(first.unresolved_commodity_postings, 1);

        crate::CommodityService::new(pool.clone())
            .create(
                &bc_models::Commodity::builder()
                    .code("DOGE")
                    .decimals(8)
                    .is_iso(false)
                    .build(),
            )
            .await
            .expect("register DOGE");

        let second = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "DOGE").await;
        assert_eq!(second.unresolved_commodity_postings, 0);
        assert_eq!(second.new_transactions, 1);
    }

    /// The dedup fingerprint is computed over the *canonical* code, so the same
    /// posting stated `aud` in one file and `AUD` in the next dedups rather
    /// than importing twice. Fingerprinting the raw code would silently
    /// duplicate every posting whose file changed the spelling of its currency.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_re_import_spelling_the_code_differently_dedups(pool: sqlx::SqlitePool) {
        let first = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "aud").await;
        assert_eq!(first.new_transactions, 1);

        let second = import_one_leg(&pool, "Assets:Bank:Checking", dec!(5), "AUD").await;
        assert_eq!(
            second.new_transactions, 0,
            "the same posting spelled AUD rather than aud is not a new transaction"
        );
        assert_eq!(second.attached_postings, 0);
        assert_eq!(
            posting_count(&pool).await,
            1,
            "one posting, not one per spelling"
        );
    }

    /// A balance is corroboration, not the posting: an unresolved commodity on
    /// it costs the balance, not the leg.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_unresolved_balance_commodity_drops_the_balance_not_the_leg(pool: sqlx::SqlitePool) {
        let outcome = import_leg_with_balance(&pool, "Assets:Bank:Checking", ("AUD", "DOGE")).await;
        assert_eq!(outcome.unresolved_commodity_postings, 0);
        assert_eq!(outcome.new_transactions, 1);
    }
}
