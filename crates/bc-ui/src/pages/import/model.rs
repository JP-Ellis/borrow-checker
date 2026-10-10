//! Leptos-free state and text for the import page, so it runs under a native
//! `cargo nextest`.

use bc_ipc::BcError;
use bc_ipc::CauseCount;
use bc_ipc::DiagnosticInfo;
use bc_ipc::FailureStageInfo;
use bc_ipc::ImportFailure;
use bc_ipc::ImportPreview;
use bc_ipc::ImportProfileInfo;
use bc_ipc::ImportProfiles;
use bc_ipc::LegFateInfo;
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
    /// The run failed with a stage and message.
    Failed(ImportFailure),
    /// The call itself failed.
    Error(String),
}

impl RunState {
    /// Whether a call is in flight; every Preview button disables while one is.
    #[must_use]
    pub(crate) fn busy(&self) -> bool {
        matches!(self, Self::Previewing)
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
/// `true` when it would create a transaction or attach a leg.
#[must_use]
pub(crate) fn would_write(p: &ImportPreview) -> bool {
    p.new_transactions > 0 || p.attached_postings > 0
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
/// Such as `"42 new · 3 attach legs · 118 already imported · 7 legs skipped"`.
#[must_use]
pub(crate) fn summary_line(p: &ImportPreview) -> String {
    let lead = (!would_write(p)).then(|| "Nothing to import".to_owned());
    let counts = [
        (p.new_transactions, "new", "new"),
        (p.attached_postings, "attach leg", "attach legs"),
        (p.already_imported, "already imported", "already imported"),
        (p.skipped_postings, "leg skipped", "legs skipped"),
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
}

impl FateKind {
    /// Every kind, in chip order.
    pub(crate) const ALL: [Self; 5] = [
        Self::Create,
        Self::Attach,
        Self::AlreadyImported,
        Self::Conflict,
        Self::Skipped,
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
        }
    }

    /// The pill tone.
    #[must_use]
    pub(crate) fn tone(self) -> Tone {
        match self {
            Self::Create | Self::Attach => Tone::Good,
            Self::AlreadyImported => Tone::Muted,
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
        }
    }
}

/// Which fates the rows table shows, one bit per [`FateKind`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FateFilter(u8);

impl Default for FateFilter {
    /// Every fate but already imported.
    fn default() -> Self {
        Self(
            FateKind::ALL
                .into_iter()
                .filter(|&k| k != FateKind::AlreadyImported)
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
/// Each chip counts the rows it shows. Every chip but Skipped counts by row
/// fate; Skipped also counts rows of any other fate with a skipped leg, which
/// [`visible_rows`] reveals under that chip. Such a row counts under both its
/// own chip and Skipped, so the counts can sum to more than the row total.
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
            let n = rows
                .iter()
                .filter(|r| {
                    FateKind::of(&r.fate) == kind
                        || (kind == FateKind::Skipped && has_skipped_leg(r))
                })
                .count();
            (kind, n)
        })
        .collect()
}

/// Whether any of the row's legs is skipped.
fn has_skipped_leg(row: &PreviewRow) -> bool {
    row.legs
        .iter()
        .any(|leg| matches!(leg.fate, LegFateInfo::Skipped { .. }))
}

/// The slice of rows the table renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Visible {
    /// Indices into the preview's rows, in source order.
    pub indices: Vec<usize>,
    /// Filtered rows past the limit.
    pub remaining: usize,
}

/// The first `limit` rows `filter` shows.
///
/// A row with a skipped leg also shows under the Skipped chip, so an
/// already-imported row still missing a leg stays visible by default.
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
            filter.contains(FateKind::of(&r.fate))
                || (filter.contains(FateKind::Skipped) && has_skipped_leg(r))
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

/// Why a leg is skipped.
///
/// # Arguments
///
/// * `fate` - The leg's fate.
///
/// # Returns
///
/// The skip label, or `None` for a leg that writes or is stored.
#[must_use]
#[expect(
    clippy::wildcard_enum_match_arm,
    reason = "LegFateInfo is non_exhaustive"
)]
pub(crate) fn leg_skip_cause(fate: &LegFateInfo) -> Option<&str> {
    match fate {
        LegFateInfo::Skipped { cause } => Some(cause.as_str()),
        _ => None,
    }
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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
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
    #[case(json!({}), "Nothing to import")]
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
    fn the_default_filter_hides_only_already_imported() {
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
    #[case(json!({ "kind": "skipped", "cause": "blank commodity code" }), Some("blank commodity code"))]
    fn leg_skip_cause_cases(#[case] fate: serde_json::Value, #[case] expected: Option<&str>) {
        assert_eq!(leg_skip_cause(&fixtures::leg_fate(fate)), expected);
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
    /// with a skipped leg, skipped, conflict.
    fn mixed_rows() -> Vec<PreviewRow> {
        let with_skipped_leg = |mut row: PreviewRow| {
            row.legs.push(
                serde_json::from_value(json!({
                    "account": "Expenses:Groceries",
                    "amount": null,
                    "fate": { "kind": "skipped", "cause": "unresolved account" }
                }))
                .expect("fixture matches PreviewLeg"),
            );
            row
        };
        vec![
            fixtures::row(json!({ "kind": "create" })),
            with_skipped_leg(fixtures::row(json!({ "kind": "create" }))),
            fixtures::row(json!({ "kind": "already_imported", "owner": "tx-1" })),
            with_skipped_leg(fixtures::row(
                json!({ "kind": "already_imported", "owner": "tx-1" }),
            )),
            fixtures::row(json!({ "kind": "skipped", "cause": "unresolved account" })),
            fixtures::row(json!({ "kind": "conflict", "owners": ["tx-1", "tx-2"] })),
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
    fn chip_counts_include_the_rows_the_skipped_chip_reveals() {
        assert_eq!(
            fate_counts(&mixed_rows()),
            vec![
                (FateKind::Create, 2),
                (FateKind::Attach, 0),
                (FateKind::AlreadyImported, 2),
                (FateKind::Conflict, 1),
                (FateKind::Skipped, 3),
            ]
        );
    }

    #[rstest]
    #[case(&[FateKind::Create, FateKind::Attach, FateKind::Conflict, FateKind::Skipped], vec![0, 1, 3, 4, 5])]
    #[case(&FateKind::ALL, vec![0, 1, 2, 3, 4, 5])]
    #[case(&[FateKind::Skipped], vec![1, 3, 4])]
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
        let members: std::collections::BTreeSet<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                enabled.iter().any(|&k| {
                    FateKind::of(&r.fate) == k || (k == FateKind::Skipped && has_skipped_leg(r))
                })
            })
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            v.indices
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>(),
            members
        );
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
}
