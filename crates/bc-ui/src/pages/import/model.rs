//! Leptos-free state and text for the import page, so it runs under a native
//! `cargo nextest`.

use bc_ipc::BcError;
use bc_ipc::CauseCount;
use bc_ipc::CommitResult;
use bc_ipc::DeclarationKindInfo;
use bc_ipc::DeclaredAccountInfo;
use bc_ipc::DiagnosticInfo;
use bc_ipc::FailureStageInfo;
use bc_ipc::ImportFailure;
use bc_ipc::ImportPreview;
use bc_ipc::ImportProfileInfo;
use bc_ipc::ImportProfiles;
use bc_ipc::ImportResult;
use bc_ipc::LegFateInfo;
use bc_ipc::PreviewLeg;
use bc_ipc::PreviewResult;
use bc_ipc::PreviewRow;
use bc_ipc::RowFateInfo;

use crate::components::status_pill::Tone;

/// The text for a reply variant this build does not know.
pub(crate) const UNKNOWN_REPLY: &str =
    "The server sent a reply this page does not understand. Reload the page.";

/// Why a profile cannot preview.
///
/// # Arguments
///
/// * `profile` - The profile to check.
/// * `documents_root_set` - Whether `import.documents-root` is configured.
///
/// # Returns
///
/// The reason shown beside its disabled Preview button, or `None` when it can run.
#[must_use]
pub(crate) fn block_reason(
    profile: &ImportProfileInfo,
    documents_root_set: bool,
) -> Option<String> {
    if !documents_root_set {
        Some("import.documents-root is not set".to_owned())
    } else if !profile.installed {
        Some(format!("importer `{}` not installed", profile.importer))
    } else {
        None
    }
}

/// The profile a `?profile=` query asks to preview on load.
///
/// # Arguments
///
/// * `wanted` - The query value, if any.
/// * `profiles` - The loaded profile list.
///
/// # Returns
///
/// The profile's name when it exists and can run, else `None`.
#[must_use]
pub(crate) fn auto_start_target(wanted: Option<&str>, profiles: &ImportProfiles) -> Option<String> {
    let name = wanted?;
    profiles
        .profiles
        .iter()
        .find(|p| p.name == name && block_reason(p, profiles.documents_root_set).is_none())
        .map(|p| p.name.clone())
}

/// Where the open profile's run stands. `ImportCtx::active` names the profile.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RunState {
    /// No run; no panel content.
    Idle,
    /// `preview_import` is in flight.
    Previewing,
    /// A preview to review.
    Previewed {
        /// The preview.
        preview: Box<ImportPreview>,
        /// Whether a commit found the source changed and returned this preview.
        changed: bool,
    },
    /// `commit_import` is in flight for this preview.
    Committing {
        /// The preview being committed.
        preview: Box<ImportPreview>,
    },
    /// `release_rejected_sources` is in flight from this preview, which a
    /// fresh preview replaces once it succeeds.
    Releasing {
        /// The preview the release started from.
        preview: Box<ImportPreview>,
    },
    /// The commit wrote a batch.
    Done(Box<ImportResult>),
    /// The run failed with a stage and message.
    Failed(ImportFailure),
    /// The call itself failed.
    Error(String),
}

impl RunState {
    /// Whether a call is in flight; every Preview, Commit and re-import
    /// button disables while one is.
    #[must_use]
    pub(crate) fn busy(&self) -> bool {
        matches!(
            self,
            Self::Previewing | Self::Committing { .. } | Self::Releasing { .. }
        )
    }

    /// Whether a commit reply that led to this state may have written, so
    /// History and the shared stores need a refetch.
    ///
    /// True for a commit, a failure that names its partial batch, and an
    /// error: a call that failed in transport may still have committed on the
    /// server. Meaningful only for the state [`after_commit`] returns.
    #[must_use]
    pub(crate) fn refresh_after_commit(&self) -> bool {
        match self {
            Self::Done(_) | Self::Error(_) => true,
            Self::Failed(failure) => failure.batch_id.is_some(),
            Self::Idle
            | Self::Previewing
            | Self::Previewed { .. }
            | Self::Committing { .. }
            | Self::Releasing { .. } => false,
        }
    }
}

/// The state a `preview_import` reply leads to.
///
/// # Arguments
///
/// * `result` - The reply.
///
/// # Returns
///
/// `Previewed`, `Failed` or `Error`.
#[must_use]
pub(crate) fn after_preview(result: Result<PreviewResult, BcError>) -> RunState {
    match result {
        Ok(PreviewResult::Ready(preview)) => RunState::Previewed {
            preview: Box::new(preview),
            changed: false,
        },
        Ok(PreviewResult::Failed(failure)) => RunState::Failed(failure),
        Ok(_) => RunState::Error(UNKNOWN_REPLY.to_owned()),
        Err(e) => RunState::Error(e.to_string()),
    }
}

/// One sentence naming a failed run's stage and message.
///
/// # Arguments
///
/// * `failure` - The failure.
///
/// # Returns
///
/// `"Import failed at the <stage> stage: <message>"`.
#[must_use]
pub(crate) fn failure_text(failure: &ImportFailure) -> String {
    format!(
        "Import failed at the {} stage: {}",
        stage_label(&failure.stage),
        failure.message
    )
}

/// The word naming a failure stage.
fn stage_label(stage: &FailureStageInfo) -> &'static str {
    match stage {
        FailureStageInfo::UnknownImporter => "unknown-importer",
        FailureStageInfo::Importer => "importer",
        FailureStageInfo::Engine => "engine",
        FailureStageInfo::SourceChanged => "source-changed",
        _ => "unrecognised",
    }
}

/// Rows the rows table renders per "Show more".
pub(crate) const PAGE: usize = 200;

/// The skip label `bc_core::SkipCause::UnresolvedAccount` carries.
const UNRESOLVED_ACCOUNT: &str = "unresolved account";

/// The skip label `bc_core::SkipCause::UnresolvedCommodity` carries.
const UNREGISTERED_COMMODITY: &str = "unregistered commodity";

/// `"<n> <noun>"`, with the singular noun when `n` is one.
///
/// # Arguments
///
/// * `n` - The count.
/// * `one` - The noun for exactly one.
/// * `many` - The noun for any other count.
///
/// # Returns
///
/// The counted noun, such as `"3 postings"`.
#[must_use]
pub(crate) fn count_noun<N>(n: &N, one: &str, many: &str) -> String
where
    N: core::fmt::Display + PartialEq + From<u8>,
{
    let word = if *n == N::from(1_u8) { one } else { many };
    format!("{n} {word}")
}

