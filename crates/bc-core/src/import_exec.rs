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

use core::future::Future;
use core::future::ready;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::collections::HashSet;

use bc_models::AccountId;
use bc_models::Amount;
use bc_models::Balances;
use bc_models::CommodityCode;
use bc_models::ImportBatchId;
use bc_models::MetaEntry;
use bc_models::MetaKey;
use bc_models::MetaValue;
use bc_models::Metadata;
use bc_models::Posting;
use bc_models::PostingId;
use bc_models::Reconciliation;
use bc_models::SourceRef;
use bc_models::SourceRefId;
use bc_models::TagId;
use bc_models::TagPath;
use bc_models::Transaction;
use bc_models::TransactionId;
use jiff::Timestamp;

use crate::AccountPath;
use crate::AccountResolver;
use crate::BcResult;
use crate::CommodityResolver;
use crate::RawMetaEntry;
use crate::RawMetaValue;
use crate::RawPosting;
use crate::RawTransaction;
use crate::Resolution;
use crate::StoredLeg;
use crate::Warning;

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
    /// Tag paths this run created, sorted. Tags name no balance, so an unknown
    /// one is created rather than skipped; this is the list that makes a typo
    /// visible, since a wrong tag is cheap to rename or delete but an omitted
    /// one is expensive to reconstruct.
    pub created_tags: Vec<String>,
    /// Postings charged to each [`SkipCause`] encountered, in the cause's
    /// declaration order. The three coarse buckets above group causes into
    /// `unresolved_account`/`unresolved_commodity`/`other`; this keeps every
    /// cause distinct.
    ///
    /// This is not the same as counting [`Self::diagnostics`] per cause: one
    /// diagnostic can be noted for a row while charging several of its
    /// postings (a [`SkipCause::MultiOwnerConflict`] row loses every leg that
    /// failed to match, noted once), and [`SkipCause::MalformedTag`] is noted
    /// without charging any posting at all, since the tag is dropped but the
    /// leg still persists. A cause with no charge does not appear here.
    pub charged_by_cause: Vec<(SkipCause, usize)>,
    /// Every leg or row this run could not persist, in encounter order, with
    /// the cause and the document location. The counts above are the totals of
    /// these; this is the per-row detail behind them.
    ///
    /// Granularity differs by cause. A leg or row diagnostic is recorded per
    /// occurrence, so two rows naming one missing account yield two entries.
    /// A [`SkipCause::MalformedTag`] is recorded per distinct spelling, because
    /// tag paths are deduplicated before they are parsed: one bad tag named by
    /// two hundred rows yields a single entry, carrying the first row's
    /// location. Such an entry costs no posting and so appears in no count.
    pub diagnostics: Vec<Diagnostic>,
    /// Advisory warnings raised by postings that were nonetheless written: a
    /// commodity outside an account's declared list, a date outside its
    /// declared life, or an archived account.
    ///
    /// Deliberately separate from [`Self::diagnostics`] and
    /// [`Self::charged_by_cause`], which account for postings that were thrown
    /// away and whose counts must keep summing to [`Self::skipped_postings`].
    pub warnings: Vec<Warning>,
}

/// What an import run **would** do, computed without writing anything.
///
/// Every field up to [`Self::unresolved_commodities`] mirrors the
/// [`ImportOutcome`] field of the same name, and [`Self::would_create_tags`]
/// mirrors [`ImportOutcome::created_tags`]. That correspondence is asserted by
/// the crate's equivalence tests: a plan and the run it predicts walk identical
/// branches, because no decision in a run observes a write the run made.
///
/// The absent field is the batch: a dry run opens none.
///
/// # Limits
///
/// A plan predicts decisions, not writes. If a real run's *insert* fails, those
/// legs are charged to [`Self::other_skipped_postings`] under
/// [`SkipCause::RowLocalFailure`], and no plan will have predicted it: the sink
/// is the one step a plan does not perform. The report therefore describes what
/// the run would do **absent a write failure**.
///
/// [`SkipCause::RowLocalFailure`] can still appear here, because the steps
/// *above* the sink are shared and two of them raise it: a row whose amounts
/// overflow when summed, and an unreadable stored transaction met while
/// matching an owner. Both are decisions a real run would reach the same way,
/// so a plan reporting one is predicting the run rather than diverging from it.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportPlan {
    /// Transactions the run would create.
    pub new_transactions: usize,
    /// Legs it would book onto transactions an earlier run created.
    ///
    /// Counts legs *adopted* as well as legs inserted, exactly as
    /// [`ImportOutcome::attached_postings`] does.
    pub attached_postings: usize,
    /// Postings it could not persist, whatever the cause. The sum of
    /// [`Self::unresolved_account_postings`],
    /// [`Self::unresolved_commodity_postings`] and
    /// [`Self::other_skipped_postings`].
    pub skipped_postings: usize,
    /// Postings whose account path names no existing account.
    pub unresolved_account_postings: usize,
    /// Postings whose commodity code names no registered commodity.
    pub unresolved_commodity_postings: usize,
    /// Postings skipped for any other reason — a malformed account path, a
    /// blank commodity code, an ambiguous residual, legs owned by several
    /// transactions, or a candidate that failed to corroborate.
    pub other_skipped_postings: usize,
    /// Account paths that resolve to no account, deduplicated and sorted.
    ///
    /// This is the actionable output: create these accounts and re-run.
    pub unresolved_accounts: Vec<String>,
    /// The distinct unregistered codes encountered, sorted.
    pub unresolved_commodities: Vec<String>,
    /// Tag paths the run would create, sorted. A path already in the tag tree
    /// is not listed, so this is the typo-spotting list rather than the full
    /// set of tags the document names.
    pub would_create_tags: Vec<String>,
    /// Per-account sums of the legs that would post, keyed by rendered account
    /// path and sorted by it. Multi-commodity by construction: an account
    /// touched in two commodities holds a bucket for each.
    ///
    /// A leg that elides its amount still moves its account, because the
    /// balance engine derives its value from its siblings; the residual is
    /// derived here through the same function the read path uses, so this
    /// figure tracks the balance a real run would leave behind. That holds for
    /// a leg appended to an earlier run's transaction as much as for one on a
    /// transaction this run would create, since the residual is taken over the
    /// transaction's whole leg set either way.
    ///
    /// An account can therefore appear here having been booked no posting at
    /// all: appending a leg to a transaction that already holds an elided one
    /// changes what that elided leg derives, moving its account. Such an entry
    /// carries the movement, not the resulting balance, exactly as every other
    /// entry does.
    ///
    /// Two elided legs on one transaction are the exception, and not one this
    /// report invents: the balance engine attributes such a residual to neither
    /// leg, so neither bucket moves here.
    ///
    /// An account whose legs net to zero still holds a bucket — an empty one —
    /// so the report can say the account was touched rather than omitting it.
    pub account_totals: Vec<(String, Balances)>,
    /// Postings that would be charged to each [`SkipCause`] encountered, in
    /// the cause's declaration order. Mirrors
    /// [`ImportOutcome::charged_by_cause`]; see that field for why this is
    /// not the same as counting [`Self::diagnostics`] per cause.
    pub charged_by_cause: Vec<(SkipCause, usize)>,
    /// Every leg or row the run would skip, in encounter order, with the cause
    /// and the document location. The counts above are the totals of these.
    pub diagnostics: Vec<Diagnostic>,
    /// Advisory warnings raised by postings that would nonetheless be written: a
    /// commodity outside an account's declared list, a date outside its
    /// declared life, or an archived account.
    ///
    /// Deliberately separate from [`Self::diagnostics`] and
    /// [`Self::charged_by_cause`], which account for postings that were thrown
    /// away and whose counts must keep summing to [`Self::skipped_postings`].
    pub warnings: Vec<Warning>,
}

/// Adds `postings` to `tally`'s entry for `cause`, creating it if this is the
/// first charge against that cause.
///
/// Shared between [`Counts::charge`] and [`resolve_legs`], which is the other
/// place a leg is charged to a cause — account and commodity resolution
/// charge as they go, before a [`Counts`] even exists.
///
/// # Arguments
///
/// * `tally` - The per-cause tally to update.
/// * `cause` - Why the postings were charged.
/// * `postings` - How many to add.
fn charge_cause(tally: &mut BTreeMap<SkipCause, usize>, cause: SkipCause, postings: usize) {
    tally
        .entry(cause)
        .and_modify(|charged| *charged = charged.saturating_add(postings))
        .or_insert(postings);
}

/// Running totals and diagnostics for one import run, with skips attributed to
/// their cause.
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
    /// Postings charged to each cause, keyed by the cause itself rather than
    /// the three coarse buckets above. `charge` and `note` are independent —
    /// one note can precede a charge of several postings, or precede none at
    /// all — so this cannot be recovered by counting diagnostics per cause.
    charged_by_cause: BTreeMap<SkipCause, usize>,
    /// Every leg or row this run could not persist, in encounter order.
    diagnostics: Vec<Diagnostic>,
    /// Advisory warnings raised by postings that were nonetheless written,
    /// deliberately kept apart from `diagnostics` and `charged_by_cause`. See
    /// [`ImportOutcome::warnings`] for why.
    warnings: Vec<Warning>,
    /// `(variant, account)` pairs already represented in `warnings`, so a run
    /// over many rows against one account reports each finding once rather
    /// than once per posting. Mirrors the warn-once precedent
    /// [`resolve_leg`] already set for [`Warning::PostingIntoArchivedAccount`]
    /// — extended here to the other three variants, which `check_postings`
    /// otherwise raises afresh for every posting it checks.
    warned_seen: HashSet<WarningKey>,
}

impl Counts {
    /// Records one unpersistable leg or row without charging it.
    ///
    /// # Arguments
    ///
    /// * `location` - Where the document says it came from.
    /// * `cause` - Why it could not be persisted.
    /// * `detail` - The offending path, code, or conflict.
    fn note(&mut self, location: &str, cause: SkipCause, detail: impl Into<String>) {
        self.diagnostics.push(Diagnostic {
            location: location.to_owned(),
            cause,
            detail: detail.into(),
        });
    }

    /// Charges `postings` to the tally `cause` belongs to, and to the cause's
    /// own line.
    ///
    /// Every charge in the run passes through here, so the coarse split and the
    /// per-cause breakdown are incremented together and cannot disagree about
    /// how much was lost. Which coarse column a cause lands in is
    /// [`Bucket::of`]'s decision alone.
    ///
    /// # Arguments
    ///
    /// * `cause` - Why the postings were skipped.
    /// * `postings` - How many were lost.
    fn charge(&mut self, cause: SkipCause, postings: usize) {
        let bucket = match Bucket::of(cause) {
            Bucket::UnresolvedAccount => &mut self.unresolved_account_postings,
            Bucket::UnresolvedCommodity => &mut self.unresolved_commodity_postings,
            Bucket::Other => &mut self.other_skipped_postings,
        };
        *bucket = bucket.saturating_add(postings);

        charge_cause(&mut self.charged_by_cause, cause, postings);
    }

    /// Returns the total skipped, whatever the cause.
    fn skipped(&self) -> usize {
        self.unresolved_account_postings
            .saturating_add(self.unresolved_commodity_postings)
            .saturating_add(self.other_skipped_postings)
    }

    /// Adds the write-time guard's warnings for one row, deduplicated to at
    /// most one per `(variant, account)` for the whole run.
    ///
    /// [`Warning::PostingIntoArchivedAccount`] is dropped outright: the
    /// resolution pass already raised its own warn-once version of that
    /// finding into `self.warnings` before any row was written (see
    /// [`resolve_leg`]), so a second copy from the write-time guard would
    /// double up. The other three variants get no such treatment upstream —
    /// `check_postings` raises one per posting it checks — so without this
    /// dedup, importing thousands of postings against one closed or
    /// out-of-list account would report thousands of identical lines.
    ///
    /// # Arguments
    ///
    /// * `warnings` - The warnings one row's write raised.
    fn push_warnings(&mut self, warnings: Vec<Warning>) {
        for warning in warnings {
            if matches!(warning, Warning::PostingIntoArchivedAccount { .. }) {
                continue;
            }
            if let Some(key) = WarningKey::of(&warning)
                && !self.warned_seen.insert(key)
            {
                continue;
            }
            self.warnings.push(warning);
        }
    }
}

/// A [`Warning`] variant paired with the account it names, used to dedup
/// warnings that `check_postings` otherwise raises once per posting. The
/// account-life variants collapse to one per account for the whole run; the
/// commodity variant also keys on the code, so each distinct undeclared
/// commodity is still reported once.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum WarningKey {
    /// Keys a [`Warning::CommodityOutsideAccountList`] by account *and* code:
    /// two undeclared codes in one account are two distinct facts, and
    /// collapsing them would report the first and hide the second.
    CommodityOutsideAccountList(AccountId, String),
    /// Keys a [`Warning::PostingBeforeAccountOpened`].
    PostingBeforeAccountOpened(AccountId),
    /// Keys a [`Warning::PostingAfterAccountClosed`].
    PostingAfterAccountClosed(AccountId),
}

impl WarningKey {
    /// Returns the dedup key for `warning`, or `None` for a variant this
    /// module does not dedup at collection (currently just
    /// [`Warning::PostingIntoArchivedAccount`], deduped upstream instead).
    fn of(warning: &Warning) -> Option<Self> {
        match *warning {
            Warning::CommodityOutsideAccountList {
                ref account_id,
                ref commodity_code,
                ..
            } => Some(Self::CommodityOutsideAccountList(
                account_id.clone(),
                commodity_code.clone(),
            )),
            Warning::PostingBeforeAccountOpened { ref account_id, .. } => {
                Some(Self::PostingBeforeAccountOpened(account_id.clone()))
            }
            Warning::PostingAfterAccountClosed { ref account_id, .. } => {
                Some(Self::PostingAfterAccountClosed(account_id.clone()))
            }
            Warning::PostingIntoArchivedAccount { .. } => None,
        }
    }
}

/// The coarse column a [`SkipCause`] is charged to.
///
/// The report leads with this three-way split, so which column a cause lands in
/// is decided once, here, rather than at each site that keeps a tally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bucket {
    /// The leg's account path named no existing account.
    UnresolvedAccount,
    /// The leg's commodity code named no registered commodity.
    UnresolvedCommodity,
    /// Anything else.
    Other,
}

