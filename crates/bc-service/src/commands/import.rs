//! Command handlers for running import profiles and managing their batches.
#![expect(
    clippy::module_name_repetitions,
    reason = "command names are the IPC contract"
)]

use std::collections::HashMap;

use bc_core::ipc::ImportBatchInfoExt as _;
use bc_core::ipc::ImportPreviewExt as _;
use bc_core::ipc::ImportResultExt as _;
use bc_ipc::BcError;

use crate::AppState;

// MARK: Profiles

/// Lists every import profile by name, with whether its importer is loaded
/// and its config as pretty-printed JSON.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if a stored profile will not parse, or
/// [`BcError::Internal`] if the profiles cannot be read or a config cannot
/// be rendered.
pub async fn list_import_profiles(state: &AppState) -> Result<bc_ipc::ImportProfiles, BcError> {
    let mut stored = state.profiles.list_all().await?;
    stored.sort_by(|left, right| left.name.cmp(&right.name));
    let profiles = stored
        .iter()
        .map(|profile| {
            let config_text = serde_json::to_string_pretty(profile.config.as_value())
                .map_err(|e| BcError::Internal(e.to_string()))?;
            Ok(bc_ipc::ImportProfileInfo::builder()
                .name(profile.name.clone())
                .importer(profile.importer.clone())
                .installed(state.importers.names().any(|name| name == profile.importer))
                .config_text(config_text)
                .build())
        })
        .collect::<Result<Vec<_>, BcError>>()?;
    Ok(bc_ipc::ImportProfiles::builder()
        .documents_root_set(state.documents_root_set)
        .profiles(profiles)
        .build())
}

// MARK: Run

/// Dry-runs a profile and reports each row's fate.
///
/// A missing importer or an unparsable source is an `Ok`
/// [`bc_ipc::PreviewResult::Failed`], following the engine.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] if no profile has that name, or
/// [`BcError::Internal`] if the run cannot start.
pub async fn preview_import(
    state: &AppState,
    args: bc_ipc::commands::PreviewImportArgs,
) -> Result<bc_ipc::PreviewResult, BcError> {
    let bc_ipc::commands::PreviewImportArgs { profile, .. } = args;
    Ok(match plan(state, profile).await? {
        Ok(preview) => bc_ipc::PreviewResult::Ready(preview),
        Err(failure) => bc_ipc::PreviewResult::Failed(failure),
    })
}

/// Runs a profile if its source still parses to the previewed fingerprint.
///
/// A changed source writes nothing, takes no snapshot, and returns
/// [`bc_ipc::CommitResult::Changed`] with a fresh preview.
///
/// # Errors
///
/// Returns [`BcError::Validation`] for a malformed fingerprint,
/// [`BcError::NotFound`] if no profile has that name, or
/// [`BcError::Internal`] if the run cannot start or the snapshot fails.
pub async fn commit_import(
    state: &AppState,
    args: bc_ipc::commands::CommitImportArgs,
) -> Result<bc_ipc::CommitResult, BcError> {
    let bc_ipc::commands::CommitImportArgs {
        profile,
        fingerprint,
        ..
    } = args;
    let expected = fingerprint
        .parse::<bc_core::SourceFingerprint>()
        .map_err(|e| BcError::Validation(format!("malformed fingerprint '{fingerprint}': {e}")))?;
    let report = state
        .engine
        .sync(
            bc_core::ImportSelection::One(profile.clone()),
            bc_core::ImportMode::Commit {
                expect: Some(expected),
            },
        )
        .await?;
    let snapshot = report.snapshot.clone();
    let result = only_result(report)?;
    match result.result {
        Ok(bc_core::ProfileRun::Imported(outcome)) => Ok(bc_ipc::CommitResult::Imported(
            bc_ipc::ImportResult::from_outcome(&outcome, snapshot.as_deref()),
        )),
        Ok(other) => Err(BcError::Internal(format!(
            "a commit returned a plan: {other:?}"
        ))),
        Err(failure) if failure.stage == bc_core::FailureStage::SourceChanged => {
            Ok(match plan(state, profile).await? {
                Ok(preview) => bc_ipc::CommitResult::Changed(preview),
                Err(fresh) => bc_ipc::CommitResult::Failed(fresh),
            })
        }
        Err(failure) => Ok(bc_ipc::CommitResult::Failed(bc_ipc::ImportFailure::from(
            &failure,
        ))),
    }
}