/// `"<n> leg(s) skipped"`.
///
/// # Arguments
///
/// * `n` - Skipped legs.
///
/// # Returns
///
/// The phrase.
#[must_use]
pub(crate) fn legs_skipped(n: u64) -> String {
    count_noun(&n, "leg skipped", "legs skipped")
}

/// Whether committing this preview would write anything.
///
/// # Arguments
///
/// * `p` - The preview.
///
/// # Returns
///
/// `true` when it would create a transaction, attach a leg, or apply an
/// account declaration.
#[must_use]
pub(crate) fn would_write(p: &ImportPreview) -> bool {
    p.new_transactions > 0
        || p.attached_postings > 0
        || !p.declared.is_empty()
        || !p.would_create_accounts.is_empty()
}

/// The account opens and closes a commit would apply, as a count.
fn declarations(p: &ImportPreview) -> u64 {
    u64::try_from(p.declared.len()).unwrap_or(u64::MAX)
}

/// The preview's one-line summary: the counts that are not zero, led by
/// "Nothing to import" when no row would write.
///
/// # Arguments
///
/// * `p` - The preview.
///
/// # Returns
///
/// Such as `"42 new · 3 attach legs · 2 account declarations · 118 already
/// imported · 7 legs skipped"`.
#[must_use]
pub(crate) fn summary_line(p: &ImportPreview) -> String {
    let lead = (!would_write(p)).then(|| "Nothing to import".to_owned());
    let counts = [
        (p.new_transactions, "new", "new"),
        (p.attached_postings, "attach leg", "attach legs"),
        (
            declarations(p),
            "account declaration",
            "account declarations",
        ),
        (p.already_imported, "already imported", "already imported"),
        (p.skipped_postings, "leg skipped", "legs skipped"),
        (
            p.skipped_rejected,
            "rejected leg skipped",
            "rejected legs skipped",
        ),
    ]
    .into_iter()
    .filter(|&(n, _, _)| n > 0)
    .map(|(n, one, many)| count_noun(&n, one, many));
    lead.into_iter()
        .chain(counts)
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// The CLI command that creates `path`, quoted for a POSIX shell.
///
/// # Arguments
///
/// * `path` - The account path as the document states it.
///
/// # Returns
///
/// Such as `"borrow-checker account create 'Assets:Import Test:Checking'"`.
#[must_use]
pub(crate) fn account_create_command(path: &str) -> String {
    let bare = !path.is_empty()
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-' | b'.' | b'/'));
    let arg = if bare {
        path.to_owned()
    } else {
        format!("'{}'", path.replace('\'', r"'\''"))
    };
    format!("borrow-checker account create {arg}")
}

/// Skip causes other than the unresolved account and commodity, which the
/// blockers list item by item.
///
/// # Arguments
///
/// * `causes` - The preview's skips by cause.
///
/// # Returns
///
/// The remaining causes, in their given order.
#[must_use]
pub(crate) fn other_causes(causes: &[CauseCount]) -> Vec<CauseCount> {
    causes
        .iter()
        .filter(|c| c.cause != UNRESOLVED_ACCOUNT && c.cause != UNREGISTERED_COMMODITY)
        .cloned()
        .collect()
}

/// A row's fate without its payload: the unit the filter chips toggle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FateKind {
    /// A new transaction.
    Create,
    /// New legs onto an existing transaction.
    Attach,
    /// Every resolved leg already stored.
    AlreadyImported,
    /// Legs owned by several transactions.
    Conflict,
    /// No leg reached the writer.
    Skipped,
    /// The user deleted the transaction built from the row.
    Rejected,
}

impl FateKind {
    /// Every kind, in chip order.
    pub(crate) const ALL: [Self; 6] = [
        Self::Create,
        Self::Attach,
        Self::AlreadyImported,
        Self::Conflict,
        Self::Skipped,
        Self::Rejected,
    ];

    /// The kind of `fate`; a variant this build does not know reads as skipped.
    #[must_use]
    pub(crate) fn of(fate: &RowFateInfo) -> Self {
        match fate {
            RowFateInfo::Create => Self::Create,
            RowFateInfo::Attach { .. } => Self::Attach,
            RowFateInfo::AlreadyImported { .. } => Self::AlreadyImported,
            RowFateInfo::Conflict { .. } => Self::Conflict,
            RowFateInfo::Skipped { .. } => Self::Skipped,
            RowFateInfo::Rejected { .. } => Self::Rejected,
            #[expect(
                clippy::match_same_arms,
                reason = "named for clarity beside the unknown-variant fallback"
            )]
            _ => Self::Skipped,
        }
    }

    /// The chip and pill label.
    #[must_use]
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Attach => "attach",
            Self::AlreadyImported => "already imported",
            Self::Conflict => "conflict",
            Self::Skipped => "skipped",
            Self::Rejected => "rejected",
        }
    }

    /// The `data-fate` value and test-id suffix.
    #[must_use]
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Attach => "attach",
            Self::AlreadyImported => "already_imported",
            Self::Conflict => "conflict",
            Self::Skipped => "skipped",
            Self::Rejected => "rejected",
        }
    }

    /// The pill tone.
    #[must_use]
    pub(crate) fn tone(self) -> Tone {
        match self {
            Self::Create | Self::Attach => Tone::Good,
            Self::AlreadyImported | Self::Rejected => Tone::Muted,
            Self::Conflict => Tone::Bad,
            Self::Skipped => Tone::Warn,
        }
    }

    /// This kind's bit in a [`FateFilter`].
    fn bit(self) -> u8 {
        match self {
            Self::Create => 1,
            Self::Attach => 2,
            Self::AlreadyImported => 4,
            Self::Conflict => 8,
            Self::Skipped => 16,
            Self::Rejected => 32,
        }
    }
}

/// Which fates the rows table shows, one bit per [`FateKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FateFilter(u8);

impl Default for FateFilter {
    /// Every fate but the settled ones: already imported, and rejected.
    fn default() -> Self {
        Self(
            FateKind::ALL
                .into_iter()
                .filter(|&k| k != FateKind::AlreadyImported && k != FateKind::Rejected)
                .fold(0_u8, |mask, k| mask | k.bit()),
        )
    }
}