impl Bucket {
    /// Returns the column `cause` is charged to.
    ///
    /// Exhaustive by design: a new [`SkipCause`] fails to compile here until
    /// someone says which column it belongs in.
    ///
    /// # Arguments
    ///
    /// * `cause` - Why the postings were skipped.
    ///
    /// # Returns
    ///
    /// The coarse column, which is the only thing that decides it.
    fn of(cause: SkipCause) -> Self {
        match cause {
            SkipCause::UnresolvedAccount => Self::UnresolvedAccount,
            SkipCause::UnresolvedCommodity => Self::UnresolvedCommodity,
            SkipCause::MalformedPath
            | SkipCause::MalformedTag
            | SkipCause::BlankCommodity
            | SkipCause::AmbiguousResidual
            | SkipCause::UndeterminedResidual
            | SkipCause::MultiOwnerConflict
            | SkipCause::FailedCorroboration
            | SkipCause::RowLocalFailure => Self::Other,
        }
    }
}

/// Why one leg — or one whole transaction — could not be persisted this run.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SkipCause {
    /// The leg's account path named no existing account. Creating the account
    /// and re-running attaches the leg.
    UnresolvedAccount,
    /// The leg's commodity code named no registered commodity. Registering the
    /// commodity and re-running attaches the leg.
    UnresolvedCommodity,
    /// The leg's account path could not be parsed.
    MalformedPath,
    /// A tag path stated by the row or one of its legs could not be parsed. The
    /// tag is dropped and the leg still persists, so this costs no posting.
    MalformedTag,
    /// The leg stated an amount with no commodity code.
    BlankCommodity,
    /// Two or more legs elide their amount, so the residual is undetermined.
    AmbiguousResidual,
    /// The elided leg is the only one that resolved, and the document fixes no
    /// single amount for it.
    UndeterminedResidual,
    /// The legs of one document transaction already belong to several
    /// transactions.
    MultiOwnerConflict,
    /// A posting of the matched transaction is not explained by this document
    /// transaction.
    FailedCorroboration,
    /// A step failed in a way charged to the row rather than the run — the
    /// write itself, the summing of the row's amounts, or reading the
    /// transaction a leg would attach to.
    ///
    /// A dry run cannot report the write, which is the one step it does not
    /// perform, but it reports the other two: they sit above the sink and a
    /// real run reaches them identically.
    RowLocalFailure,
}

impl SkipCause {
    /// Returns the stable, lower-case label this cause groups under in a report.
    ///
    /// # Returns
    ///
    /// A short noun phrase such as `unresolved account`.
    #[must_use]
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::UnresolvedAccount => "unresolved account",
            Self::UnresolvedCommodity => "unregistered commodity",
            Self::MalformedPath => "malformed account path",
            Self::MalformedTag => "malformed tag path",
            Self::BlankCommodity => "blank commodity code",
            Self::AmbiguousResidual => "ambiguous residual",
            Self::UndeterminedResidual => "undetermined residual",
            Self::MultiOwnerConflict => "multi-owner conflict",
            Self::FailedCorroboration => "failed corroboration",
            Self::RowLocalFailure => "write failure",
        }
    }
}

/// One leg or row the run could not persist, and why.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// Where the document says this came from, as `location_of` renders it.
    pub location: String,
    /// Why it was skipped.
    pub cause: SkipCause,
    /// Human-readable detail: the offending path, code, or conflict.
    pub detail: String,
}

/// One leg whose account path resolved to an existing account.
#[derive(Debug, Clone)]
struct ResolvedLeg {
    /// The account the leg's path named.
    account_id: AccountId,
    /// The rendered account path this leg resolved through.
    account_path: String,
    /// The leg's amount as the document stated it; `None` for the elided residual.
    amount: Option<Amount>,
    /// The leg's dedup fingerprint, over the document's own values.
    fingerprint: String,
    /// The leg's metadata, with every account path the document stated bound.
    metadata: Metadata,
    /// The leg's tag paths, as the document stated them.
    tag_paths: Vec<String>,
}

/// A resolved leg together with the occurrence slot it claims for this run.
#[derive(Debug, Clone)]
struct LegPlan {
    /// The account the leg's path named.
    account_id: AccountId,
    /// The rendered account path this leg resolved through.
    account_path: String,
    /// The leg's amount as the document stated it; `None` for the elided residual.
    amount: Option<Amount>,
    /// The leg's dedup fingerprint, over the document's own values.
    fingerprint: String,
    /// The leg's metadata, with every account path the document stated bound.
    metadata: Metadata,
    /// The leg's tag paths, as the document stated them.
    tag_paths: Vec<String>,
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
    /// * `tags` - Rendered-path → tag-ID map from the run's pre-pass. A path
    ///   absent from it failed to parse and was already warned about, so it is
    ///   dropped here rather than costing the leg.
    ///
    /// # Returns
    ///
    /// A freshly-identified [`Posting`] on this leg's account.
    fn posting(&self, residual: Option<&Amount>, tags: &HashMap<String, TagId>) -> Posting {
        Posting::builder()
            .id(PostingId::new())
            .account_id(self.account_id.clone())
            .maybe_amount(self.amount.clone().or_else(|| residual.cloned()))
            .metadata(self.metadata.clone())
            .tag_ids(resolve_tag_ids(&self.tag_paths, tags))
            .build()
    }
}

/// Maps stated tag paths onto the IDs the run's pre-pass created.
///
/// # Arguments
///
/// * `stated` - Tag paths as the document wrote them.
/// * `tags` - Rendered-path → tag-ID map from the pre-pass.
///
/// # Returns
///
/// The IDs of the paths that parsed, deduplicated, in first-seen order. A path
/// the map does not hold failed to parse and was warned about there.
fn resolve_tag_ids(stated: &[String], tags: &HashMap<String, TagId>) -> Vec<TagId> {
    let mut out: Vec<TagId> = Vec::new();
    for path in stated {
        if let Some(id) = tags.get(path)
            && !out.contains(id)
        {
            out.push(id.clone());
        }
    }
    out
}

/// Tag paths materialised for one run, with the paths that failed to parse.
struct Tags {
    /// Leaf tag ID for every path that parsed, keyed by its rendered form.
    ids: HashMap<String, TagId>,
    /// The rendered paths this run brought into existence, sorted.
    created: Vec<String>,
    /// The paths that failed to parse and were dropped.
    diagnostics: Vec<Diagnostic>,
}

/// The outcome of the resolution pass over every leg of every raw transaction.
struct Resolved {
    /// Resolved legs per raw transaction, index-aligned with the input slice.
    ///
    /// An entry is shorter than its raw transaction's posting list when some leg
    /// was skipped, and empty when the whole transaction was.
    rows: Vec<Vec<ResolvedLeg>>,
    /// Distinct account paths naming no account; sorted and unique by construction.
    unresolved_accounts: BTreeSet<String>,
    /// Distinct codes naming no registered commodity; sorted and unique by
    /// construction.
    unresolved_commodities: BTreeSet<String>,
    /// The pass's charges and diagnostics, kept in the run's own tally type so
    /// attribution is [`Counts::charge`]'s decision here as everywhere else.
    /// The run continues accumulating into it rather than copying it out.
    counts: Counts,
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
/// * `tags` - Tag service, used to materialise every path the run names.
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
    tags: &crate::TagService,
    batches: &crate::ImportBatchService,
    profile_id: Option<&bc_models::ProfileId>,
    importer: &str,
    raws: &[RawTransaction],
) -> BcResult<ImportOutcome> {
    let mut sink = Commit {
        transactions,
        sources,
        batch_id: None,
    };
    let run = run_with(
        &mut sink,
        transactions,
        sources,
        accounts,
        commodities,
        tags,
        batches,
        profile_id,
        importer,
        raws,
    )
    .await?;

    let batch_id = run
        .batch_id
        .ok_or_else(|| crate::BcError::BadData("the commit sink opened no batch".to_owned()))?;

    Ok(ImportOutcome {
        batch_id,
        new_transactions: run.counts.new_transactions,
        attached_postings: run.counts.attached_postings,
        skipped_postings: run.counts.skipped(),
        unresolved_account_postings: run.counts.unresolved_account_postings,
        unresolved_commodity_postings: run.counts.unresolved_commodity_postings,
        other_skipped_postings: run.counts.other_skipped_postings,
        unresolved_accounts: run.unresolved_accounts,
        unresolved_commodities: run.unresolved_commodities,
        created_tags: run.created_tags,
        charged_by_cause: run.counts.charged_by_cause.into_iter().collect(),
        diagnostics: run.counts.diagnostics,
        warnings: run.counts.warnings,
    })
}

/// Reports what [`execute_import`] would do, without writing anything.
///
/// This is not a separate resolution pass: it is the same run, with the
/// terminal writes diverted into a sink that only tallies. Every branch —
/// account and commodity resolution, occurrence allocation, owner matching,
/// corroboration, residual materialisation — is the code a real run takes, so a
/// plan cannot disagree with the run it predicts. What makes that safe is that
/// the stored legs are read once before any write, so no decision in a run
/// observes a write that run made.
///
/// Tag paths are resolved rather than created, so a plan leaves the tag tree
/// untouched and reports the paths a real run would bring into existence.
///
/// # Limits
///
/// A plan predicts decisions, not writes. A real run whose insert fails charges
/// those legs to [`ImportOutcome::other_skipped_postings`] under
/// [`SkipCause::RowLocalFailure`]; no plan predicts that, the sink being the one
/// step it does not perform. The report describes what the run would do absent a
/// write failure. See [`ImportPlan`] for the row-local failures a plan *does*
/// report, which arise above the sink and so are shared with a real run.
///
/// # Arguments
///
/// * `transactions` - Transaction service, read for the pool the run shares.
/// * `sources` - Source-reference service, snapshotted for the stored legs.
/// * `accounts` - Account service, snapshotted once for path resolution.
/// * `commodities` - Commodity service, snapshotted once for code resolution.
/// * `tags` - Tag service, used to resolve every path the run names.
/// * `batches` - Import batch provenance service; a plan opens no batch.
/// * `profile_id` - The driving profile, if the run is profile-driven.
/// * `importer` - Stable importer name a real run would record on its batch.
/// * `raws` - Parsed transactions in document order.
///
/// # Returns
///
/// An [`ImportPlan`] summarising what would be written and what would be
/// skipped.
///
/// # Errors
///
/// Returns [`crate::BcError`] on query failure.
#[expect(
    clippy::too_many_arguments,
    reason = "the same parameter list as `execute_import`, which it must mirror \
              exactly for a caller to swap one for the other"
)]
#[inline]
pub async fn plan_import(
    transactions: &crate::TransactionService,
    sources: &crate::SourceService,
    accounts: &crate::AccountService,
    commodities: &crate::CommodityService,
    tags: &crate::TagService,
    batches: &crate::ImportBatchService,
    profile_id: Option<&bc_models::ProfileId>,
    importer: &str,
    raws: &[RawTransaction],
) -> BcResult<ImportPlan> {
    let mut sink = Plan::default();
    let run = run_with(
        &mut sink,
        transactions,
        sources,
        accounts,
        commodities,
        tags,
        batches,
        profile_id,
        importer,
        raws,
    )
    .await?;

    Ok(ImportPlan {
        new_transactions: run.counts.new_transactions,
        attached_postings: run.counts.attached_postings,
        skipped_postings: run.counts.skipped(),
        unresolved_account_postings: run.counts.unresolved_account_postings,
        unresolved_commodity_postings: run.counts.unresolved_commodity_postings,
        other_skipped_postings: run.counts.other_skipped_postings,
        unresolved_accounts: run.unresolved_accounts,
        unresolved_commodities: run.unresolved_commodities,
        would_create_tags: run.created_tags,
        account_totals: sink.totals.into_iter().collect(),
        charged_by_cause: run.counts.charged_by_cause.into_iter().collect(),
        diagnostics: run.counts.diagnostics,
        warnings: run.counts.warnings,
    })
}

/// What one import run produced, whatever sink it wrote through.
struct Run {
    /// The batch the sink opened, if it opened one at all.
    batch_id: Option<ImportBatchId>,
    /// The run's totals and the diagnostics behind them.
    counts: Counts,
    /// Tag paths the sink brought into existence, sorted.
    created_tags: Vec<String>,
    /// Distinct account paths naming no account, sorted.
    unresolved_accounts: Vec<String>,
    /// Distinct codes naming no registered commodity, sorted.
    unresolved_commodities: Vec<String>,
}