/// Dry-runs `profile`.
///
/// # Arguments
///
/// * `state` - Shared services.
/// * `profile` - The profile's name.
///
/// # Returns
///
/// The preview, or the failure the engine carried.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] if no profile has that name, or
/// [`BcError::Internal`] if a plan arrives without a fingerprint.
async fn plan(
    state: &AppState,
    profile: String,
) -> Result<Result<bc_ipc::ImportPreview, bc_ipc::ImportFailure>, BcError> {
    let report = state
        .engine
        .sync(
            bc_core::ImportSelection::One(profile),
            bc_core::ImportMode::DryRun,
        )
        .await?;
    let result = only_result(report)?;
    match result.result {
        Ok(bc_core::ProfileRun::Planned(planned)) => {
            let fingerprint = result.fingerprint.ok_or_else(|| {
                BcError::Internal("a planned run carried no source fingerprint".to_owned())
            })?;
            Ok(Ok(bc_ipc::ImportPreview::from_plan(
                &result.profile.name,
                fingerprint,
                &planned,
            )))
        }
        Ok(other) => Err(BcError::Internal(format!(
            "a dry run returned an outcome: {other:?}"
        ))),
        Err(failure) => Ok(Err(bc_ipc::ImportFailure::from(&failure))),
    }
}

/// Takes the one result a single-profile sweep yields.
///
/// # Errors
///
/// Returns [`BcError::Internal`] if the sweep yielded none.
fn only_result(report: bc_core::SyncReport) -> Result<bc_core::ProfileResult, BcError> {
    report
        .profiles
        .into_iter()
        .next()
        .ok_or_else(|| BcError::Internal("the engine returned no profile".to_owned()))
}

// MARK: Batches