impl FateFilter {
    /// Whether rows of `kind` show.
    #[must_use]
    pub(crate) fn contains(self, kind: FateKind) -> bool {
        self.0 & kind.bit() != 0
    }

    /// Shows `kind` if hidden, hides it if shown.
    pub(crate) fn toggle(&mut self, kind: FateKind) {
        self.0 ^= kind.bit();
    }
}

/// Rows per chip, in [`FateKind::ALL`] order, zeros included.
///
/// Each chip counts the rows it shows, by [`shows_under`]. A row with a
/// skipped or rejected leg counts under its own chip and that leg's, so the
/// counts can sum to more than the row total.
///
/// # Arguments
///
/// * `rows` - The preview's rows.
///
/// # Returns
///
/// One `(kind, count)` per kind.
#[must_use]
pub(crate) fn fate_counts(rows: &[PreviewRow]) -> Vec<(FateKind, usize)> {
    FateKind::ALL
        .into_iter()
        .map(|kind| {
            let n = rows.iter().filter(|r| shows_under(r, kind)).count();
            (kind, n)
        })
        .collect()
}

/// Whether `kind`'s chip reveals `row`.
///
/// A chip reveals the rows of its fate. Skipped also reveals a row of any
/// fate with a skipped leg, and Rejected one with a leg the user deleted, so
/// an already-imported row missing a leg stays in view.
///
/// # Arguments
///
/// * `row` - The parsed row.
/// * `kind` - The chip.
///
/// # Returns
///
/// `true` when the chip shows the row.
fn shows_under(row: &PreviewRow, kind: FateKind) -> bool {
    FateKind::of(&row.fate) == kind
        || match kind {
            FateKind::Skipped => row
                .legs
                .iter()
                .any(|leg| matches!(leg.fate, LegFateInfo::Skipped { .. })),
            FateKind::Rejected => row
                .legs
                .iter()
                .any(|leg| matches!(leg.fate, LegFateInfo::Rejected)),
            FateKind::Create
            | FateKind::Attach
            | FateKind::AlreadyImported
            | FateKind::Conflict => false,
        }
}

/// The slice of rows the table renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Visible {
    /// Indices into the preview's rows, in source order.
    pub indices: Vec<usize>,
    /// Filtered rows past the limit.
    pub remaining: usize,
}

/// The first `limit` rows `filter` shows: those any enabled chip reveals, by
/// [`shows_under`].
///
/// # Arguments
///
/// * `rows` - The preview's rows.
/// * `filter` - The fates shown.
/// * `limit` - How many rows to render.
///
/// # Returns
///
/// The indices to render and how many more the filter matches.
#[must_use]
pub(crate) fn visible_rows(rows: &[PreviewRow], filter: FateFilter, limit: usize) -> Visible {
    let matching: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, r)| {
            FateKind::ALL
                .into_iter()
                .any(|kind| filter.contains(kind) && shows_under(r, kind))
        })
        .map(|(i, _)| i)
        .collect();
    let remaining = matching.len().saturating_sub(limit);
    Visible {
        indices: matching.into_iter().take(limit).collect(),
        remaining,
    }
}

/// A row's fate pill: its label and tone.
///
/// # Arguments
///
/// * `fate` - The row's fate.
///
/// # Returns
///
/// Such as `("skipped: unresolved account", Tone::Warn)`.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RowFateInfo is non_exhaustive"
)]
pub(crate) fn fate_pill(fate: &RowFateInfo) -> (String, Tone) {
    let kind = FateKind::of(fate);
    let label = match fate {
        RowFateInfo::Skipped { cause } => format!("skipped: {cause}"),
        RowFateInfo::Conflict { owners } => {
            format!("conflict: {}", count_noun(&owners.len(), "owner", "owners"))
        }
        _ => kind.label().to_owned(),
    };
    (label, kind.tone())
}

/// The transaction an attach or already-imported row points at.
///
/// # Arguments
///
/// * `fate` - The row's fate.
///
/// # Returns
///
/// The owner's ID, or `None` for any other fate.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RowFateInfo is non_exhaustive"
)]
pub(crate) fn fate_owner(fate: &RowFateInfo) -> Option<String> {
    match fate {
        RowFateInfo::Attach { owner } | RowFateInfo::AlreadyImported { owner } => {
            Some(owner.clone())
        }
        _ => None,
    }
}

/// Why a leg is not written, for the struck-through leg.
///
/// # Arguments
///
/// * `fate` - The leg's fate.
///
/// # Returns
///
/// Such as `"skipped: unresolved account"` or `"rejected"`, or `None` for a
/// leg that writes or is stored.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "LegFateInfo is non_exhaustive"
)]
pub(crate) fn leg_skip_note(fate: &LegFateInfo) -> Option<String> {
    match fate {
        LegFateInfo::Skipped { cause } => Some(format!("skipped: {cause}")),
        LegFateInfo::Rejected => Some("rejected".to_owned()),
        _ => None,
    }
}

/// The line a commit outcome adds for the legs it kept out because the user
/// deleted them.
///
/// # Arguments
///
/// * `n` - Rejected legs skipped.
///
/// # Returns
///
/// Such as `"2 rejected legs skipped (deleted earlier)"`, or `None` for
/// none.
#[must_use]
pub(crate) fn rejected_line(n: u64) -> Option<String> {
    (n > 0).then(|| {
        format!(
            "{} (deleted earlier)",
            count_noun(&n, "rejected leg skipped", "rejected legs skipped")
        )
    })
}

/// What opening an owning transaction does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OwnerTarget {
    /// Go to the register of this account.
    Register(String),
    /// The transaction has no postings, so no register to open.
    NotFound,
    /// The lookup failed; the text goes in a toast.
    Failed(String),
}

/// The outcome of looking up an owning transaction.
///
/// # Arguments
///
/// * `reply` - The first posting's account ID, or the lookup's error.
///
/// # Returns
///
/// The register to open, or why none opens.
#[must_use]
pub(crate) fn owner_target(reply: Result<Option<String>, BcError>) -> OwnerTarget {
    match reply {
        Ok(Some(account)) => OwnerTarget::Register(format!("/accounts/{account}")),
        Ok(None) => OwnerTarget::NotFound,
        Err(e) => OwnerTarget::Failed(e.to_string()),
    }
}