/// Runs the whole import pipeline, sending every write to `sink`.
///
/// Every decision — resolution, occurrence allocation, owner matching,
/// corroboration, residual materialisation — happens here and is therefore
/// shared by every sink. The stored legs are loaded once, before the first
/// write, so no decision this run makes observes a write this run made; that is
/// what lets a sink that writes nothing walk identical branches.
///
/// # Arguments
///
/// * `sink` - Where this run's writes go.
/// * `transactions` - Transaction persistence service.
/// * `sources` - Source-reference persistence service.
/// * `accounts` - Account service, snapshotted once for path resolution.
/// * `commodities` - Commodity service, snapshotted once for code resolution.
/// * `tags` - Tag service, passed to the sink's tag pre-pass.
/// * `batches` - Import batch provenance service, passed to the sink.
/// * `profile_id` - The driving profile, if the run is profile-driven.
/// * `importer` - Stable importer name, recorded on the batch.
/// * `raws` - Parsed transactions in document order.
///
/// # Returns
///
/// The batch the sink opened, the run's totals and diagnostics, and the
/// resolution worklists.
///
/// # Errors
///
/// Returns [`crate::BcError`] on query, insert, or batch-record failure.
#[expect(
    clippy::too_many_arguments,
    reason = "one parameter per service the run reads or writes; bundling them \
              into a struct would only move the same list one level out"
)]
async fn run_with<S>(
    sink: &mut S,
    transactions: &crate::TransactionService,
    sources: &crate::SourceService,
    accounts: &crate::AccountService,
    commodities: &crate::CommodityService,
    tags: &crate::TagService,
    batches: &crate::ImportBatchService,
    profile_id: Option<&bc_models::ProfileId>,
    importer: &str,
    raws: &[RawTransaction],
) -> BcResult<Run>
where
    S: Sink,
{
    let resolver = crate::AccountResolver::load(accounts).await?;
    let commodity_resolver = CommodityResolver::load(commodities).await?;
    let batch_id = sink.open_batch(batches, profile_id, importer).await?;

    let tag_pass = sink.ensure_tags(tags, raws).await?;

    let pass = resolve_legs(&resolver, &commodity_resolver, raws);
    let unresolved_accounts: Vec<String> = pass.unresolved_accounts.into_iter().collect();
    let unresolved_commodities: Vec<String> = pass.unresolved_commodities.into_iter().collect();
    let mut counts = pass.counts;
    // The tag pre-pass ran first, so its diagnostics precede the resolution
    // pass's rather than being appended after them.
    let mut diagnostics = tag_pass.diagnostics;
    diagnostics.append(&mut counts.diagnostics);
    counts.diagnostics = diagnostics;

    let planned = allocate_occurrences(pass.rows);
    // One query per touched account for the whole run, not per row.
    let mut writer = Writer {
        transactions,
        sources,
        commodities: &commodity_resolver,
        accounts: &resolver,
        existing: sources.existing_legs(&touched_accounts(&planned)).await?,
        tags: tag_pass.ids,
        sink: &mut *sink,
    };

    for (raw, legs) in raws.iter().zip(&planned) {
        writer.write_row(raw, legs, &mut counts).await?;
    }

    sink.close_batch(batches, &counts).await?;

    Ok(Run {
        batch_id,
        counts,
        created_tags: tag_pass.created,
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
        unresolved_accounts: BTreeSet::new(),
        unresolved_commodities: BTreeSet::new(),
        counts: Counts::default(),
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
            out.counts.note(
                location_of(raw),
                SkipCause::AmbiguousResidual,
                format!("{} legs, two or more elided", raw.postings.len()),
            );
            out.counts
                .charge(SkipCause::AmbiguousResidual, raw.postings.len());
            out.rows.push(Vec::new());
            continue;
        }

        let mut legs = Vec::with_capacity(raw.postings.len());
        for posting in &raw.postings {
            let mut guards = ResolveGuards {
                unresolved: &mut out.unresolved_accounts,
                unresolved_commodities: &mut out.unresolved_commodities,
                archived_seen: &mut archived,
                warnings: &mut out.counts.warnings,
            };
            match resolve_leg(resolver, commodities, raw, posting, &mut guards) {
                Ok(leg) => legs.push(leg),
                Err((cause, detail)) => {
                    out.counts.note(location_of(raw), cause, detail);
                    out.counts.charge(cause, 1_usize);
                }
            }
        }
        out.rows.push(legs);
    }

    out
}

/// Parses every distinct tag path the document names, in encounter order.
///
/// A path that will not parse warns once and is dropped. It costs the tag, never
/// the leg: tags are decoration, the amount is the value.
///
/// # Arguments
///
/// * `raws` - Parsed transactions in document order.
///
/// # Returns
///
/// The paths that parsed, and a diagnostic per distinct spelling that did not.
fn parse_tag_paths(raws: &[RawTransaction]) -> (Vec<TagPath>, Vec<Diagnostic>) {
    let mut parsed: Vec<TagPath> = Vec::new();
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    // Also the warn-once guard: a spelling reaches the `Err` arm only the first
    // time `insert` returns `true` for it, since every later occurrence hits the
    // `continue` above.
    let mut seen: BTreeSet<String> = BTreeSet::new();

    for raw in raws {
        let leg_tags = raw.postings.iter().flat_map(|posting| posting.tags.iter());
        for stated in raw.tags.iter().chain(leg_tags) {
            if !seen.insert(stated.clone()) {
                continue;
            }
            match stated.parse::<TagPath>() {
                Ok(path) => parsed.push(path),
                Err(error) => {
                    tracing::warn!(
                        location = location_of(raw),
                        tag = stated.as_str(),
                        %error,
                        "malformed tag path; dropping the tag and keeping the leg"
                    );
                    diagnostics.push(Diagnostic {
                        location: location_of(raw).to_owned(),
                        cause: SkipCause::MalformedTag,
                        detail: stated.clone(),
                    });
                }
            }
        }
    }

    (parsed, diagnostics)
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

/// The warn-once guards and warnings accumulator [`resolve_leg`] updates,
/// grouped into one value so the function keeps a reasonable arity.
struct ResolveGuards<'a> {
    /// Accumulator of distinct unresolved accounts; also the warn-once guard,
    /// since inserting a path reports whether it is new.
    unresolved: &'a mut BTreeSet<String>,
    /// Accumulator of distinct unregistered commodity codes, and likewise
    /// their warn-once guard.
    unresolved_commodities: &'a mut BTreeSet<String>,
    /// Warn-once guard for archived accounts already reported.
    archived_seen: &'a mut BTreeSet<String>,
    /// Run-level warnings accumulator; gains one
    /// [`Warning::PostingIntoArchivedAccount`] the first time a distinct
    /// archived account is named.
    warnings: &'a mut Vec<Warning>,
}

/// Resolves one leg, reporting the diagnostics a skipped leg warrants.
///
/// # Arguments
///
/// * `resolver` - The account-tree snapshot to resolve against.
/// * `commodities` - The registry snapshot to resolve commodity codes against.
/// * `raw` - The transaction the leg belongs to, for diagnostics.
/// * `posting` - The leg to resolve.
/// * `guards` - The warn-once guards and warnings accumulator to update.
///
/// # Returns
///
/// The [`ResolvedLeg`], or the [`SkipCause`] that stopped it being persisted
/// this run together with the detail naming what stopped it.
fn resolve_leg(
    resolver: &AccountResolver,
    commodities: &CommodityResolver,
    raw: &RawTransaction,
    posting: &RawPosting,
    guards: &mut ResolveGuards<'_>,
) -> Result<ResolvedLeg, (SkipCause, String)> {
    let path = match AccountPath::parse(&posting.account) {
        Ok(parsed) => parsed,
        Err(error) => {
            tracing::warn!(
                location = location_of(raw),
                account = posting.account.as_str(),
                %error,
                "malformed account path; skipping this leg"
            );
            return Err((SkipCause::MalformedPath, posting.account.clone()));
        }
    };
    let rendered = path.to_string();

    let account_id = match resolver.resolve(&path) {
        Resolution::Resolved { id, archived } => {
            // Warn once per distinct account, for the same reason the unresolved
            // path below does: one archived account named by every row of a file
            // should log one line, not one per row.
            if archived && guards.archived_seen.insert(rendered.clone()) {
                tracing::warn!(
                    location = location_of(raw),
                    account = %path,
                    "importing into an archived account"
                );
                guards.warnings.push(Warning::PostingIntoArchivedAccount {
                    account_id: id.clone(),
                    account_path: rendered.clone(),
                });
            }
            id
        }
        Resolution::Missing {
            resolved_prefix,
            missing_segment,
        } => {
            // Warn once per distinct path: a file naming one missing account in
            // every row should log one line, not one per row.
            if guards.unresolved.insert(rendered.clone()) {
                tracing::warn!(
                    location = location_of(raw),
                    account = rendered.as_str(),
                    resolved_prefix = resolved_prefix.as_str(),
                    missing_segment = missing_segment.as_str(),
                    "account path names no existing account; create it and re-run to \
                     attach the legs skipped now"
                );
            }
            return Err((
                SkipCause::UnresolvedAccount,
                format!(
                    "{rendered} (resolved as far as '{resolved_prefix}', missing \
                     '{missing_segment}')"
                ),
            ));
        }
    };

    let amount = match canonicalise(commodities, posting.amount.as_ref()) {
        Canonical::Resolved(amount) => amount,
        Canonical::Unregistered(code) => {
            // Warn once per distinct code, for the same reason an unresolved
            // account path does: one unregistered commodity named by every row
            // of a file should log one line, not one per row.
            if guards.unresolved_commodities.insert(code.clone()) {
                tracing::warn!(
                    location = location_of(raw),
                    commodity = code.as_str(),
                    "commodity code names no registered commodity; register it and \
                     re-run to attach the legs skipped now"
                );
            }
            return Err((SkipCause::UnresolvedCommodity, code));
        }
        Canonical::Blank => {
            tracing::warn!(
                location = location_of(raw),
                "posting has a blank commodity code; skipping this leg"
            );
            return Err((SkipCause::BlankCommodity, String::new()));
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
        account_path: rendered,
        amount,
        metadata: resolve_metadata(resolver, location_of(raw), &posting.metadata),
        tag_paths: posting.tags.clone(),
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
                        account_path: leg.account_path,
                        amount: leg.amount,
                        fingerprint: leg.fingerprint,
                        metadata: leg.metadata,
                        tag_paths: leg.tag_paths,
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
/// * `tags` - Rendered-path → tag-ID map from the run's pre-pass.
///
/// # Returns
///
/// The postings, or `None` when the residual must be materialised but the
/// document does not determine it.
fn build_postings(
    raw: &RawTransaction,
    legs: &[LegPlan],
    commodities: &CommodityResolver,
    tags: &HashMap<String, TagId>,
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
            .map(|leg| leg.posting(residual.as_ref(), tags))
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

/// Applies the run's per-row failure policy to one row's step.
///
/// A condition local to this row — input that cannot be represented, or a
/// `UNIQUE` violation showing its slot is already claimed — warns and skips the
/// row, exactly as every other unpersistable-row case in this pipeline does.
/// One bad row among thousands must not abort the run and leave a half-written
/// database behind an unclosed batch. A genuine I/O failure still propagates.
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
            counts.note(location_of(raw), SkipCause::RowLocalFailure, action);
            counts.charge(SkipCause::RowLocalFailure, postings);
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// Binds every account path a document's metadata names to an account.
///
/// The six self-contained value types pass through untouched. An account path
/// that names an account becomes a [`MetaValue::Account`]; one that is
/// malformed or names nothing becomes text and warns. Keeping the path as text
/// is what makes the entry repairable: the write path reads it against the
/// key's registered type, finds a path where an id belongs, and flags it.
///
/// # Arguments
///
/// * `resolver` - The account-tree snapshot to resolve against.
/// * `location` - Where the document stated these entries, for diagnostics.
/// * `entries` - The entries the importer stated, in display order.
///
/// # Returns
///
/// The same entries in the same order, every account bound that could be.
fn resolve_metadata(
    resolver: &AccountResolver,
    location: &str,
    entries: &[RawMetaEntry],
) -> Metadata {
    entries
        .iter()
        .map(|entry| {
            let value = match entry.value {
                RawMetaValue::Resolved(ref value) => value.clone(),
                RawMetaValue::AccountPath(ref stated) => {
                    resolve_meta_account(resolver, location, &entry.key, stated)
                }
            };
            MetaEntry::new(entry.key.clone(), value)
        })
        .collect()
}

/// Binds one account path stated as a metadata value.
///
/// # Arguments
///
/// * `resolver` - The account-tree snapshot to resolve against.
/// * `location` - Where the document stated this entry, for diagnostics.
/// * `key` - The key the path is filed under, for diagnostics.
/// * `stated` - The account path as the document wrote it.
///
/// # Returns
///
/// The bound account, or the path as text when nothing binds it.
fn resolve_meta_account(
    resolver: &AccountResolver,
    location: &str,
    key: &MetaKey,
    stated: &str,
) -> MetaValue {
    let Ok(path) = AccountPath::parse(stated) else {
        tracing::warn!(
            location,
            key = key.as_str(),
            account = stated,
            "malformed account path in metadata; keeping the path as text"
        );
        return MetaValue::Text(stated.to_owned());
    };
    match resolver.resolve(&path) {
        Resolution::Resolved { id, .. } => MetaValue::Account(id),
        Resolution::Missing { .. } => {
            tracing::warn!(
                location,
                key = key.as_str(),
                account = stated,
                "account path in metadata names no account; keeping the path as text"
            );
            MetaValue::Text(stated.to_owned())
        }
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

/// Returns the one posting of `stored` that elides its amount.
///
/// # Arguments
///
/// * `stored` - The postings the owning transaction already holds.
///
/// # Returns
///
/// The single elided posting, or `None` when none or several elide — in both of
/// those cases no one leg absorbs the transaction's residual.
fn single_elided<'post>(stored: &[StoredPosting<'post>]) -> Option<StoredPosting<'post>> {
    let mut elided = stored.iter().filter(|held| held.posting.amount().is_none());
    let first = elided.next()?;
    elided.next().is_none().then_some(*first)
}

/// A posting the owning transaction already holds, with the account path it
/// sits on.
///
/// A [`Posting`] carries an [`AccountId`], not a path, and rendering one back
/// needs the account tree — which the sinks do not hold. The run resolves it
/// once here so a report can name the account a stored leg moves.
#[derive(Debug, Clone, Copy)]
struct StoredPosting<'post> {
    /// The rendered path of the account this posting is booked to, or `None`
    /// when the run's account snapshot does not name it.
    account_path: Option<&'post str>,
    /// The posting as it is stored.
    posting: &'post Posting,
}

/// Where an import run's writes go.
///
/// Every decision the run makes — resolution, occurrence allocation, owner
/// matching, corroboration, residual materialisation — happens above this
/// trait and is shared by all implementations. Only the terminal writes differ.
/// That is what lets a dry run be the same run: it walks identical branches
/// because no decision in the run observes a write the run made.
trait Sink {
    /// Materialises or merely resolves the tag paths the document names.
    ///
    /// # Arguments
    ///
    /// * `tags` - The tag service to resolve or create through.
    /// * `raws` - Parsed transactions in document order.
    ///
    /// # Returns
    ///
    /// The rendered-path → leaf-ID map, the paths this run brought into
    /// existence, and a diagnostic per path that would not parse.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on query or insert failure.
    async fn ensure_tags(
        &self,
        tags: &crate::TagService,
        raws: &[RawTransaction],
    ) -> BcResult<Tags>;

    /// Opens the batch this run's writes are stamped with, if it writes any.
    ///
    /// Called once, before the first row, so a sink that stamps the batch onto
    /// what it writes may record it here rather than reaching for interior
    /// mutability.
    ///
    /// # Arguments
    ///
    /// * `batches` - Import batch provenance service.
    /// * `profile_id` - The driving profile, if the run is profile-driven.
    /// * `importer` - Stable importer name, recorded on the batch.
    ///
    /// # Returns
    ///
    /// The batch opened, or `None` from a sink that records no provenance.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on insert failure.
    async fn open_batch(
        &mut self,
        batches: &crate::ImportBatchService,
        profile_id: Option<&bc_models::ProfileId>,
        importer: &str,
    ) -> BcResult<Option<ImportBatchId>>;

    /// Persists a new transaction and the provenance of each of its legs.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction the legs came from.
    /// * `tx` - The transaction to create, postings included.
    /// * `postings` - Its postings, index-aligned with `legs`.
    /// * `legs` - The planned legs those postings were built from.
    ///
    /// # Returns
    ///
    /// Advisory warnings the write-time guard raised for this transaction's
    /// postings. A sink that performs no such check (a dry-run plan) returns
    /// none.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on insert failure.
    async fn create(
        &mut self,
        raw: &RawTransaction,
        tx: Transaction,
        postings: &[Posting],
        legs: &[LegPlan],
    ) -> BcResult<Vec<Warning>>;

    /// Appends unstored legs to the transaction that owns their siblings.
    ///
    /// # Arguments
    ///
    /// * `raw` - The document transaction the legs came from.
    /// * `owner` - The transaction their stored siblings belong to.
    /// * `owner_date` - `owner`'s own value date, checked against each
    ///   inserted posting's account exactly as [`Sink::create`] checks a new
    ///   transaction's postings.
    /// * `stored` - Every posting `owner` already holds, as loaded before this
    ///   row was decided. Together with `postings` this is the complete leg set
    ///   the transaction will hold, which is what a residual has to be derived
    ///   from; the adopted postings are among these, so a sink must not count
    ///   them again from `adoptions`.  Each carries the account path it sits
    ///   on, which the posting itself cannot supply.
    /// * `postings` - The postings to append, index-aligned with `insertions`.
    /// * `insertions` - The legs those postings were built from.
    /// * `adoptions` - Legs whose posting the user already wrote, paired with
    ///   the posting to record provenance against.
    ///
    /// # Returns
    ///
    /// Advisory warnings the write-time guard raised for the appended
    /// postings. A sink that performs no such check (a dry-run plan) returns
    /// none.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on insert failure.
    #[expect(
        clippy::too_many_arguments,
        reason = "one parameter per fact the write needs; bundling them into a \
                  struct would only move the same list one level out"
    )]
    async fn attach(
        &mut self,
        raw: &RawTransaction,
        owner: &TransactionId,
        owner_date: jiff::civil::Date,
        stored: &[StoredPosting<'_>],
        postings: &[Posting],
        insertions: &[&LegPlan],
        adoptions: &[(PostingId, &LegPlan)],
    ) -> BcResult<Vec<Warning>>;

    /// Records the run's totals against the batch, if it opened one.
    ///
    /// # Arguments
    ///
    /// * `batches` - Import batch provenance service.
    /// * `counts` - The run's totals.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError`] on update failure.
    async fn close_batch(
        &self,
        batches: &crate::ImportBatchService,
        counts: &Counts,
    ) -> BcResult<()>;
}

/// The sink that persists what the run decided.
struct Commit<'svc> {
    /// Transaction persistence service.
    transactions: &'svc crate::TransactionService,
    /// Source-reference persistence service.
    sources: &'svc crate::SourceService,
    /// The batch stamped onto every reference this run writes, set by
    /// [`Sink::open_batch`] before the first row is written.
    batch_id: Option<ImportBatchId>,
}