/// Lists every import batch, newest first, naming each one's profile and
/// attaching each discard's record.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if a stored row will not parse, or
/// [`BcError::Internal`] if the batches cannot be read.
pub async fn list_import_batches(
    state: &AppState,
) -> Result<Vec<bc_ipc::ImportBatchInfo>, BcError> {
    let names: HashMap<bc_models::ProfileId, String> = state
        .profiles
        .list_all()
        .await?
        .into_iter()
        .map(|profile| (profile.id, profile.name))
        .collect();
    let discards: HashMap<bc_models::ImportBatchId, bc_core::DiscardRecord> = state
        .batches
        .discards()
        .await?
        .into_iter()
        .map(|record| (record.outcome.batch_id.clone(), record))
        .collect();
    Ok(state
        .batches
        .list()
        .await?
        .iter()
        .map(|batch| {
            let profile = batch
                .profile_id
                .as_ref()
                .and_then(|id| names.get(id))
                .map(String::as_str);
            bc_ipc::ImportBatchInfo::from_batch(batch, profile, discards.get(&batch.id))
        })
        .collect())
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_core::Directive;
    use bc_core::ImportConfig;
    use bc_core::ImportError;
    use bc_core::Importer;
    use bc_core::ImporterFactory;
    use bc_core::RawPosting;
    use bc_core::RawTransaction;
    use bc_core::SourceLocation;
    use bc_ipc::BcError;
    use bc_ipc::commands;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use bc_models::CommodityCode;
    use pretty_assertions::assert_eq;
    use pretty_assertions::assert_ne;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use serde::de::DeserializeOwned;
    use serde_json::Value;
    use serde_json::json;
    use tempfile::TempDir;

    use crate::AppState;

    /// Yields one purchase from the profile's config: `Checking` pays 42.00
    /// AUD into the account `expense` names, described by `description`.
    struct ShopImporter;

    impl Importer for ShopImporter {
        fn name(&self) -> &'static str {
            "shop"
        }

        fn import(&self, config: &ImportConfig) -> Result<Vec<Directive>, ImportError> {
            let field = |key: &str| {
                config
                    .as_value()
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .ok_or_else(|| ImportError::MissingField(key.to_owned()))
            };
            Ok(vec![Directive::Transaction(
                RawTransaction::builder()
                    .date(jiff::civil::date(2026, 1, 3))
                    .description(field("description")?)
                    .source_location(
                        SourceLocation::builder()
                            .display("statement.csv row 2")
                            .build(),
                    )
                    .postings(vec![
                        RawPosting::builder()
                            .account("Checking")
                            .amount(bc_models::Amount::new(
                                dec!(-42.00),
                                CommodityCode::new("AUD"),
                            ))
                            .build(),
                        RawPosting::builder()
                            .account(field("expense")?)
                            .amount(bc_models::Amount::new(
                                dec!(42.00),
                                CommodityCode::new("AUD"),
                            ))
                            .build(),
                    ])
                    .build(),
            )])
        }

        fn validate(&self, _config: &ImportConfig) -> Result<(), ImportError> {
            Ok(())
        }
    }

    /// Fails every parse, standing in for an unreadable statement.
    struct BrokenImporter;

    impl Importer for BrokenImporter {
        fn name(&self) -> &'static str {
            "broken"
        }

        fn import(&self, _config: &ImportConfig) -> Result<Vec<Directive>, ImportError> {
            Err(ImportError::Parse("unreadable statement".to_owned()))
        }

        fn validate(&self, _config: &ImportConfig) -> Result<(), ImportError> {
            Ok(())
        }
    }

    /// Builds a [`ShopImporter`].
    fn make_shop() -> Box<dyn Importer> {
        Box::new(ShopImporter)
    }

    /// Builds a [`BrokenImporter`].
    fn make_broken() -> Box<dyn Importer> {
        Box::new(BrokenImporter)
    }

    /// Opens a fresh state in `dir` with the `shop` and `broken` importers.
    async fn open_state(dir: &TempDir) -> AppState {
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(dir.path().join("ledger.db"));
        settings.set_backup_dir(dir.path().join("backups"));
        AppState::open_with_importers(&settings, |registry| {
            registry.register(ImporterFactory::new("shop", make_shop));
            registry.register(ImporterFactory::new("broken", make_broken));
        })
        .await
        .expect("open")
    }

    /// Creates a root account named `name`.
    async fn account(state: &AppState, name: &str, account_type: AccountType) {
        state
            .accounts
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account");
    }

    /// The config of one purchase into `expense`.
    fn shop_config(description: &str, expense: &str) -> Value {
        json!({ "description": description, "expense": expense })
    }

    /// Stores a profile named `name` over `importer`.
    async fn profile(state: &AppState, name: &str, importer: &str, config: Value) {
        state
            .profiles
            .create(name, importer, ImportConfig::from_value(config))
            .await
            .expect("create profile");
    }

    /// Dispatches `cmd` and decodes its result as `T`.
    async fn call<T>(state: &AppState, cmd: &str, args: Value) -> T
    where
        T: DeserializeOwned,
    {
        let out = crate::dispatch(state, cmd, args).await.expect(cmd);
        serde_json::from_value(out).expect("decode")
    }

    /// Previews `name`, expecting a plan.
    async fn preview(state: &AppState, name: &str) -> bc_ipc::ImportPreview {
        let result: bc_ipc::PreviewResult =
            call(state, commands::PREVIEW_IMPORT, json!({ "profile": name })).await;
        let bc_ipc::PreviewResult::Ready(preview) = result else {
            panic!("expected a preview, got {result:?}");
        };
        preview
    }

    /// Commits `name` against `fingerprint`.
    async fn commit(state: &AppState, name: &str, fingerprint: &str) -> bc_ipc::CommitResult {
        call(
            state,
            commands::COMMIT_IMPORT,
            json!({ "profile": name, "fingerprint": fingerprint }),
        )
        .await
    }

    /// Counts the managed backups of `kind` (`"pre-import"`, `"pre-discard"`).
    fn backups(state: &AppState, kind: &str) -> usize {
        state
            .backup
            .list()
            .expect("list backups")
            .iter()
            .filter(|record| record.kind.suffix() == kind)
            .count()
    }

    /// Lists every batch.
    async fn batches(state: &AppState) -> Vec<bc_ipc::ImportBatchInfo> {
        call(state, commands::LIST_IMPORT_BATCHES, json!({})).await
    }

    #[tokio::test]
    async fn profiles_list_by_name_with_installation_and_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        profile(&state, "legacy", "missing", json!({})).await;
        profile(
            &state,
            "groceries",
            "shop",
            shop_config("Supermarket", "Groceries"),
        )
        .await;

        let listed: bc_ipc::ImportProfiles =
            call(&state, commands::LIST_IMPORT_PROFILES, json!({})).await;

        assert!(!listed.documents_root_set);
        let summary: Vec<(&str, &str, bool)> = listed
            .profiles
            .iter()
            .map(|p| (p.name.as_str(), p.importer.as_str(), p.installed))
            .collect();
        assert_eq!(
            summary,
            vec![("groceries", "shop", true), ("legacy", "missing", false)]
        );
        assert_eq!(
            listed.profiles.first().map(|p| p.config_text.clone()),
            Some(
                serde_json::to_string_pretty(&shop_config("Supermarket", "Groceries"))
                    .expect("render")
            )
        );
    }

    #[tokio::test]
    async fn a_preview_shows_each_row_and_its_blockers_and_writes_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        account(&state, "Checking", AccountType::Asset).await;
        profile(
            &state,
            "groceries",
            "shop",
            shop_config("Supermarket", "Dining"),
        )
        .await;

        let preview = preview(&state, "groceries").await;

        assert_eq!(preview.profile, "groceries");
        assert_eq!(preview.fingerprint.len(), 16);
        assert_eq!(preview.new_transactions, 1);
        assert_eq!(preview.skipped_postings, 1);
        assert_eq!(
            preview.unresolved_accounts,
            vec![
                bc_ipc::UnresolvedItem::builder()
                    .name("Dining".to_owned())
                    .postings(1)
                    .build()
            ]
        );
        let row = preview.rows.first().expect("one row");
        assert_eq!(row.fate, bc_ipc::RowFateInfo::Create);
        let fates: Vec<bc_ipc::LegFateInfo> = row.legs.iter().map(|leg| leg.fate.clone()).collect();
        assert_eq!(
            fates,
            vec![
                bc_ipc::LegFateInfo::New,
                bc_ipc::LegFateInfo::Skipped {
                    cause: "unresolved account".to_owned()
                },
            ]
        );
        assert_eq!(batches(&state).await, Vec::new());
        assert_eq!(backups(&state, "pre-import"), 0);
    }

    #[rstest]
    #[case::parse_fails("broken", "broken", bc_ipc::FailureStageInfo::Importer)]
    #[case::importer_missing("legacy", "missing", bc_ipc::FailureStageInfo::UnknownImporter)]
    #[tokio::test]
    async fn a_failed_run_is_a_result_not_an_error(
        #[case] name: &str,
        #[case] importer: &str,
        #[case] stage: bc_ipc::FailureStageInfo,
    ) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        profile(&state, name, importer, json!({})).await;

        let result: bc_ipc::PreviewResult =
            call(&state, commands::PREVIEW_IMPORT, json!({ "profile": name })).await;

        let bc_ipc::PreviewResult::Failed(failure) = result else {
            panic!("expected a failure, got {result:?}");
        };
        assert_eq!(failure.stage, stage);
        assert_eq!(failure.batch_id, None);
    }

    #[rstest]
    #[case::preview(commands::PREVIEW_IMPORT, json!({ "profile": "nope" }))]
    #[case::commit(
        commands::COMMIT_IMPORT,
        json!({ "profile": "nope", "fingerprint": "00000000000000ff" })
    )]
    #[tokio::test]
    async fn an_unknown_profile_is_not_found(#[case] cmd: &str, #[case] args: Value) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;

        let result = crate::dispatch(&state, cmd, args).await;

        assert!(matches!(result, Err(BcError::NotFound(_))), "{result:?}");
    }

    #[tokio::test]
    async fn a_commit_writes_the_previewed_rows_after_a_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        account(&state, "Checking", AccountType::Asset).await;
        account(&state, "Groceries", AccountType::Expense).await;
        profile(
            &state,
            "groceries",
            "shop",
            shop_config("Supermarket", "Groceries"),
        )
        .await;
        let fingerprint = preview(&state, "groceries").await.fingerprint;

        let result = commit(&state, "groceries", &fingerprint).await;

        let bc_ipc::CommitResult::Imported(imported) = result else {
            panic!("expected an import, got {result:?}");
        };
        assert_eq!(imported.new_transactions, 1);
        assert_eq!(imported.skipped_postings, 0);
        let snapshot = imported.snapshot.expect("auto-pre-import is on by default");
        assert!(std::path::Path::new(&snapshot).is_file(), "{snapshot}");
        let listed: Vec<String> = batches(&state).await.into_iter().map(|b| b.id).collect();
        assert_eq!(listed, vec![imported.batch_id]);
    }

    #[tokio::test]
    async fn a_changed_source_refuses_the_commit_with_a_fresh_preview() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        account(&state, "Checking", AccountType::Asset).await;
        account(&state, "Groceries", AccountType::Expense).await;
        profile(
            &state,
            "groceries",
            "shop",
            shop_config("Supermarket", "Groceries"),
        )
        .await;
        let previewed = preview(&state, "groceries").await.fingerprint;
        let stored = state
            .profiles
            .find_by_name("groceries")
            .await
            .expect("profile");
        state
            .profiles
            .update(
                &stored.id,
                "groceries",
                "shop",
                ImportConfig::from_value(shop_config("Corner store", "Groceries")),
            )
            .await
            .expect("update profile");

        let result = commit(&state, "groceries", &previewed).await;

        let bc_ipc::CommitResult::Changed(fresh) = result else {
            panic!("expected a changed source, got {result:?}");
        };
        assert_ne!(fresh.fingerprint, previewed);
        assert_eq!(
            fresh.rows.first().map(|row| row.description.as_str()),
            Some("Corner store")
        );
        assert_eq!(batches(&state).await, Vec::new(), "nothing was written");
        assert_eq!(backups(&state, "pre-import"), 0, "no snapshot was taken");
    }

    #[rstest]
    #[case::empty("")]
    #[case::not_hex("not-a-fingerprint")]
    #[case::too_long("00000000000000ff00")]
    #[tokio::test]
    async fn a_malformed_fingerprint_is_a_validation_error(#[case] fingerprint: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        profile(
            &state,
            "groceries",
            "shop",
            shop_config("Supermarket", "Groceries"),
        )
        .await;

        let result = crate::dispatch(
            &state,
            commands::COMMIT_IMPORT,
            json!({ "profile": "groceries", "fingerprint": fingerprint }),
        )
        .await;

        assert!(matches!(result, Err(BcError::Validation(_))), "{result:?}");
        assert_eq!(batches(&state).await, Vec::new());
    }
}