/// A diagnostic listed under "Other diagnostics": its location, cause and detail.
///
/// # Arguments
///
/// * `d` - The diagnostic.
///
/// # Returns
///
/// `"<location>: <cause>: <detail>"`, without the location when it is empty.
#[must_use]
pub(crate) fn diagnostic_text(d: &DiagnosticInfo) -> String {
    if d.location.is_empty() {
        row_diagnostic_text(d)
    } else {
        format!("{}: {}: {}", d.location, d.cause, d.detail)
    }
}

/// A diagnostic under its row, which already shows the location.
///
/// # Arguments
///
/// * `d` - The diagnostic.
///
/// # Returns
///
/// `"<cause>: <detail>"`.
#[must_use]
pub(crate) fn row_diagnostic_text(d: &DiagnosticInfo) -> String {
    format!("{}: {}", d.cause, d.detail)
}

/// The id that releases a rejected row: its deleted transaction.
///
/// # Arguments
///
/// * `fate` - The row's fate.
///
/// # Returns
///
/// The `transaction_…` id for a rejected row, else `None`.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "RowFateInfo is non_exhaustive"
)]
pub(crate) fn row_release_target(fate: &RowFateInfo) -> Option<String> {
    match fate {
        RowFateInfo::Rejected { deleted } => Some(deleted.clone()),
        _ => None,
    }
}

/// The id that releases a leg the user deleted from a live transaction: its
/// tombstone.
///
/// A leg of a rejected row has none, since its transaction releases whole.
///
/// # Arguments
///
/// * `leg` - The leg.
///
/// # Returns
///
/// The `source_ref_…` id, or `None`.
#[must_use]
pub(crate) fn leg_release_target(leg: &PreviewLeg) -> Option<String> {
    matches!(leg.fate, LegFateInfo::Rejected)
        .then(|| leg.tombstone.clone())
        .flatten()
}

/// The toast for a release that failed.
///
/// # Arguments
///
/// * `error` - The call's error.
///
/// # Returns
///
/// Such as `"Could not allow re-import: not found: transaction tx-1"`.
#[must_use]
pub(crate) fn release_error_text(error: &BcError) -> String {
    format!("Could not allow re-import: {error}")
}

/// The state a `commit_import` reply leads to.
///
/// # Arguments
///
/// * `result` - The reply.
///
/// # Returns
///
/// `Done`, the fresh preview marked changed, `Failed` or `Error`.
#[must_use]
pub(crate) fn after_commit(result: Result<CommitResult, BcError>) -> RunState {
    match result {
        Ok(CommitResult::Imported(outcome)) => RunState::Done(Box::new(outcome)),
        Ok(CommitResult::Changed(preview)) => RunState::Previewed {
            preview: Box::new(preview),
            changed: true,
        },
        Ok(CommitResult::Failed(failure)) => RunState::Failed(failure),
        Ok(_) => RunState::Error(UNKNOWN_REPLY.to_owned()),
        Err(e) => RunState::Error(e.to_string()),
    }
}

/// The Commit button's label, stating its effect.
///
/// # Arguments
///
/// * `p` - The preview.
///
/// # Returns
///
/// Such as `"Commit: 42 new, 3 attached"`. A preview that only declares
/// accounts reads `"Commit: apply 2 account declarations"`. `None` when
/// nothing would write.
#[must_use]
pub(crate) fn commit_label(p: &ImportPreview) -> Option<String> {
    let parts: Vec<String> = [
        (p.new_transactions, "new"),
        (p.attached_postings, "attached"),
    ]
    .into_iter()
    .filter(|&(n, _)| n > 0)
    .map(|(n, word)| format!("{n} {word}"))
    .collect();
    if !parts.is_empty() {
        return Some(format!("Commit: {}", parts.join(", ")));
    }
    if !p.declared.is_empty() {
        let n = declarations(p);
        return Some(format!(
            "Commit: apply {}",
            count_noun(&n, "account declaration", "account declarations")
        ));
    }
    would_write(p).then(|| {
        let n = u64::try_from(p.would_create_accounts.len()).unwrap_or(u64::MAX);
        format!("Commit: create {}", count_noun(&n, "account", "accounts"))
    })
}

/// The warn line above Commit: legs a commit would skip, and why.
///
/// # Arguments
///
/// * `p` - The preview.
///
/// # Returns
///
/// Such as `"7 legs will be skipped: 5 unresolved account, 2 unregistered
/// commodity."`, or `None` when none would be.
#[must_use]
pub(crate) fn skip_warning(p: &ImportPreview) -> Option<String> {
    (p.skipped_postings > 0).then(|| {
        let legs = count_noun(&p.skipped_postings, "leg", "legs");
        if p.skips_by_cause.is_empty() {
            format!("{legs} will be skipped.")
        } else {
            let causes: Vec<String> = p
                .skips_by_cause
                .iter()
                .map(|c| format!("{} {}", c.postings, c.cause))
                .collect();
            format!("{legs} will be skipped: {}.", causes.join(", "))
        }
    })
}

/// A run's written counts on one line, zeros hidden.
///
/// # Arguments
///
/// * `new` - Transactions created.
/// * `attached` - Legs attached.
/// * `declared` - Account opens and closes applied.
/// * `skipped` - Legs skipped.
///
/// # Returns
///
/// Such as `"42 new \u{b7} 3 attached \u{b7} 1 account declaration \u{b7} 7
/// skipped"`, or `"nothing written"`.
#[must_use]
pub(crate) fn counts_line(new: u64, attached: u64, declared: u64, skipped: u64) -> String {
    let parts: Vec<String> = [
        (new, "new", "new"),
        (attached, "attached", "attached"),
        (declared, "account declaration", "account declarations"),
        (skipped, "skipped", "skipped"),
    ]
    .into_iter()
    .filter(|&(n, _, _)| n > 0)
    .map(|(n, one, many)| count_noun(&n, one, many))
    .collect();
    if parts.is_empty() {
        "nothing written".to_owned()
    } else {
        parts.join(" \u{b7} ")
    }
}

