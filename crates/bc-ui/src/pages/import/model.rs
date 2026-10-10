//! Leptos-free state and text for the import page, so it runs under a native
//! `cargo nextest`.

use bc_ipc::BcError;
use bc_ipc::FailureStageInfo;
use bc_ipc::ImportFailure;
use bc_ipc::ImportPreview;
use bc_ipc::ImportProfileInfo;
use bc_ipc::ImportProfiles;
use bc_ipc::PreviewResult;

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

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::BcError;
    use bc_ipc::ImportProfiles;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use serde_json::json;

    use super::*;
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
}