impl Commit<'_> {
    /// Returns the batch every reference this sink writes is stamped with.
    ///
    /// # Returns
    ///
    /// The batch [`Sink::open_batch`] opened.
    ///
    /// # Errors
    ///
    /// Returns [`crate::BcError::InvalidInput`] if the sink is asked to write
    /// before its batch is open. Like every other broken invariant this module
    /// is itself responsible for, that fails the run rather than being absorbed
    /// into the skip count — a reference written without its batch is
    /// provenance silently lost.
    fn batch(&self) -> BcResult<&ImportBatchId> {
        self.batch_id.as_ref().ok_or_else(|| {
            crate::BcError::InvalidInput(
                "the commit sink was asked to write before opening its batch".to_owned(),
            )
        })
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
    /// * `batch_id` - The batch this run stamps onto what it writes.
    /// * `transaction_id` - The transaction the posting belongs to.
    /// * `posting_id` - The posting this leg produced.
    /// * `owns_posting` - `true` when this run inserted that posting, `false`
    ///   when it adopted a posting the user had already written.
    ///
    /// # Returns
    ///
    /// The [`SourceRef`] to persist.
    fn source_ref(
        raw: &RawTransaction,
        leg: &LegPlan,
        batch_id: &ImportBatchId,
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
            .import_batch_id(Some(batch_id.clone()))
            .owns_posting(owns_posting)
            .created_at(Timestamp::now())
            .build()
    }
}

impl Sink for Commit<'_> {
    /// Creates every tag path the document names, before any row is written.
    ///
    /// Creation is a pre-pass rather than per-row work because a tag created
    /// inside a row's database transaction would vanish if that row hit the
    /// row-local skip path, while the in-memory map still held its ID — every
    /// later row naming that tag would then fail on the `tags` foreign key. The
    /// cost of the pre-pass is that a run whose rows are all skipped still
    /// leaves its tags behind; a tag is cheap to delete, and reconstructing an
    /// omitted one by hand across a bulk import is not.
    async fn ensure_tags(
        &self,
        tags: &crate::TagService,
        raws: &[RawTransaction],
    ) -> BcResult<Tags> {
        let (parsed, diagnostics) = parse_tag_paths(raws);
        let created = tags.create_paths(&parsed).await?;
        Ok(Tags {
            ids: created.ids,
            created: created.created,
            diagnostics,
        })
    }

    async fn open_batch(
        &mut self,
        batches: &crate::ImportBatchService,
        profile_id: Option<&bc_models::ProfileId>,
        importer: &str,
    ) -> BcResult<Option<ImportBatchId>> {
        let batch_id = batches.open(profile_id, importer).await?;
        self.batch_id = Some(batch_id.clone());
        Ok(Some(batch_id))
    }

    /// Creates the transaction and attaches its provenance atomically, so a
    /// failure can never leave a posting without the reference that stops a
    /// later re-import duplicating it.
    async fn create(
        &mut self,
        raw: &RawTransaction,
        tx: Transaction,
        postings: &[Posting],
        legs: &[LegPlan],
    ) -> BcResult<Vec<Warning>> {
        let batch_id = self.batch()?;
        let tx_id = tx.id().clone();
        let mut db_tx = self.transactions.pool().begin().await?;
        let warned = self.transactions.create_in_tx(&mut db_tx, tx).await?;
        for (posting, leg) in postings.iter().zip(legs) {
            let source = Self::source_ref(raw, leg, batch_id, &tx_id, posting.id(), true);
            self.sources.attach_in_tx(&mut db_tx, &source).await?;
        }
        db_tx.commit().await?;
        Ok(warned.warnings)
    }

    /// The owner's stored postings are of no interest here: this sink appends
    /// what the run decided and records provenance, and neither depends on what
    /// the transaction already holds.
    async fn attach(
        &mut self,
        raw: &RawTransaction,
        owner: &TransactionId,
        owner_date: jiff::civil::Date,
        _stored: &[StoredPosting<'_>],
        postings: &[Posting],
        insertions: &[&LegPlan],
        adoptions: &[(PostingId, &LegPlan)],
    ) -> BcResult<Vec<Warning>> {
        let batch_id = self.batch()?;
        let mut db_tx = self.transactions.pool().begin().await?;
        let warnings = if postings.is_empty() {
            Vec::new()
        } else {
            self.transactions
                .add_postings_in_tx(&mut db_tx, owner, owner_date, postings)
                .await?
        };
        for (posting, leg) in postings.iter().zip(insertions) {
            let source = Self::source_ref(raw, leg, batch_id, owner, posting.id(), true);
            self.sources.attach_in_tx(&mut db_tx, &source).await?;
        }
        for (posting_id, leg) in adoptions {
            let source = Self::source_ref(raw, leg, batch_id, owner, posting_id, false);
            self.sources.attach_in_tx(&mut db_tx, &source).await?;
        }
        db_tx.commit().await?;
        Ok(warnings)
    }

    /// Records the run's totals against the batch this sink opened.
    async fn close_batch(
        &self,
        batches: &crate::ImportBatchService,
        counts: &Counts,
    ) -> BcResult<()> {
        batches
            .close(
                self.batch()?,
                crate::ImportBatchCounts {
                    new_transactions: counts.new_transactions,
                    attached_postings: counts.attached_postings,
                    unresolved_account_postings: counts.unresolved_account_postings,
                    unresolved_commodity_postings: counts.unresolved_commodity_postings,
                    other_skipped_postings: counts.other_skipped_postings,
                },
            )
            .await
    }
}

/// A [`Sink`] that records what a run would write and writes nothing.
///
/// It holds no service, no pool and no transaction, so it cannot commit: the
/// guarantee is structural, not a matter of discipline.
#[derive(Debug, Default)]
struct Plan {
    /// Per-account sums of the legs handed to this sink, ordered by path.
    totals: BTreeMap<String, Balances>,
}

impl Plan {
    /// Folds one leg's value into its account's running total.
    ///
    /// A posting always claims a bucket, even when it moves nothing, so an
    /// account whose legs net to zero stays distinguishable from one the run
    /// never touches.
    ///
    /// # Arguments
    ///
    /// * `posting` - The posting the run would write.
    /// * `leg` - The plan it came from, which carries the rendered path.
    /// * `derived` - The per-commodity residual this posting absorbs, when it is
    ///   the row's one elided leg and the row determines one; `None` when the
    ///   posting states its own amount or the residual is unavailable.
    fn record(&mut self, posting: &Posting, leg: &LegPlan, derived: Option<&Balances>) {
        match posting.amount() {
            // A stated amount moves its account by itself.
            Some(amount) => {
                let entry = self.totals.entry(leg.account_path.clone()).or_default();
                // A total that overflows `Decimal` is a report defect, not a run
                // failure: the legs still post. Keep the bucket at its last good
                // value.
                if entry.try_add(amount).is_err() {
                    Self::warn_overflow(&leg.account_path);
                }
            }
            // An elided leg moves by whatever residual it absorbs, which is
            // nothing at all when the transaction determines none.
            None => self.shift(&leg.account_path, derived, None),
        }
    }

    /// Moves `path`'s running total by `add` minus `subtract`, per commodity.
    ///
    /// The bucket is claimed whatever the movement, so an account the run
    /// touches stays distinguishable from one it never names — including an
    /// account whose legs net to zero.
    ///
    /// # Arguments
    ///
    /// * `path` - The rendered account path to move.
    /// * `add` - Per-commodity amounts the account gains.
    /// * `subtract` - Per-commodity amounts it loses.
    fn shift(&mut self, path: &str, add: Option<&Balances>, subtract: Option<&Balances>) {
        let entry = self.totals.entry(path.to_owned()).or_default();
        // Every commodity is folded, not just those up to the first failure:
        // one overflowing bucket must not silently drop the rest of a
        // multi-commodity movement.
        let mut overflowed = false;
        for (code, value) in add.into_iter().flat_map(Balances::iter) {
            overflowed |= entry.try_add(&Amount::new(value, code)).is_err();
        }
        for (code, value) in subtract.into_iter().flat_map(Balances::iter) {
            overflowed |= entry.try_sub(&Amount::new(value, code)).is_err();
        }
        if overflowed {
            Self::warn_overflow(path);
        }
    }

    /// Warns that `path`'s reported figure is short of the truth.
    ///
    /// # Arguments
    ///
    /// * `path` - The account whose total could not be kept.
    fn warn_overflow(path: &str) {
        tracing::warn!(
            account = path,
            "the account's planned total overflowed; the reported figure is short"
        );
    }

    /// Derives the residual a transaction's one elided leg absorbs.
    ///
    /// An elided leg is not weightless: the balance engine derives its value
    /// from its siblings, so a report that left the bucket empty would say an
    /// account nets to zero when the import is about to move it. This is
    /// [`crate::residual::residual_of`], the very function the balance read path
    /// derives its own residuals through, so the reported figure and the
    /// balance the user later reads cannot drift apart.
    ///
    /// # Arguments
    ///
    /// * `postings` - **Every** posting the transaction will hold once this
    ///   run's writes land, the ones already stored included. A partial set
    ///   would name a residual the transaction does not have.
    ///
    /// # Returns
    ///
    /// The per-commodity residual, or `None` when the transaction has no elided
    /// leg, has more than one, or overflows.
    fn residual<'post>(postings: impl IntoIterator<Item = &'post Posting>) -> Option<Balances> {
        // Two or more elided legs never reach a create — `has_ambiguous_residual`
        // skips the row whole, and resolution only ever drops legs — but an
        // attach can still meet one stored beside one appended, and the balance
        // engine attributes that residual to neither. `residual_of` classifies
        // it as `Ambiguous` and nothing is folded, exactly as the read path does.
        match crate::residual::residual_of(postings.into_iter().map(Posting::amount)) {
            Ok(crate::residual::Residual::Attributable(balances)) => Some(balances),
            Ok(crate::residual::Residual::NotElided | crate::residual::Residual::Ambiguous) => None,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "the row's residual overflowed; its elided leg is reported as moving nothing"
                );
                None
            }
        }
    }

    /// Folds the postings this run would write into their accounts' totals.
    ///
    /// # Arguments
    ///
    /// * `postings` - The postings the run would write, index-aligned with
    ///   `legs`. Only these move a total: a posting the transaction already
    ///   holds is already in the user's balances.
    /// * `legs` - The plans those postings were built from.
    /// * `residual` - The transaction's residual, from [`Self::residual`].
    fn record_legs<'leg>(
        &mut self,
        postings: &[Posting],
        legs: impl IntoIterator<Item = &'leg LegPlan>,
        residual: Option<&Balances>,
    ) {
        for (posting, leg) in postings.iter().zip(legs) {
            let derived = if posting.amount().is_none() {
                residual
            } else {
                None
            };
            self.record(posting, leg, derived);
        }
    }
}