/// One account open or close a source declares, as a line.
///
/// # Arguments
///
/// * `declared` - The open or close.
///
/// # Returns
///
/// `"Opens <path> on <date>"` or `"Closes <path> on <date>"`.
#[must_use]
pub(crate) fn declared_line(declared: &DeclaredAccountInfo) -> String {
    let verb = match declared.kind {
        DeclarationKindInfo::Open => "Opens",
        DeclarationKindInfo::Close => "Closes",
        _ => "Declares",
    };
    format!("{verb} {} on {}", declared.account, declared.date)
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use std::collections::BTreeSet;

    use bc_ipc::BcError;
    use bc_ipc::ImportProfiles;
    use bc_ipc::PreviewRow;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::json;

    use super::*;
    use crate::components::status_pill::Tone;
    use crate::pages::import::fixtures;

    fn listing(root_set: bool) -> ImportProfiles {
        fixtures::profiles(json!({
            "documents_root_set": root_set,
            "profiles": [
                {
                    "name": "Everyday & Bills",
                    "importer": "csv",
                    "installed": true,
                    "config_text": "{\n  \"account\": \"123456789\"\n}"
                },
                { "name": "card", "importer": "ofx", "installed": false, "config_text": "" }
            ]
        }))
    }

    #[rstest]
    #[case(true, "Everyday & Bills", None)]
    #[case(true, "card", Some("importer `ofx` not installed"))]
    #[case(false, "Everyday & Bills", Some("import.documents-root is not set"))]
    #[case(false, "card", Some("import.documents-root is not set"))]
    fn block_reason_cases(
        #[case] root_set: bool,
        #[case] name: &str,
        #[case] expected: Option<&str>,
    ) {
        let list = listing(root_set);
        let profile = list
            .profiles
            .iter()
            .find(|p| p.name == name)
            .expect("fixture names the profile");
        assert_eq!(block_reason(profile, root_set).as_deref(), expected);
    }

    #[rstest]
    #[case(true, Some("Everyday & Bills"), Some("Everyday & Bills"))]
    #[case(true, Some("card"), None)]
    #[case(true, Some("missing"), None)]
    #[case(true, None, None)]
    #[case(false, Some("Everyday & Bills"), None)]
    fn auto_start_target_cases(
        #[case] root_set: bool,
        #[case] wanted: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        assert_eq!(
            auto_start_target(wanted, &listing(root_set)).as_deref(),
            expected
        );
    }

    #[rstest]
    #[case(RunState::Idle, false)]
    #[case(RunState::Previewing, true)]
    #[case(RunState::Error("not found: profile everyday".to_owned()), false)]
    fn busy_cases(#[case] state: RunState, #[case] expected: bool) {
        assert_eq!(state.busy(), expected);
    }

    #[test]
    fn a_ready_preview_opens_unchanged() {
        let state = after_preview(Ok(fixtures::ready(json!({ "new_transactions": 2_u64 }))));
        assert_eq!(
            state,
            RunState::Previewed {
                preview: Box::new(fixtures::preview(json!({ "new_transactions": 2_u64 }))),
                changed: false,
            }
        );
    }

    #[test]
    fn a_failed_preview_keeps_the_failure_text() {
        let state = after_preview(Ok(fixtures::failed(json!({
            "stage": { "kind": "importer" },
            "message": "no column named Amount",
            "batch_id": null
        }))));
        let RunState::Failed(failure) = state else {
            panic!("expected Failed, got {state:?}");
        };
        assert_eq!(
            failure_text(&failure),
            "Import failed at the importer stage: no column named Amount"
        );
    }

    #[test]
    fn a_transport_error_becomes_its_message() {
        let state = after_preview(Err(BcError::NotFound("profile everyday".to_owned())));
        assert_eq!(
            state,
            RunState::Error("not found: profile everyday".to_owned())
        );
    }

    #[rstest]
    #[case(json!({ "new_transactions": 42_u64, "attached_postings": 3_u64, "already_imported": 118_u64, "skipped_postings": 7_u64 }),
           "42 new \u{b7} 3 attach legs \u{b7} 118 already imported \u{b7} 7 legs skipped")]
    #[case(json!({ "new_transactions": 1_u64, "attached_postings": 1_u64 }), "1 new \u{b7} 1 attach leg")]
    #[case(json!({ "already_imported": 4_u64 }), "Nothing to import \u{b7} 4 already imported")]
    #[case(json!({ "skipped_postings": 1_u64 }), "Nothing to import \u{b7} 1 leg skipped")]
    #[case::rejected(json!({ "already_imported": 1_u64, "skipped_rejected": 2_u64 }),
           "Nothing to import \u{b7} 1 already imported \u{b7} 2 rejected legs skipped")]
    #[case(json!({}), "Nothing to import")]
    #[case::declarations_only(json!({ "declared": [
        { "account": "Expenses:Snacks", "kind": { "kind": "open" }, "date": "2026-01-01" },
        { "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }
    ]}), "2 account declarations")]
    #[case::transactions_and_declarations(json!({ "new_transactions": 3_u64, "declared": [
        { "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }
    ]}), "3 new \u{b7} 1 account declaration")]
    fn summary_line_cases(#[case] patch: serde_json::Value, #[case] expected: &str) {
        assert_eq!(summary_line(&fixtures::preview(patch)), expected);
    }

    #[rstest]
    #[case(
        "Assets:Bank:Checking",
        "borrow-checker account create Assets:Bank:Checking"
    )]
    #[case(
        "Assets:Import Test:Checking",
        "borrow-checker account create 'Assets:Import Test:Checking'"
    )]
    #[case(
        "Expenses:Cafe's",
        r"borrow-checker account create 'Expenses:Cafe'\''s'"
    )]
    fn account_create_command_quotes_for_a_shell(#[case] path: &str, #[case] expected: &str) {
        assert_eq!(account_create_command(path), expected);
    }

    #[test]
    fn other_causes_drop_the_two_with_their_own_blocker_lists() {
        let p = fixtures::preview(json!({ "skips_by_cause": [
            { "cause": "unresolved account", "postings": 5_u64 },
            { "cause": "unregistered commodity", "postings": 2_u64 },
            { "cause": "ambiguous residual", "postings": 1_u64 }
        ]}));
        let others: Vec<(String, u64)> = other_causes(&p.skips_by_cause)
            .into_iter()
            .map(|c| (c.cause, c.postings))
            .collect();
        assert_eq!(others, vec![("ambiguous residual".to_owned(), 1)]);
    }

    #[rstest]
    #[case(json!({ "kind": "create" }), FateKind::Create)]
    #[case(json!({ "kind": "attach", "owner": "tx-1" }), FateKind::Attach)]
    #[case(json!({ "kind": "already_imported", "owner": "tx-1" }), FateKind::AlreadyImported)]
    #[case(json!({ "kind": "conflict", "owners": ["tx-1", "tx-2"] }), FateKind::Conflict)]
    #[case(json!({ "kind": "skipped", "cause": "unresolved account" }), FateKind::Skipped)]
    #[case(json!({ "kind": "rejected", "deleted": "tx-1" }), FateKind::Rejected)]
    fn fate_kind_of_each_fate(#[case] fate: serde_json::Value, #[case] expected: FateKind) {
        // The slug doubles as the test-id suffix; it matches the wire's `kind`.
        assert_eq!(
            Some(expected.slug()),
            fate.get("kind").and_then(serde_json::Value::as_str)
        );
        assert_eq!(FateKind::of(&fixtures::row(fate).fate), expected);
    }

    #[rstest]
    #[case(json!({ "kind": "create" }), "create", Tone::Good, None)]
    #[case(json!({ "kind": "attach", "owner": "tx-1" }), "attach", Tone::Good, Some("tx-1"))]
    #[case(json!({ "kind": "already_imported", "owner": "tx-2" }), "already imported", Tone::Muted, Some("tx-2"))]
    #[case(json!({ "kind": "conflict", "owners": ["tx-1", "tx-2"] }), "conflict: 2 owners", Tone::Bad, None)]
    #[case(json!({ "kind": "skipped", "cause": "unresolved account" }), "skipped: unresolved account", Tone::Warn, None)]
    #[case(json!({ "kind": "rejected", "deleted": "tx-3" }), "rejected", Tone::Muted, None)]
    fn fate_pill_and_owner(
        #[case] fate: serde_json::Value,
        #[case] label: &str,
        #[case] tone: Tone,
        #[case] owner: Option<&str>,
    ) {
        let r = fixtures::row(fate);
        assert_eq!(fate_pill(&r.fate), (label.to_owned(), tone));
        assert_eq!(fate_owner(&r.fate).as_deref(), owner);
    }

    #[test]
    fn the_default_filter_hides_the_settled_fates() {
        let mut filter = FateFilter::default();
        let on: Vec<FateKind> = FateKind::ALL
            .into_iter()
            .filter(|&k| filter.contains(k))
            .collect();
        assert_eq!(
            on,
            vec![
                FateKind::Create,
                FateKind::Attach,
                FateKind::Conflict,
                FateKind::Skipped
            ]
        );
        filter.toggle(FateKind::AlreadyImported);
        filter.toggle(FateKind::Create);
        assert_eq!(
            (
                filter.contains(FateKind::AlreadyImported),
                filter.contains(FateKind::Create)
            ),
            (true, false)
        );
    }

    /// 250 created rows, then one already imported.
    fn many_rows() -> Vec<PreviewRow> {
        let mut rows: Vec<PreviewRow> =
            core::iter::repeat_with(|| fixtures::row(json!({ "kind": "create" })))
                .take(250)
                .collect();
        rows.push(fixtures::row(
            json!({ "kind": "already_imported", "owner": "tx-1" }),
        ));
        rows
    }

    #[test]
    fn fate_counts_cover_every_kind_in_order() {
        assert_eq!(
            fate_counts(&many_rows()),
            vec![
                (FateKind::Create, 250),
                (FateKind::Attach, 0),
                (FateKind::AlreadyImported, 1),
                (FateKind::Conflict, 0),
                (FateKind::Skipped, 0),
                (FateKind::Rejected, 0),
            ]
        );
    }

    #[test]
    fn an_already_imported_row_with_a_skipped_leg_shows_by_default() {
        let mut row = fixtures::row(json!({ "kind": "already_imported", "owner": "tx-1" }));
        row.legs.push(
            serde_json::from_value(json!({
                "account": "Expenses:Groceries",
                "amount": null,
                "fate": { "kind": "skipped", "cause": "unresolved account" }
            }))
            .expect("fixture matches PreviewLeg"),
        );
        let v = visible_rows(&[row], FateFilter::default(), PAGE);
        assert_eq!(v.indices, vec![0]);
    }

    #[rstest]
    #[case(false, PAGE, 200, 50, Some(199))]
    #[case(false, 400, 250, 0, Some(249))]
    #[case(true, 400, 251, 0, Some(250))]
    fn visible_rows_pages_the_filtered_rows(
        #[case] show_already: bool,
        #[case] limit: usize,
        #[case] shown: usize,
        #[case] remaining: usize,
        #[case] last: Option<usize>,
    ) {
        let mut filter = FateFilter::default();
        if show_already {
            filter.toggle(FateKind::AlreadyImported);
        }
        let v = visible_rows(&many_rows(), filter, limit);
        assert_eq!(
            (v.indices.len(), v.remaining, v.indices.last().copied()),
            (shown, remaining, last)
        );
    }

    #[rstest]
    #[case(json!({ "kind": "new" }), None)]
    #[case(json!({ "kind": "stored" }), None)]
    #[case(json!({ "kind": "skipped", "cause": "blank commodity code" }), Some("skipped: blank commodity code"))]
    #[case(json!({ "kind": "rejected" }), Some("rejected"))]
    fn leg_skip_note_cases(#[case] fate: serde_json::Value, #[case] expected: Option<&str>) {
        assert_eq!(
            leg_skip_note(&fixtures::leg_fate(fate)).as_deref(),
            expected
        );
    }

    #[rstest]
    #[case(0, None)]
    #[case(1, Some("1 rejected leg skipped (deleted earlier)"))]
    #[case(3, Some("3 rejected legs skipped (deleted earlier)"))]
    fn rejected_line_cases(#[case] n: u64, #[case] expected: Option<&str>) {
        assert_eq!(rejected_line(n).as_deref(), expected);
    }

    #[test]
    fn diagnostics_read_with_and_without_their_location() {
        let p = fixtures::preview(json!({ "other_diagnostics": [
            { "location": "statement.csv header", "cause": "ignored column", "detail": "Balance is not mapped" },
            { "location": "", "cause": "ignored declaration", "detail": "open Assets:Cash" }
        ]}));
        let texts: Vec<String> = p.other_diagnostics.iter().map(diagnostic_text).collect();
        assert_eq!(
            texts,
            vec![
                "statement.csv header: ignored column: Balance is not mapped".to_owned(),
                "ignored declaration: open Assets:Cash".to_owned(),
            ]
        );
        let first = p.other_diagnostics.first().expect("two diagnostics");
        assert_eq!(
            row_diagnostic_text(first),
            "ignored column: Balance is not mapped"
        );
    }

    #[rstest]
    #[case(1, "1 leg skipped")]
    #[case(3, "3 legs skipped")]
    fn legs_skipped_agrees_in_number(#[case] n: u64, #[case] expected: &str) {
        assert_eq!(legs_skipped(n), expected);
    }

    /// Create, create with a skipped leg, already imported, already imported
    /// with a skipped leg, skipped, conflict, rejected, already imported with
    /// a deleted leg.
    fn mixed_rows() -> Vec<PreviewRow> {
        let with_leg = |fate: serde_json::Value| {
            move |mut row: PreviewRow| {
                row.legs.push(
                    serde_json::from_value(json!({
                        "account": "Expenses:Groceries",
                        "amount": null,
                        "fate": fate
                    }))
                    .expect("fixture matches PreviewLeg"),
                );
                row
            }
        };
        let with_skipped_leg =
            with_leg(json!({ "kind": "skipped", "cause": "unresolved account" }));
        let with_deleted_leg = with_leg(json!({ "kind": "rejected" }));
        vec![
            fixtures::row(json!({ "kind": "create" })),
            with_skipped_leg(fixtures::row(json!({ "kind": "create" }))),
            fixtures::row(json!({ "kind": "already_imported", "owner": "tx-1" })),
            with_skipped_leg(fixtures::row(
                json!({ "kind": "already_imported", "owner": "tx-1" }),
            )),
            fixtures::row(json!({ "kind": "skipped", "cause": "unresolved account" })),
            fixtures::row(json!({ "kind": "conflict", "owners": ["tx-1", "tx-2"] })),
            fixtures::row(json!({ "kind": "rejected", "deleted": "tx-3" })),
            with_deleted_leg(fixtures::row(
                json!({ "kind": "already_imported", "owner": "tx-1" }),
            )),
        ]
    }

    fn filter_of(kinds: &[FateKind]) -> FateFilter {
        let mut filter = FateFilter::default();
        for kind in FateKind::ALL {
            if filter.contains(kind) != kinds.contains(&kind) {
                filter.toggle(kind);
            }
        }
        filter
    }

    #[test]
    fn chip_counts_include_the_rows_the_leg_chips_reveal() {
        assert_eq!(
            fate_counts(&mixed_rows()),
            vec![
                (FateKind::Create, 2),
                (FateKind::Attach, 0),
                (FateKind::AlreadyImported, 3),
                (FateKind::Conflict, 1),
                (FateKind::Skipped, 3),
                (FateKind::Rejected, 2),
            ]
        );
    }

    #[rstest]
    #[case(&[FateKind::Create, FateKind::Attach, FateKind::Conflict, FateKind::Skipped], vec![0, 1, 3, 4, 5])]
    #[case(&FateKind::ALL, vec![0, 1, 2, 3, 4, 5, 6, 7])]
    #[case(&[FateKind::Skipped], vec![1, 3, 4])]
    #[case(&[FateKind::Rejected], vec![6, 7])]
    #[case(&[FateKind::Create], vec![0, 1])]
    #[case(&[], vec![])]
    fn visible_rows_are_the_union_of_the_enabled_chips(
        #[case] enabled: &[FateKind],
        #[case] expected: Vec<usize>,
    ) {
        let rows = mixed_rows();
        let v = visible_rows(&rows, filter_of(enabled), PAGE);
        assert_eq!(v.indices, expected);
        // Each enabled chip's members, counted by the chip counts' rule.
        let members: BTreeSet<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| enabled.iter().any(|&k| shows_under(r, k)))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(v.indices.iter().copied().collect::<BTreeSet<_>>(), members);
    }

    #[rstest]
    #[case(Ok(Some("acc-1".to_owned())), OwnerTarget::Register("/accounts/acc-1".to_owned()))]
    #[case(Ok(None), OwnerTarget::NotFound)]
    #[case(Err(BcError::NotFound("transaction tx-1".to_owned())), OwnerTarget::Failed("not found: transaction tx-1".to_owned()))]
    fn owner_target_cases(
        #[case] reply: Result<Option<String>, BcError>,
        #[case] expected: OwnerTarget,
    ) {
        assert_eq!(owner_target(reply), expected);
    }

    #[rstest]
    #[case::declarations_only(json!({ "declared": [
        { "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }
    ]}), true)]
    #[case::created_accounts_only(json!({ "would_create_accounts": ["Expenses:Snacks"] }), true)]
    #[case::transactions_only(json!({ "new_transactions": 2_u64 }), true)]
    #[case::attached_only(json!({ "attached_postings": 1_u64 }), true)]
    #[case::both(json!({ "new_transactions": 2_u64, "declared": [
        { "account": "Expenses:Snacks", "kind": { "kind": "open" }, "date": "2026-01-01" }
    ]}), true)]
    #[case::neither(json!({ "already_imported": 5_u64, "skipped_postings": 1_u64 }), false)]
    fn would_write_cases(#[case] patch: serde_json::Value, #[case] expected: bool) {
        assert_eq!(would_write(&fixtures::preview(patch)), expected);
    }

    #[rstest]
    #[case(json!({ "new_transactions": 42_u64, "attached_postings": 3_u64 }), Some("Commit: 42 new, 3 attached"))]
    #[case(json!({ "new_transactions": 1_u64 }), Some("Commit: 1 new"))]
    #[case(json!({ "attached_postings": 2_u64 }), Some("Commit: 2 attached"))]
    #[case(json!({ "already_imported": 9_u64 }), None)]
    #[case::neither(json!({}), None)]
    #[case::declarations_only(json!({ "would_create_accounts": ["Expenses", "Expenses:Snacks"], "declared": [
        { "account": "Expenses:Snacks", "kind": { "kind": "open" }, "date": "2026-01-01" },
        { "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }
    ]}), Some("Commit: apply 2 account declarations"))]
    #[case::one_declaration(json!({ "declared": [
        { "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }
    ]}), Some("Commit: apply 1 account declaration"))]
    #[case::transactions_and_declarations(json!({ "new_transactions": 4_u64, "declared": [
        { "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }
    ]}), Some("Commit: 4 new"))]
    #[case::created_accounts_only(json!({ "would_create_accounts": ["Expenses:Snacks"] }), Some("Commit: create 1 account"))]
    fn commit_label_cases(#[case] patch: serde_json::Value, #[case] expected: Option<&str>) {
        assert_eq!(commit_label(&fixtures::preview(patch)).as_deref(), expected);
    }

    #[rstest]
    #[case(json!({ "skipped_postings": 7_u64, "skips_by_cause": [
        { "cause": "unresolved account", "postings": 5_u64 },
        { "cause": "unregistered commodity", "postings": 2_u64 }
    ]}), Some("7 legs will be skipped: 5 unresolved account, 2 unregistered commodity."))]
    #[case(json!({ "skipped_postings": 1_u64 }), Some("1 leg will be skipped."))]
    #[case(json!({}), None)]
    fn skip_warning_cases(#[case] patch: serde_json::Value, #[case] expected: Option<&str>) {
        assert_eq!(skip_warning(&fixtures::preview(patch)).as_deref(), expected);
    }

    #[rstest]
    #[case::open(
        json!({ "account": "Expenses:Snacks", "kind": { "kind": "open" }, "date": "2026-01-01" }),
        "Opens Expenses:Snacks on 2026-01-01"
    )]
    #[case::close(
        json!({ "account": "Assets:Bank:Savings", "kind": { "kind": "close" }, "date": "2026-01-31" }),
        "Closes Assets:Bank:Savings on 2026-01-31"
    )]
    fn declared_line_cases(#[case] declared: serde_json::Value, #[case] expected: &str) {
        assert_eq!(declared_line(&fixtures::declared(declared)), expected);
    }

    #[rstest]
    #[case(42, 3, 0, 7, "42 new \u{b7} 3 attached \u{b7} 7 skipped")]
    #[case(4, 0, 0, 0, "4 new")]
    #[case::declarations_only(0, 0, 2, 0, "2 account declarations")]
    #[case::one_declaration(1, 0, 1, 0, "1 new \u{b7} 1 account declaration")]
    #[case(0, 0, 0, 0, "nothing written")]
    fn counts_line_cases(
        #[case] n: u64,
        #[case] a: u64,
        #[case] d: u64,
        #[case] s: u64,
        #[case] expected: &str,
    ) {
        assert_eq!(counts_line(n, a, d, s), expected);
    }

    #[test]
    fn releasing_is_busy_and_never_refreshes() {
        let state = RunState::Releasing {
            preview: Box::new(fixtures::preview(json!({}))),
        };
        assert_eq!((state.busy(), state.refresh_after_commit()), (true, false));
    }

    #[rstest]
    #[case(json!({ "kind": "rejected", "deleted": "transaction_1" }), Some("transaction_1"))]
    #[case(json!({ "kind": "already_imported", "owner": "transaction_2" }), None)]
    #[case(json!({ "kind": "skipped", "cause": "unresolved account" }), None)]
    fn row_release_target_cases(#[case] fate: serde_json::Value, #[case] expected: Option<&str>) {
        assert_eq!(
            row_release_target(&fixtures::row(fate).fate).as_deref(),
            expected
        );
    }

    #[rstest]
    #[case::deleted_leg(json!({ "kind": "rejected" }), Some("source_ref_1"), Some("source_ref_1"))]
    #[case::leg_of_a_rejected_row(json!({ "kind": "rejected" }), None, None)]
    #[case::stored_leg(json!({ "kind": "stored" }), Some("source_ref_1"), None)]
    fn leg_release_target_cases(
        #[case] fate: serde_json::Value,
        #[case] tombstone: Option<&str>,
        #[case] expected: Option<&str>,
    ) {
        let leg: PreviewLeg = serde_json::from_value(json!({
            "account": "Expenses:Groceries",
            "amount": null,
            "fate": fate,
            "tombstone": tombstone
        }))
        .expect("fixture matches PreviewLeg");
        assert_eq!(leg_release_target(&leg).as_deref(), expected);
    }

    #[test]
    fn a_failed_release_names_the_action() {
        assert_eq!(
            release_error_text(&BcError::NotFound("transaction tx-1".to_owned())),
            "Could not allow re-import: not found: transaction tx-1"
        );
    }

    #[test]
    fn committing_is_busy_so_a_second_click_is_refused() {
        let state = RunState::Committing {
            preview: Box::new(fixtures::preview(json!({}))),
        };
        assert_eq!(state.busy(), true);
    }

    #[test]
    fn an_import_is_done_and_refreshes() {
        let state = after_commit(Ok(fixtures::commit(
            "imported",
            json!({ "new_transactions": 4_u64 }),
        )));
        assert_eq!(
            state,
            RunState::Done(Box::new(fixtures::import_result(
                json!({ "new_transactions": 4_u64 })
            )))
        );
        assert_eq!(state.refresh_after_commit(), true);
    }

    #[test]
    fn a_changed_source_swaps_in_the_fresh_preview() {
        let state = after_commit(Ok(fixtures::commit(
            "changed",
            json!({ "new_transactions": 4_u64 }),
        )));
        assert_eq!(
            state,
            RunState::Previewed {
                preview: Box::new(fixtures::preview(json!({ "new_transactions": 4_u64 }))),
                changed: true,
            }
        );
        assert_eq!(state.refresh_after_commit(), false);
    }

    #[rstest]
    #[case(json!("batch-0001"), true)]
    #[case(serde_json::Value::Null, false)]
    fn a_failure_refreshes_only_when_it_names_a_batch(
        #[case] batch: serde_json::Value,
        #[case] refresh: bool,
    ) {
        let state = after_commit(Ok(fixtures::commit(
            "failed",
            json!({ "stage": { "kind": "engine" }, "message": "disk full", "batch_id": batch }),
        )));
        assert_eq!(state.refresh_after_commit(), refresh);
    }

    #[test]
    fn a_rejected_commit_is_an_error() {
        let state = after_commit(Err(BcError::Validation("malformed fingerprint".to_owned())));
        assert_eq!(
            state,
            RunState::Error("validation error: malformed fingerprint".to_owned())
        );
    }

    #[rstest]
    #[case(BcError::Internal("connection reset".to_owned()))]
    #[case(BcError::Validation("malformed fingerprint".to_owned()))]
    fn an_errored_commit_refreshes_in_case_it_wrote(#[case] error: BcError) {
        assert_eq!(after_commit(Err(error)).refresh_after_commit(), true);
    }
}