impl Sink for Plan {
    /// Resolves the tag paths rather than creating them, so the tag tree is left
    /// exactly as the plan found it while still reporting what a run would add.
    async fn ensure_tags(
        &self,
        tags: &crate::TagService,
        raws: &[RawTransaction],
    ) -> BcResult<Tags> {
        let (parsed, diagnostics) = parse_tag_paths(raws);
        let resolved = tags.resolve_paths(&parsed).await?;
        Ok(Tags {
            ids: resolved.ids,
            created: resolved.created,
            diagnostics,
        })
    }

    /// Opens no batch: a dry run records no provenance.
    fn open_batch(
        &mut self,
        _batches: &crate::ImportBatchService,
        _profile_id: Option<&bc_models::ProfileId>,
        _importer: &str,
    ) -> impl Future<Output = BcResult<Option<ImportBatchId>>> {
        ready(Ok(None))
    }

    /// Raises no warnings: `check_postings` needs a `&mut sqlx::SqliteConnection`,
    /// and `Plan` holds no connection by design (see the struct doc), so it cannot
    /// run here. A plan's warnings are therefore only those [`resolve_leg`] already
    /// raised during resolution (currently just the archived-account one) — a
    /// lower bound, not the complete set a real run will produce.
    fn create(
        &mut self,
        _raw: &RawTransaction,
        _tx: Transaction,
        postings: &[Posting],
        legs: &[LegPlan],
    ) -> impl Future<Output = BcResult<Vec<Warning>>> {
        // A brand-new transaction holds exactly these postings, so they are the
        // whole leg set the residual is derived from.
        let residual = Self::residual(postings);
        self.record_legs(postings, legs, residual.as_ref());
        ready(Ok(Vec::new()))
    }

    /// Records the appended legs, and the movement the appending causes
    /// elsewhere.
    ///
    /// Two accounts move here, not one. The appended legs move their own, and
    /// an elided leg the transaction *already* holds moves too: its value is
    /// derived from its siblings, and this run is adding siblings. That second
    /// movement writes no posting, so it is easy to miss, and missing it would
    /// report an account as untouched in exactly the workflow this feature
    /// exists for — create the missing account, re-run, watch the rest land.
    ///
    /// An adoption is not among either: its posting is already the user's own
    /// and already in their balances, so it moves no total, though it does feed
    /// the residual by way of `stored`, which is where its posting sits.
    ///
    /// Raises no warnings, for the same reason [`Self::create`] raises none: no
    /// connection is available to run `check_postings` here.
    fn attach(
        &mut self,
        _raw: &RawTransaction,
        _owner: &TransactionId,
        _owner_date: jiff::civil::Date,
        stored: &[StoredPosting<'_>],
        postings: &[Posting],
        insertions: &[&LegPlan],
        _adoptions: &[(PostingId, &LegPlan)],
    ) -> impl Future<Output = BcResult<Vec<Warning>>> {
        let before = Self::residual(stored.iter().map(|held| held.posting));
        let after = Self::residual(stored.iter().map(|held| held.posting).chain(postings));

        // The stored elided leg — if there is exactly one — held `before` and
        // will hold `after`, so it moves by the difference. Both being `None`
        // covers the ordinary case of no stored elided leg at all, and the
        // two-elided case where the balance engine attributes the residual to
        // neither leg, which is a movement of `-before` and equally right.
        if let Some(held) = single_elided(stored) {
            if let Some(path) = held.account_path {
                self.shift(path, after.as_ref(), before.as_ref());
            } else {
                // The run's account snapshot does not name this account, so the
                // report has nothing to file the movement under — it can only
                // have been deleted since the snapshot was taken.
                tracing::warn!(
                    posting = %held.posting.id(),
                    "a stored elided posting sits on an account the run cannot name; its \
                     planned movement is unreported"
                );
            }
        }

        self.record_legs(postings, insertions.iter().copied(), after.as_ref());
        ready(Ok(Vec::new()))
    }

    /// Closes no batch, because none was opened.
    fn close_batch(
        &self,
        _batches: &crate::ImportBatchService,
        _counts: &Counts,
    ) -> impl Future<Output = BcResult<()>> {
        ready(Ok(()))
    }
}

/// The write half of an import run: everything the per-row decision needs.
struct Writer<'svc, S> {
    /// Transaction persistence service.
    transactions: &'svc crate::TransactionService,
    /// Source-reference persistence service.
    sources: &'svc crate::SourceService,
    /// Commodity registry snapshot, for canonicalising a materialised
    /// residual's commodity code.
    commodities: &'svc CommodityResolver,
    /// Account tree snapshot, for naming the account a stored posting sits on.
    accounts: &'svc AccountResolver,
    /// Legs already stored for every account this run touches, keyed by
    /// `(account id string, fingerprint)`.
    existing: HashMap<(String, String), Vec<StoredLeg>>,
    /// Rendered-path → tag-ID map from the run's pre-pass.
    tags: HashMap<String, TagId>,
    /// Where this run's writes go.
    sink: &'svc mut S,
}

impl<S> Writer<'_, S>
where
    S: Sink,
{
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
        &mut self,
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
                counts.note(
                    location_of(raw),
                    SkipCause::MultiOwnerConflict,
                    format!("{} transactions claim these legs", conflicting.len()),
                );
                counts.charge(SkipCause::MultiOwnerConflict, unstored);
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
        &mut self,
        raw: &RawTransaction,
        legs: &[LegPlan],
        counts: &mut Counts,
    ) -> BcResult<()> {
        // An overflow is this row's own defect, so it warns and skips like any
        // other unpersistable row rather than aborting the run.
        let built = row_local_value(
            build_postings(raw, legs, self.commodities, &self.tags),
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
                counts.note(
                    location_of(raw),
                    SkipCause::UndeterminedResidual,
                    "the elided leg is the only leg that resolved".to_owned(),
                );
                counts.charge(SkipCause::UndeterminedResidual, legs.len());
            }
            return Ok(());
        };

        // A freshly imported transaction may hold fewer legs than the document
        // did, so it can be unbalanced (an accepted state). It stays
        // `Unreconciled` until its remaining legs arrive.
        let tx = Transaction::builder()
            .id(TransactionId::new())
            .date(raw.date)
            .description(raw.description.clone())
            .metadata(resolve_metadata(
                self.accounts,
                location_of(raw),
                &raw.metadata,
            ))
            .tag_ids(resolve_tag_ids(&raw.tags, &self.tags))
            .postings(postings.clone())
            .reconciliation(Reconciliation::Unreconciled)
            .created_at(Timestamp::now())
            .build();

        let written = self.sink.create(raw, tx, &postings, legs).await;

        let Some(warnings) =
            row_local_value(written, raw, "creating the transaction", legs.len(), counts)?
        else {
            return Ok(());
        };
        counts.push_warnings(warnings);

        counts.new_transactions = counts.new_transactions.saturating_add(1_usize);
        Ok(())
    }

    /// Appends the legs an earlier run could not persist to the transaction it
    /// created.
    ///
    /// Only the inserted postings' own tags are applied — `raw.tags`, the
    /// document's transaction-level tags, are not, because `owner` already
    /// exists and a re-import must not revise a transaction's own fields (the
    /// same invariant that keeps its note from being overwritten, see
    /// `attaching_a_leg_does_not_revise_transaction_metadata`). The one wrinkle:
    /// the pre-pass still creates and reports those paths in
    /// [`ImportOutcome::created_tags`] even when this path ends up linking
    /// nothing to them, since creation happens before any row decides whether
    /// it is creating or attaching.
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
        &mut self,
        raw: &RawTransaction,
        candidates: &[Candidate<'_>],
        owner: &TransactionId,
        counts: &mut Counts,
    ) -> BcResult<()> {
        let unstored = candidates.iter().filter(|c| !c.stored).count();
        if unstored == 0 {
            return Ok(());
        }

        // Both lookups precede any write *of this row*, but not of the run: they
        // read live, inside the write loop, so an earlier row of the same run
        // could in principle have changed what they return. What stops it is the
        // occurrence slot. A leg matches an owner through
        // `(account, fingerprint, occurrence)`, `allocate_occurrences` hands each
        // leg of the run a distinct slot within that key, and a slot is claimed
        // by at most one stored leg — so no two rows of one run can match the
        // same owner, and no row sees this transaction after another row of the
        // same run has appended to it. Relaxing corroboration, or allocating
        // slots per row rather than per run, breaks that and makes this a stale
        // read.
        //
        // One hole in the argument, recorded rather than closed. `existing_legs`
        // does not join `postings`, so a reference whose posting was deleted
        // still names its owner while contributing no posting for a row to
        // explain. That construct can leave a real run failing corroboration
        // where a plan does not, which is the one place the two can part
        // company. It is the run that is the more conservative of the two, so
        // the plan over-promises rather than the run over-writing — the safe
        // direction for a report, and the reason this is a note and not a fix.
        //
        // A failure of either lookup is row-local: one unreadable candidate must
        // not abort a run whose other rows are fine.
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
            counts.note(
                location_of(raw),
                SkipCause::FailedCorroboration,
                format!("transaction {owner}"),
            );
            counts.charge(SkipCause::FailedCorroboration, unstored);
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
        let postings: Vec<Posting> = insertions
            .iter()
            .map(|leg| leg.posting(None, &self.tags))
            .collect();

        let stored: Vec<StoredPosting<'_>> = candidate
            .postings()
            .iter()
            .map(|posting| StoredPosting {
                account_path: self.accounts.path_of(posting.account_id()),
                posting,
            })
            .collect();

        let written = self
            .sink
            .attach(
                raw,
                owner,
                candidate.date(),
                &stored,
                &postings,
                &insertions,
                &adoptions,
            )
            .await;

        let Some(warnings) = row_local_value(
            written,
            raw,
            "appending the unstored legs",
            unstored,
            counts,
        )?
        else {
            return Ok(());
        };
        counts.push_warnings(warnings);

        counts.attached_postings = counts.attached_postings.saturating_add(unstored);
        Ok(())
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
    use crate::account::Cascade;

    /// Builds a text metadata entry, panicking on a key the tests wrote wrong.
    fn meta_text(key: &str, value: &str) -> RawMetaEntry {
        RawMetaEntry::resolved(
            MetaKey::new(key).expect("test key must be valid"),
            MetaValue::Text(value.to_owned()),
        )
    }

    /// Builds a date metadata entry, panicking on a key the tests wrote wrong.
    fn meta_date(key: &str, when: jiff::civil::Date) -> RawMetaEntry {
        RawMetaEntry::resolved(
            MetaKey::new(key).expect("test key must be valid"),
            MetaValue::Date(when),
        )
    }

    /// Builds an unresolved account-path metadata entry.
    fn meta_account(key: &str, path: &str) -> RawMetaEntry {
        RawMetaEntry::account_path(MetaKey::new(key).expect("test key must be valid"), path)
    }

    /// Builds the service bundle every test needs.
    struct Services {
        transactions: crate::TransactionService,
        sources: crate::SourceService,
        accounts: crate::AccountService,
        commodities: crate::CommodityService,
        batches: crate::ImportBatchService,
        tags: crate::TagService,
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
            tags: crate::TagService::new(pool.clone()),
        }
    }

    /// Runs an import with no profile, under the "test" importer name.
    async fn run(svcs: &Services, raws: &[RawTransaction]) -> ImportOutcome {
        execute_import(
            &svcs.transactions,
            &svcs.sources,
            &svcs.accounts,
            &svcs.commodities,
            &svcs.tags,
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
        sqlx::query_scalar(
            "SELECT m.value_text FROM posting_metadata m \
             JOIN postings p ON m.posting_id = p.id \
             WHERE p.account_id = ? AND m.key = 'note'",
        )
        .bind(account.to_string())
        .fetch_optional(pool)
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
                    .metadata(vec![meta_text("note", "paid by card")])
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
        sqlx::query_scalar("SELECT value_text FROM transaction_metadata WHERE key = 'note'")
            .fetch_optional(pool)
            .await
            .expect("transaction note")
    }

    /// Reads every date-keyed metadata entry of the single transaction, in
    /// display order.
    async fn dates_of_transaction(pool: &SqlitePool) -> Vec<(String, String)> {
        sqlx::query_as(
            "SELECT m.key, m.value_text FROM transaction_metadata m \
             JOIN metadata_keys k ON m.key = k.key \
             WHERE k.value_type = 'date' ORDER BY m.position",
        )
        .fetch_all(pool)
        .await
        .expect("transaction dates")
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_note_and_dates_persist_as_metadata(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .metadata(vec![
                meta_text("note", "split with flatmate"),
                meta_date("settled", date(2025, 6, 29)),
                meta_date("posted", date(2025, 6, 28)),
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
                ("settled".to_owned(), "2025-06-29".to_owned()),
                ("posted".to_owned(), "2025-06-28".to_owned()),
            ],
            "entries keep the order the document stated them in"
        );
    }

    /// Reads the stored text and account foreign key of the single entry filed
    /// under `key`.
    async fn entry_under(pool: &SqlitePool, key: &str) -> Option<(String, Option<String>)> {
        sqlx::query_as("SELECT value_text, value_account FROM transaction_metadata WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .expect("metadata entry")
    }

    /// Imports one transaction carrying `stated` as an account-valued entry.
    async fn run_with_counterparty(pool: &SqlitePool, stated: &str) {
        let svcs = services(pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .metadata(vec![meta_account("counterparty", stated)])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();
        run(&svcs, &[raw]).await;
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_path_in_metadata_binds_to_that_account(pool: SqlitePool) {
        let (_bank, food) = two_account_tree(&pool).await;

        run_with_counterparty(&pool, "Expenses:Food").await;

        assert_eq!(
            entry_under(&pool, "counterparty").await,
            Some(("Expenses:Food".to_owned(), Some(food.to_string()))),
            "a bound account is stored by id, with its path for a human to read"
        );
    }

    /// A plugin has no account tree, so it can only name an account by path. A
    /// path the tree does not hold costs the binding, never the entry: the text
    /// survives for the user to repair.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_path_naming_nothing_stays_text(pool: SqlitePool) {
        two_account_tree(&pool).await;

        run_with_counterparty(&pool, "Assets:Nowhere").await;

        assert_eq!(
            entry_under(&pool, "counterparty").await,
            Some(("Assets:Nowhere".to_owned(), None))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_malformed_account_path_stays_text(pool: SqlitePool) {
        two_account_tree(&pool).await;

        run_with_counterparty(&pool, "").await;

        assert_eq!(
            entry_under(&pool, "counterparty").await,
            Some((String::new(), None))
        );
    }

    /// Reads the stored text and account foreign key of the single entry filed
    /// under `key` on the leg booked to `account`.
    async fn leg_entry_under(
        pool: &SqlitePool,
        account: &AccountId,
        key: &str,
    ) -> Option<(String, Option<String>)> {
        sqlx::query_as(
            "SELECT m.value_text, m.value_account FROM posting_metadata m \
             JOIN postings p ON m.posting_id = p.id \
             WHERE p.account_id = ? AND m.key = ?",
        )
        .bind(account.to_string())
        .bind(key)
        .fetch_optional(pool)
        .await
        .expect("leg metadata entry")
    }

    /// Imports one transaction whose only leg states `stated` as an
    /// account-valued entry.
    async fn run_with_leg_counterparty(pool: &SqlitePool, stated: &str) {
        let svcs = services(pool).await;
        let raw = raw_with(
            "groceries",
            vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .metadata(vec![meta_account("counterparty", stated)])
                    .build(),
            ],
        );
        run(&svcs, &[raw]).await;
    }

    /// A leg's entries reach the resolver down a different call path from a
    /// row's — `resolve_leg` rather than the writer — so binding is asserted on
    /// each owner rather than inferred from the other.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_path_on_a_leg_binds_to_that_account(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;

        run_with_leg_counterparty(&pool, "Expenses:Food").await;

        assert_eq!(
            leg_entry_under(&pool, &bank, "counterparty").await,
            Some(("Expenses:Food".to_owned(), Some(food.to_string())))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_path_on_a_leg_naming_nothing_stays_text(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;

        run_with_leg_counterparty(&pool, "Assets:Nowhere").await;

        assert_eq!(
            leg_entry_under(&pool, &bank, "counterparty").await,
            Some(("Assets:Nowhere".to_owned(), None))
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_repeated_date_label_becomes_two_entries(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .metadata(vec![
                meta_date("posted", date(2025, 6, 28)),
                meta_date("posted", date(2025, 6, 30)),
                meta_date("settled", date(2025, 6, 29)),
            ])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        // Metadata permits repeated keys, so both `posted` entries are kept.
        // They previously had to be deduplicated because `transaction_dates`
        // was keyed by `(transaction_id, label)`; dropping one now would
        // discard data for no reason.
        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(
            dates_of_transaction(&pool).await,
            vec![
                ("posted".to_owned(), "2025-06-28".to_owned()),
                ("posted".to_owned(), "2025-06-30".to_owned()),
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
            .metadata(vec![meta_text("note", "original note")])
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
            .metadata(vec![meta_text("note", "revised note")])
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

    /// `Writer::attach` deliberately does not apply `raw.tags` to the
    /// transaction it attaches onto — invariant 2 of the spec, and the sibling
    /// case to `attaching_a_leg_does_not_revise_transaction_metadata`, which
    /// pins the same invariant for the note. Naming a brand-new tag on the
    /// second run's document must not attach it to the existing transaction,
    /// even though the tag itself is still created and reported: the pre-pass
    /// that creates it runs before any row decides whether it is creating or
    /// attaching, so `created_tags` names the path while nothing links to it.
    /// If `attach` were changed to apply `raw.tags`, this would fail on the
    /// first assertion below.
    #[sqlx::test(migrations = "./migrations")]
    async fn attaching_a_leg_does_not_add_a_new_transaction_tag(pool: SqlitePool) {
        // First run: only Assets:Bank exists, so the Expenses:Food leg is skipped
        // and the transaction is created carrying no tags.
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;
        let first = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .postings(vec![
                leg("Assets:Bank", Some(-50_i64)),
                leg("Expenses:Food", Some(50_i64)),
            ])
            .build();
        run(&svcs, &[first]).await;

        // Second run: the account now exists and the document names a new
        // transaction-level tag. The missing leg attaches; the tag must not.
        add_food(&pool).await;
        let second = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["urgent".to_owned()])
            .postings(vec![
                leg("Assets:Bank", Some(-50_i64)),
                leg("Expenses:Food", Some(50_i64)),
            ])
            .build();
        let outcome = run(&svcs, &[second]).await;

        assert_eq!(outcome.attached_postings, 1);
        assert!(
            tag_names_of_transaction(&pool).await.is_empty(),
            "attach must not apply the document's transaction-level tags"
        );
        assert_eq!(
            outcome.created_tags,
            vec!["urgent".to_owned()],
            "the pre-pass still creates and reports the tag, even though it links to nothing"
        );
    }

    /// `Writer::attach` is the one code path that builds a posting outside
    /// `Writer::create` — a leg an earlier run could not persist, joining the
    /// transaction that earlier run did create. It has its own `leg.posting(...)`
    /// call site, so it needs its own coverage that tags survive it.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_attached_leg_carries_its_tags(pool: SqlitePool) {
        // First run: only Assets:Bank exists, so the Expenses:Food leg is skipped
        // and the transaction is created with just the Bank posting.
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;
        let first = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .postings(vec![
                leg("Assets:Bank", Some(-50_i64)),
                leg("Expenses:Food", Some(50_i64)),
            ])
            .build();
        run(&svcs, &[first]).await;

        // Second run: Food now exists, so its leg attaches to the transaction the
        // first run created. This leg carries a tag the first run never saw.
        let food = add_food(&pool).await;
        let second = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .postings(vec![
                leg("Assets:Bank", Some(-50_i64)),
                RawPosting::builder()
                    .account("Expenses:Food")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .tags(vec!["reimbursable".to_owned()])
                    .build(),
            ])
            .build();
        let outcome = run(&svcs, &[second]).await;

        assert_eq!(outcome.attached_postings, 1);
        let posting_tags: Vec<String> = sqlx::query_scalar(
            "SELECT t.name FROM posting_tags pt \
             JOIN tags t ON t.id = pt.tag_id \
             JOIN postings p ON p.id = pt.posting_id \
             WHERE p.account_id = ? ORDER BY t.name",
        )
        .bind(food.to_string())
        .fetch_all(&pool)
        .await
        .expect("posting tag names");
        assert_eq!(posting_tags, vec!["reimbursable".to_owned()]);
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
    async fn import_reports_warnings_without_charging_a_skip(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;

        // Close the destination account before every imported row's date, via
        // direct SQL: `Service` exposes no method to set `closed_on` after
        // creation, so the write-time guard's own tests (`warning.rs`) seed it
        // the same way.
        sqlx::query("UPDATE accounts SET closed_on = ?1 WHERE id = ?2")
            .bind(date(2019, 1, 1).to_string())
            .bind(bank.to_string())
            .execute(&pool)
            .await
            .expect("seed closed_on");

        let batch = vec![raw("COFFEE", -5), raw("LUNCH", -20)];
        let outcome = run(&svcs, &batch).await;

        assert!(
            !outcome.warnings.is_empty(),
            "expected a closed-account warning"
        );
        assert_eq!(outcome.skipped_postings, 0, "a warning must not skip a leg");
        assert!(
            outcome.charged_by_cause.is_empty(),
            "{:?}",
            outcome.charged_by_cause
        );
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn a_multi_row_import_into_one_closed_account_warns_once(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;

        sqlx::query("UPDATE accounts SET closed_on = ?1 WHERE id = ?2")
            .bind(date(2019, 1, 1).to_string())
            .bind(bank.to_string())
            .execute(&pool)
            .await
            .expect("seed closed_on");

        // Several rows, all posting to the same closed account: the write-time
        // guard raises one `PostingAfterAccountClosed` per posting it checks,
        // so without a collection-site dedup this would report one line per
        // row rather than one per account.
        let batch = vec![
            raw("COFFEE", -5),
            raw("LUNCH", -20),
            raw("DINNER", -40),
            raw("SNACK", -3),
        ];
        let outcome = run(&svcs, &batch).await;

        assert_eq!(outcome.new_transactions, 4);
        assert_eq!(outcome.skipped_postings, 0, "a warning must not skip a leg");
        assert_eq!(
            outcome
                .warnings
                .iter()
                .filter(|w| matches!(w, Warning::PostingAfterAccountClosed { .. }))
                .count(),
            1,
            "one closed-account warning per account for the whole run, not one per posting: \
             {:?}",
            outcome.warnings
        );
    }

    /// Builds a row posting `amount` of `code` into `Assets:Bank`.
    fn raw_in(desc: &str, amount: i64, code: &str) -> RawTransaction {
        RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description(desc)
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(amount),
                        CommodityCode::new(code),
                    )))
                    .build(),
            ])
            .build()
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn each_undeclared_commodity_is_reported_once_per_account(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;

        // `Assets:Bank` declares AUD and nothing else.
        let aud: String = sqlx::query_scalar("SELECT id FROM commodities WHERE code = 'AUD'")
            .fetch_one(&pool)
            .await
            .expect("find AUD");
        sqlx::query(
            "INSERT INTO account_commodities (account_id, commodity_id, position) \
             VALUES (?1, ?2, 0)",
        )
        .bind(bank.to_string())
        .bind(&aud)
        .execute(&pool)
        .await
        .expect("declare AUD");

        // Two rows in BTC and two in ETH. Repeats of one code collapse, but the
        // two distinct codes are two distinct facts: reporting only the first
        // would send the user to fix BTC and leave them to rediscover ETH on
        // the next run.
        let batch = vec![
            raw_in("COIN", -1, "BTC"),
            raw_in("COIN AGAIN", -2, "BTC"),
            raw_in("ETHER", -3, "ETH"),
            raw_in("ETHER AGAIN", -4, "ETH"),
        ];
        let outcome = run(&svcs, &batch).await;

        assert_eq!(outcome.new_transactions, 4);
        assert_eq!(outcome.skipped_postings, 0, "a warning must not skip a leg");

        let mut codes: Vec<&str> = outcome
            .warnings
            .iter()
            .filter_map(|w| match *w {
                Warning::CommodityOutsideAccountList {
                    ref commodity_code, ..
                } => Some(commodity_code.as_str()),
                Warning::PostingBeforeAccountOpened { .. }
                | Warning::PostingAfterAccountClosed { .. }
                | Warning::PostingIntoArchivedAccount { .. } => None,
            })
            .collect();
        codes.sort_unstable();
        assert_eq!(codes, vec!["BTC", "ETH"], "{:?}", outcome.warnings);
    }

    #[sqlx::test(migrations = "./migrations")]
    async fn an_attached_leg_into_a_closed_account_is_warned(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;
        let batch = vec![split_raw("SPLIT")];

        // `Expenses:Food` does not exist yet, so the first pass attaches only
        // the `Assets:Bank` leg and skips the other for want of an account.
        let first = run(&svcs, &batch).await;
        assert_eq!(first.skipped_postings, 1);
        assert_eq!(posting_count(&pool).await, 1);

        // The user creates the missing account, but it is already closed as of
        // a date before this row's — the second pass attaches the leg into it
        // anyway (warn, don't block), and must say so exactly as a freshly
        // created transaction would. This is the attach counterpart of
        // `import_reports_warnings_without_charging_a_skip`: only `Sink::create`
        // used to run the write-time guard, so a leg reaching an existing
        // transaction through `Sink::attach` warned on neither channel.
        let food = add_food(&pool).await;
        sqlx::query("UPDATE accounts SET closed_on = ?1 WHERE id = ?2")
            .bind(date(2019, 1, 1).to_string())
            .bind(food.to_string())
            .execute(&pool)
            .await
            .expect("seed closed_on");

        let second = run(&svcs, &batch).await;

        assert_eq!(
            second.new_transactions, 0,
            "the transaction already exists; it must not be duplicated"
        );
        assert_eq!(second.attached_postings, 1, "the leg is still attached");
        assert_eq!(
            posting_count(&pool).await,
            2,
            "the leg was written despite the warning"
        );
        assert_eq!(second.skipped_postings, 0, "a warning must not skip a leg");
        assert!(
            second
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::PostingAfterAccountClosed { .. })),
            "attaching into a closed account must warn: {:?}",
            second.warnings
        );
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
            .archive(&food, Cascade::Reject)
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

        // The resolution pass raises the warn-once version of this warning; the
        // write-time guard raises its own per-posting one for the same account,
        // which the collection site filters out so the two do not double up.
        assert_eq!(
            outcome
                .warnings
                .iter()
                .filter(|warning| matches!(warning, Warning::PostingIntoArchivedAccount { .. }))
                .count(),
            1,
            "{:?}",
            outcome.warnings
        );
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

    /// Reads the rendered paths of every tag attached to the single transaction.
    async fn tag_names_of_transaction(pool: &SqlitePool) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT t.name FROM transaction_tags tt \
             JOIN tags t ON t.id = tt.tag_id ORDER BY t.name",
        )
        .fetch_all(pool)
        .await
        .expect("transaction tag names")
    }

    /// Reads the rendered paths of every tag attached to `account`'s posting.
    async fn tag_names_of_posting(pool: &SqlitePool, account: &AccountId) -> Vec<String> {
        sqlx::query_scalar(
            "SELECT t.name FROM posting_tags pt \
             JOIN tags t ON t.id = pt.tag_id \
             JOIN postings p ON p.id = pt.posting_id \
             WHERE p.account_id = ? ORDER BY t.name",
        )
        .bind(account.to_string())
        .fetch_all(pool)
        .await
        .expect("posting tag names")
    }

    /// A tag the document names does not exist yet, so the run creates it and
    /// attaches it to the transaction it names it on.
    #[sqlx::test(migrations = "./migrations")]
    async fn transaction_tags_are_created_and_attached(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["household".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        assert_eq!(outcome.created_tags, vec!["household".to_owned()]);
        assert_eq!(
            tag_names_of_transaction(&pool).await,
            vec!["household".to_owned()]
        );
    }

    /// No accounts exist, so every leg is skipped — but the pre-pass runs first
    /// and its tags must persist, since it sits outside the rows' transactions.
    /// This is the accepted trade of running tag creation before any row is
    /// written: it is cheap to delete a tag a fully-skipped run leaves behind,
    /// far cheaper than reconstructing one an in-transaction rollback erased.
    #[sqlx::test(migrations = "./migrations")]
    async fn tags_survive_a_run_whose_rows_all_skip(pool: SqlitePool) {
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["household".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        assert_eq!(outcome.new_transactions, 0);
        assert_eq!(outcome.created_tags, vec!["household".to_owned()]);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags")
            .fetch_one(&pool)
            .await
            .expect("count tags");
        assert_eq!(count, 1);
    }

    /// A tag path that will not parse warns once and is dropped; the leg it was
    /// stated on still persists. Unlike an unresolved account or commodity, a
    /// bad tag never costs the leg — tags are decoration, the amount is the
    /// value.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_malformed_tag_costs_the_tag_not_the_leg(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["person::alpha".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(outcome.skipped_postings, 0);
        assert!(outcome.created_tags.is_empty());
        assert!(tag_names_of_transaction(&pool).await.is_empty());
    }

    /// A malformed tag is its own cause, not the account-path one: a report
    /// grouping by cause and printing a count per group would otherwise
    /// overstate the postings lost to malformed account paths. It is noted but
    /// never charged, so no count moves.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_malformed_tag_is_diagnosed_without_being_charged(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["person::alpha".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        let causes: Vec<SkipCause> = outcome.diagnostics.iter().map(|d| d.cause).collect();
        assert_eq!(causes, vec![SkipCause::MalformedTag]);
        let detail = outcome
            .diagnostics
            .first()
            .map(|d| d.detail.clone())
            .expect("one diagnostic");
        assert_eq!(detail, "person::alpha");
        assert_eq!(
            outcome.other_skipped_postings, 0,
            "a dropped tag costs the tag, never a posting"
        );
        assert_eq!(outcome.skipped_postings, 0);
    }

    /// The warn-once guard is keyed on the spelling, not the leg, so the second
    /// occurrence of a bad path takes the `continue` arm rather than the `Err`
    /// arm. It must still be dropped there: a spelling that warns once has to
    /// stay dropped everywhere it appears, on postings as much as on the
    /// transaction.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_repeated_malformed_posting_tag_stays_dropped(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(-50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .tags(vec!["person::alpha".to_owned(), "reimbursable".to_owned()])
                    .build(),
                RawPosting::builder()
                    .account("Expenses:Food")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .tags(vec!["person::alpha".to_owned()])
                    .build(),
            ])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        // Both legs persist, and only the parsable tag was created.
        assert_eq!(outcome.new_transactions, 1);
        assert_eq!(outcome.skipped_postings, 0);
        assert_eq!(outcome.created_tags, vec!["reimbursable".to_owned()]);

        assert_eq!(
            tag_names_of_posting(&pool, &bank).await,
            vec!["reimbursable".to_owned()]
        );
        assert!(tag_names_of_posting(&pool, &food).await.is_empty());
    }

    /// `Transaction::effective_tags` already unions the two levels, so they are
    /// persisted separately rather than posting tags being flattened upward.
    #[sqlx::test(migrations = "./migrations")]
    async fn posting_tags_stay_at_the_posting_level(pool: SqlitePool) {
        let (bank, _food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["household".to_owned()])
            .postings(vec![
                RawPosting::builder()
                    .account("Assets:Bank")
                    .maybe_amount(Some(Amount::new(
                        Decimal::from(50_i64),
                        CommodityCode::new("AUD"),
                    )))
                    .tags(vec!["reimbursable".to_owned()])
                    .build(),
            ])
            .build();

        let outcome = run(&svcs, &[raw]).await;

        assert_eq!(
            outcome.created_tags,
            vec!["household".to_owned(), "reimbursable".to_owned()]
        );
        // The transaction carries only its own tag...
        assert_eq!(
            tag_names_of_transaction(&pool).await,
            vec!["household".to_owned()]
        );
        // ...and the posting only its own.
        assert_eq!(
            tag_names_of_posting(&pool, &bank).await,
            vec!["reimbursable".to_owned()]
        );
    }

    /// A re-import of the same document names the same tag again, but
    /// `create_paths` reuses the existing tag rather than minting a duplicate,
    /// so the second run's `created_tags` is empty.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_re_run_creates_no_further_tags(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let raw = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["household".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();
        run(&svcs, std::slice::from_ref(&raw)).await;

        let outcome = run(&svcs, &[raw]).await;

        assert!(outcome.created_tags.is_empty());
        assert_eq!(outcome.new_transactions, 0);
    }

    /// A later import naming a case variant of an already-imported tag resolves
    /// to the existing tag rather than forking a sibling — `eq_name` exercised
    /// through the whole import stack, not just at the `TagService` level (see
    /// `create_paths_maps_case_variants_to_one_tag` in `tag.rs`) or for an
    /// identical spelling (see `a_re_run_creates_no_further_tags` above).
    #[sqlx::test(migrations = "./migrations")]
    async fn a_later_import_folds_a_case_variant_tag(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let first = RawTransaction::builder()
            .date(date(2025, 6, 27))
            .description("groceries")
            .tags(vec!["household".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(50_i64))])
            .build();
        run(&svcs, &[first]).await;

        let second = RawTransaction::builder()
            .date(date(2025, 6, 28))
            .description("more groceries")
            .tags(vec!["Household".to_owned()])
            .postings(vec![leg("Assets:Bank", Some(30_i64))])
            .build();
        let outcome = run(&svcs, &[second]).await;

        assert!(
            outcome.created_tags.is_empty(),
            "a case variant of an existing tag must not be reported as created"
        );
        assert_eq!(outcome.new_transactions, 1);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags")
            .fetch_one(&pool)
            .await
            .expect("count tags");
        assert_eq!(
            count, 1,
            "the case variant must fold onto the one existing tag row"
        );
    }

    /// The warn-once guard collapses two rows naming one missing account into a
    /// single log line, but the report needs both rows, so the diagnostic must
    /// be recorded per occurrence rather than per distinct path.
    #[sqlx::test(migrations = "./migrations")]
    async fn every_occurrence_of_a_missing_account_is_diagnosed(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = vec![
            raw_with(
                "ONE",
                vec![
                    leg("Assets:Bank", Some(-5_i64)),
                    leg("Expenses:Utilities:Gas", Some(5_i64)),
                ],
            ),
            raw_with(
                "TWO",
                vec![
                    leg("Assets:Bank", Some(-7_i64)),
                    leg("Expenses:Utilities:Gas", Some(7_i64)),
                ],
            ),
        ];

        let outcome = run(&svcs, &docs).await;

        let missing: Vec<&Diagnostic> = outcome
            .diagnostics
            .iter()
            .filter(|d| d.cause == SkipCause::UnresolvedAccount)
            .collect();
        assert_eq!(
            missing.len(),
            2,
            "the warn-once guard must not collapse per-row diagnostics"
        );
        assert_eq!(outcome.unresolved_accounts, vec!["Expenses:Utilities:Gas"]);
    }

    /// A path that will not parse is its own cause, distinct from a path that
    /// parses but names nothing.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_malformed_path_is_diagnosed_as_such(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let doc = raw_with(
            "BAD",
            vec![
                leg("Assets:Bank", Some(-5_i64)),
                leg("Assets::Checking", Some(5_i64)),
            ],
        );

        let outcome = run(&svcs, core::slice::from_ref(&doc)).await;

        let causes: Vec<SkipCause> = outcome.diagnostics.iter().map(|d| d.cause).collect();
        assert!(causes.contains(&SkipCause::MalformedPath), "got {causes:?}");
        assert_eq!(outcome.other_skipped_postings, 1);
    }

    /// A row skipped whole for an ambiguous residual is diagnosed once, for the
    /// row, rather than once per leg.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_ambiguous_residual_is_diagnosed_as_such(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let doc = raw_with(
            "TWO ELIDED",
            vec![leg("Assets:Bank", None), leg("Expenses:Food", None)],
        );

        let outcome = run(&svcs, core::slice::from_ref(&doc)).await;

        let causes: Vec<SkipCause> = outcome.diagnostics.iter().map(|d| d.cause).collect();
        assert_eq!(causes, vec![SkipCause::AmbiguousResidual]);
    }

    /// The diagnostic carries the document location, so a report can point the
    /// user at the row rather than only at the cause.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_diagnostic_names_the_document_location(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let doc = raw_with(
            "ONE",
            vec![
                leg("Assets:Bank", Some(-5_i64)),
                leg("Expenses:Utilities:Gas", Some(5_i64)),
            ],
        );

        let outcome = run(&svcs, core::slice::from_ref(&doc)).await;

        let first = outcome.diagnostics.first().expect("one diagnostic");
        assert_eq!(first.location, location_of(&doc));
    }

    // MARK: Dry run

    /// Plans an import with no profile, under the "test" importer name.
    async fn plan(svcs: &Services, raws: &[RawTransaction]) -> ImportPlan {
        plan_import(
            &svcs.transactions,
            &svcs.sources,
            &svcs.accounts,
            &svcs.commodities,
            &svcs.tags,
            &svcs.batches,
            None,
            "test",
            raws,
        )
        .await
        .expect("the plan")
    }

    /// Snapshots every table an import can write, for a writes-nothing assertion.
    async fn table_counts(pool: &SqlitePool) -> Vec<(&'static str, i64)> {
        let mut out = Vec::new();
        for (table, query) in [
            ("transactions", "SELECT COUNT(*) FROM transactions"),
            ("postings", "SELECT COUNT(*) FROM postings"),
            (
                "transaction_sources",
                "SELECT COUNT(*) FROM transaction_sources",
            ),
            ("tags", "SELECT COUNT(*) FROM tags"),
            ("import_batches", "SELECT COUNT(*) FROM import_batches"),
        ] {
            let count: i64 = sqlx::query_scalar(query)
                .fetch_one(pool)
                .await
                .expect("the count");
            out.push((table, count));
        }
        out
    }

    /// Documents spanning every branch the plan must predict: a clean create, a
    /// missing account, an unregistered commodity, a malformed path, and two
    /// elided legs.
    fn interesting_documents() -> Vec<RawTransaction> {
        // Tags are the one thing the two sinks reach different services for —
        // one creates the paths, the other only resolves them — so a fixture
        // naming none would leave the field most worth comparing compared as
        // empty against empty.
        let mut coffee = raw("COFFEE", -5_i64);
        coffee.tags = vec!["holiday".to_owned()];
        vec![
            coffee,
            raw_with(
                "MISSING",
                vec![
                    leg("Assets:Bank", Some(-5_i64)),
                    leg("Expenses:Utilities:Gas", Some(5_i64)),
                ],
            ),
            raw_with(
                "UNREGISTERED",
                vec![
                    leg("Assets:Bank", Some(-6_i64)),
                    coded_leg("Expenses:Food", dec!(6), "DOGE"),
                ],
            ),
            raw_with(
                "BADPATH",
                vec![
                    leg("Assets:Bank", Some(-5_i64)),
                    leg("Assets::Checking", Some(5_i64)),
                ],
            ),
            raw_with(
                "TWOELIDED",
                vec![leg("Assets:Bank", None), leg("Expenses:Food", None)],
            ),
        ]
    }

    /// The whole point of a dry run: it touches no table an import writes.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plan_writes_nothing(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let mut tagged = raw_with(
            "TAGGED",
            vec![
                leg("Assets:Bank", Some(-9_i64)),
                leg("Expenses:Food", Some(9_i64)),
            ],
        );
        tagged.tags = vec!["holiday".to_owned()];
        let docs = vec![raw("COFFEE", -5_i64), tagged];
        let before = table_counts(&pool).await;

        plan(&svcs, &docs).await;

        assert_eq!(table_counts(&pool).await, before);
    }

    /// The load-bearing claim of the feature: a plan is the run it predicts,
    /// with the terminal writes diverted. Every count, both worklists, the tag
    /// list and the diagnostics must agree, because no decision in a run
    /// observes a write that run made.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plan_predicts_what_the_run_does(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = interesting_documents();

        let planned = plan(&svcs, &docs).await;
        let outcome = run(&svcs, &docs).await;

        assert_eq!(planned.new_transactions, outcome.new_transactions);
        assert_eq!(planned.attached_postings, outcome.attached_postings);
        assert_eq!(planned.skipped_postings, outcome.skipped_postings);
        assert_eq!(
            planned.unresolved_account_postings,
            outcome.unresolved_account_postings
        );
        assert_eq!(
            planned.unresolved_commodity_postings,
            outcome.unresolved_commodity_postings
        );
        assert_eq!(
            planned.other_skipped_postings,
            outcome.other_skipped_postings
        );
        assert_eq!(planned.unresolved_accounts, outcome.unresolved_accounts);
        assert_eq!(
            planned.unresolved_commodities,
            outcome.unresolved_commodities
        );
        assert_eq!(planned.would_create_tags, outcome.created_tags);
        assert_eq!(planned.charged_by_cause, outcome.charged_by_cause);
        assert_eq!(planned.diagnostics, outcome.diagnostics);

        assert_eq!(planned.new_transactions, 4, "the fixture must create rows");
        assert_eq!(
            planned.would_create_tags,
            vec!["holiday".to_owned()],
            "the fixture must name a tag, or the tag equality above is vacuous — and it is \
             the one field the two sinks reach different services for"
        );
        assert_eq!(
            (
                planned.unresolved_account_postings,
                planned.unresolved_commodity_postings,
                planned.other_skipped_postings
            ),
            (1, 1, 3),
            "every skip bucket must be exercised, or the equality above is vacuous"
        );
        assert_eq!(
            planned.charged_by_cause,
            vec![
                (SkipCause::UnresolvedAccount, 1),
                (SkipCause::UnresolvedCommodity, 1),
                (SkipCause::MalformedPath, 1),
                (SkipCause::AmbiguousResidual, 2),
            ],
            "TWOELIDED charges both of its legs to one diagnostic, so this must diverge from \
             counting diagnostics per cause, or the equality above is vacuous"
        );
    }

    /// Every charge passes through `Counts::charge`, which increments the
    /// coarse column and the cause's own line together. A charge that reached
    /// only one of the two would leave the report's headline split disagreeing
    /// with the breakdown beneath it, and this is what notices.
    ///
    /// The equality is exact rather than approximate: a
    /// [`SkipCause::MalformedTag`] is only ever *noted*, never charged, so it
    /// costs no posting and enters neither tally.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_per_cause_tally_sums_to_the_coarse_total(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let mut docs = interesting_documents();
        let mut malformed = raw("BADTAG", -3_i64);
        malformed.tags = vec!["person::alpha".to_owned()];
        docs.push(malformed);

        let outcome = run(&svcs, &docs).await;

        let per_cause: usize = outcome
            .charged_by_cause
            .iter()
            .map(|&(_, postings)| postings)
            .sum();
        assert_eq!(
            per_cause, outcome.skipped_postings,
            "the breakdown must account for exactly what the coarse split does"
        );
        assert!(
            outcome.skipped_postings > 0,
            "the fixture must skip something, or the equality above is vacuous"
        );
        assert!(
            outcome
                .diagnostics
                .iter()
                .any(|entry| entry.cause == SkipCause::MalformedTag),
            "the fixture must raise a malformed tag, or the exactness is untested"
        );
        assert!(
            !outcome
                .charged_by_cause
                .iter()
                .any(|&(cause, _)| cause == SkipCause::MalformedTag),
            "a malformed tag is noted, never charged, so it must not enter the tally"
        );
    }

    /// The attach branch is only reachable once an earlier run has left a
    /// transaction behind, so it needs its own fixture: import with the account
    /// missing, create it, then plan the same document again.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plan_predicts_an_attach_against_a_partial_first_run(pool: SqlitePool) {
        bank_only_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = vec![raw_with(
            "SPLIT",
            vec![
                leg("Assets:Bank", Some(-5_i64)),
                leg("Expenses:Food", Some(5_i64)),
            ],
        )];
        run(&svcs, &docs).await;
        add_food(&pool).await;

        let planned = plan(&svcs, &docs).await;
        let outcome = run(&svcs, &docs).await;

        assert_eq!(
            planned.attached_postings, 1,
            "the second pass attaches the leg"
        );
        assert_eq!(planned.attached_postings, outcome.attached_postings);
    }

    /// The per-account totals are the report's headline figure, so they must sum
    /// every leg that would post rather than only the last row's.
    #[sqlx::test(migrations = "./migrations")]
    async fn account_totals_sum_the_legs_that_would_post(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = vec![raw("COFFEE", -5_i64), raw("LUNCH", -7_i64)];

        let planned = plan(&svcs, &docs).await;

        let checking = planned
            .account_totals
            .iter()
            .find(|(path, _)| path == "Assets:Bank")
            .map(|(_, balances)| balances.get("AUD"))
            .expect("the account posts");
        assert_eq!(checking, Some(dec!(-12)));
    }

    /// `RowLocalFailure` covers the write, which a plan skips, but also the
    /// steps above the sink, which it does not. Summing a row's amounts is one
    /// of those, so an overflow there is a skip the plan is obliged to predict —
    /// the run reaches it before the sink is ever consulted.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plan_predicts_a_row_local_failure_raised_above_the_sink(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        // The two concrete legs resolve to nothing, so the elided leg is the
        // only one left and the residual is taken from the document — where the
        // two amounts sum past `Decimal`'s range.
        let doc = raw_with(
            "OVERFLOW",
            vec![
                leg("Assets:Bank", None),
                RawPosting::builder()
                    .account("Nowhere:One")
                    .maybe_amount(Some(Amount::new(Decimal::MAX, CommodityCode::new("AUD"))))
                    .build(),
                RawPosting::builder()
                    .account("Nowhere:Two")
                    .maybe_amount(Some(Amount::new(Decimal::MAX, CommodityCode::new("AUD"))))
                    .build(),
            ],
        );

        let planned = plan(&svcs, core::slice::from_ref(&doc)).await;
        let outcome = run(&svcs, core::slice::from_ref(&doc)).await;

        assert!(
            planned
                .charged_by_cause
                .iter()
                .any(|(cause, _)| *cause == SkipCause::RowLocalFailure),
            "the plan reports the overflow, which happens above the sink"
        );
        assert_eq!(
            planned.charged_by_cause, outcome.charged_by_cause,
            "and charges it exactly as the run does"
        );
    }

    /// A real run creates the tags it names before writing a row; a plan must
    /// report the same list without bringing any of them into existence.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_plan_creates_no_tags(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let mut doc = raw("COFFEE", -5_i64);
        doc.tags = vec!["holiday".to_owned()];

        let planned = plan(&svcs, core::slice::from_ref(&doc)).await;

        assert_eq!(planned.would_create_tags, vec!["holiday".to_owned()]);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags")
            .fetch_one(&pool)
            .await
            .expect("the count");
        assert_eq!(count, 0);
    }

    /// An adoption is the one thing the plan sink is handed and deliberately
    /// discards: the posting is already the user's own, so the run records
    /// provenance against it rather than writing anything, and no balance
    /// moves. It still counts as an attachment, and it still feeds the
    /// residual, because its posting is among the stored ones.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_adoption_is_counted_but_moves_no_total(pool: SqlitePool) {
        let food = add_food(&pool).await;
        let svcs = services(&pool).await;
        let document = raw_with(
            "COFFEE",
            vec![leg("Expenses:Food", Some(50_i64)), leg("Assets:Bank", None)],
        );
        run(&svcs, core::slice::from_ref(&document)).await;

        // The user creates the account and hand-writes the missing leg, which
        // carries no provenance — the obvious response to a partial import.
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
                .account_id(bank)
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
        let before = table_counts(&pool).await;

        let planned = plan(&svcs, core::slice::from_ref(&document)).await;

        assert_eq!(
            planned.attached_postings, 1,
            "the leg is accounted for, by adoption rather than insertion"
        );
        assert_eq!(
            bucket(&planned, "Assets:Bank"),
            Some(&Balances::new()),
            "the account is touched, but the user's own posting already holds its value"
        );
        assert_eq!(
            table_counts(&pool).await,
            before,
            "planning an adoption still writes nothing"
        );
    }

    /// Returns the bucket `path` holds in `planned`, or `None` if the plan never
    /// touched that account.
    fn bucket<'plan>(planned: &'plan ImportPlan, path: &str) -> Option<&'plan Balances> {
        planned
            .account_totals
            .iter()
            .find(|(account, _)| account == path)
            .map(|(_, balances)| balances)
    }

    /// An elided leg is not weightless: the balance engine derives its value
    /// from its siblings, so a plan that left the bucket empty would report an
    /// account as unmoved when the import is about to move it.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_elided_leg_posts_the_rows_residual(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = vec![split_raw("GROCERIES")];

        let planned = plan(&svcs, &docs).await;

        assert_eq!(
            bucket(&planned, "Expenses:Food").and_then(|b| b.get("AUD")),
            Some(dec!(50)),
            "the concrete leg posts what it states"
        );
        assert_eq!(
            bucket(&planned, "Assets:Bank").and_then(|b| b.get("AUD")),
            Some(dec!(-50)),
            "the elided leg absorbs the residual of its siblings"
        );
    }

    /// The figure the report prints must be the movement the user will actually
    /// see, so it is pinned against the balance engine's own reading of the
    /// ledger after the same documents are imported for real.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_planned_total_matches_the_balance_the_run_leaves(pool: SqlitePool) {
        let (bank, food) = two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = vec![split_raw("GROCERIES")];

        let planned = plan(&svcs, &docs).await;
        run(&svcs, &docs).await;

        let engine = crate::BalanceEngine::new(pool.clone());
        for (account, path) in [(&bank, "Assets:Bank"), (&food, "Expenses:Food")] {
            let actual = engine
                .balance_for(account, "AUD")
                .await
                .expect("the balance");
            assert_eq!(
                bucket(&planned, path).and_then(|b| b.get("AUD")),
                Some(actual.value()),
                "the plan's figure for {path} must be the balance the run leaves"
            );
        }
    }

    /// The same guarantee on the attach path, where the elided leg is appended
    /// to a transaction an earlier run created and its residual is fixed by
    /// siblings this run does not write. Deliberately phrased in balances
    /// rather than residuals: it asks only whether the figure the user is shown
    /// is the figure they get.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_planned_total_matches_the_balance_an_attach_leaves(pool: SqlitePool) {
        ensure_path(&pool, "Expenses:Food").await;
        let svcs = services(&pool).await;
        let docs = vec![split_raw("GROCERIES")];
        run(&svcs, &docs).await;
        let bank = ensure_path(&pool, "Assets:Bank").await;

        let planned = plan(&svcs, &docs).await;
        run(&svcs, &docs).await;

        assert_eq!(
            planned.attached_postings, 1,
            "the elided leg must reach the attach path, not the create path"
        );
        let actual = crate::BalanceEngine::new(pool.clone())
            .balance_for(&bank, "AUD")
            .await
            .expect("the balance");
        assert_eq!(
            actual.value(),
            dec!(-50),
            "the appended leg moves the account"
        );
        assert_eq!(
            bucket(&planned, "Assets:Bank").and_then(|b| b.get("AUD")),
            Some(actual.value()),
            "the plan's figure must be the balance the attach leaves"
        );
    }

    /// The other orientation of the attach path, and the one that writes no
    /// posting at all for the account that moves: the elided leg is *already*
    /// stored, and this run appends a sibling to it. Its derived value shifts,
    /// so the account moves without this run booking anything to it.
    ///
    /// This is the workflow the feature exists for — create the missing
    /// account, re-run, watch the rest land — so a report that omitted the
    /// account would omit it exactly when the user is looking for it.
    #[sqlx::test(migrations = "./migrations")]
    async fn the_planned_total_matches_a_shift_in_a_stored_legs_residual(pool: SqlitePool) {
        ensure_path(&pool, "Expenses:Food").await;
        let bank = ensure_path(&pool, "Assets:Bank").await;
        let svcs = services(&pool).await;
        let docs = vec![raw_with(
            "TRIP",
            vec![
                coded_leg("Expenses:Food", dec!(30), "AUD"),
                coded_leg("Expenses:Travel", dec!(20), "AUD"),
                leg("Assets:Bank", None),
            ],
        )];
        run(&svcs, &docs).await;
        let engine = crate::BalanceEngine::new(pool.clone());
        let opening = engine
            .balance_for(&bank, "AUD")
            .await
            .expect("the balance")
            .value();
        assert_eq!(opening, dec!(-30), "the first pass derives -30 for the leg");

        ensure_path(&pool, "Expenses:Travel").await;
        let planned = plan(&svcs, &docs).await;
        run(&svcs, &docs).await;

        let closing = engine
            .balance_for(&bank, "AUD")
            .await
            .expect("the balance")
            .value();
        assert_eq!(
            closing,
            dec!(-50),
            "appending the sibling moves the account"
        );
        let movement = closing.checked_sub(opening).expect("a small difference");
        assert_eq!(
            bucket(&planned, "Assets:Bank").and_then(|held| held.get("AUD")),
            Some(movement),
            "the plan must report the movement, not the whole balance"
        );
    }

    /// The residual is per-commodity, so an elided leg beside siblings in two
    /// commodities absorbs a bucket in each rather than a single scalar.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_elided_legs_residual_is_per_commodity(pool: SqlitePool) {
        ensure_path(&pool, "Assets:Bank").await;
        ensure_path(&pool, "Expenses:Food").await;
        ensure_path(&pool, "Expenses:Travel").await;
        let svcs = services(&pool).await;
        let docs = vec![raw_with(
            "TRIP",
            vec![
                coded_leg("Expenses:Food", dec!(30), "AUD"),
                coded_leg("Expenses:Travel", dec!(20), "USD"),
                leg("Assets:Bank", None),
            ],
        )];

        let planned = plan(&svcs, &docs).await;

        let bank = bucket(&planned, "Assets:Bank").expect("the elided leg's account");
        assert_eq!(bank.get("AUD"), Some(dec!(-30)));
        assert_eq!(bank.get("USD"), Some(dec!(-20)));
    }

    /// An account touched in two commodities holds a bucket for each rather
    /// than collapsing them into one figure.
    #[sqlx::test(migrations = "./migrations")]
    async fn an_account_touched_twice_holds_a_bucket_per_commodity(pool: SqlitePool) {
        ensure_path(&pool, "Assets:Bank").await;
        let svcs = services(&pool).await;
        let docs = vec![
            raw_with("ONE", vec![coded_leg("Assets:Bank", dec!(-5), "AUD")]),
            raw_with("TWO", vec![coded_leg("Assets:Bank", dec!(-7), "USD")]),
        ];

        let planned = plan(&svcs, &docs).await;

        let bank = bucket(&planned, "Assets:Bank").expect("the account posts");
        assert_eq!(bank.get("AUD"), Some(dec!(-5)));
        assert_eq!(bank.get("USD"), Some(dec!(-7)));
    }

    /// An account whose legs cancel out was still touched. It must keep an empty
    /// bucket, so a report can distinguish it from an account the run never
    /// names at all.
    #[sqlx::test(migrations = "./migrations")]
    async fn a_zero_netting_account_keeps_an_empty_bucket(pool: SqlitePool) {
        two_account_tree(&pool).await;
        let svcs = services(&pool).await;
        let docs = vec![raw("IN", 5_i64), raw("OUT", -5_i64)];

        let planned = plan(&svcs, &docs).await;

        assert_eq!(
            bucket(&planned, "Assets:Bank"),
            Some(&Balances::new()),
            "the account is touched, and nets to nothing"
        );
        assert_eq!(
            bucket(&planned, "Expenses:Food"),
            None,
            "an account no leg names holds no bucket at all"
        );
    }
}
