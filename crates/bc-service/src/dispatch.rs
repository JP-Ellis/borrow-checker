//! Routes a command name and JSON arguments to its implementation.

use bc_ipc::BcError;
use bc_ipc::commands;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

// The short name keeps each match arm in `dispatch` on one line.
use self::parse_args as parse;
use crate::AppState;
use crate::commands::accounts;
use crate::commands::backup;
use crate::commands::budget;
use crate::commands::commodities;
use crate::commands::metadata;
use crate::commands::plugins;
use crate::commands::settings;
use crate::commands::tags;
use crate::commands::transfers;

/// Runs command `cmd` with JSON `args` and returns its JSON result.
///
/// Both hosts call this: `bc-app` from its single Tauri command and
/// `bc-server` from `POST /rpc/{cmd}`. Commands without arguments ignore
/// `args`.
///
/// # Arguments
///
/// * `state` - Shared services.
/// * `cmd` - A name from [`bc_ipc::commands`].
/// * `args` - The command's argument object.
///
/// # Returns
///
/// The command's result, serialised.
///
/// # Errors
///
/// Returns [`BcError::NotFound`] for an unknown command,
/// [`BcError::Validation`] for arguments that do not deserialise, or the
/// command's own error.
#[inline]
pub async fn dispatch(state: &AppState, cmd: &str, args: Value) -> Result<Value, BcError> {
    match cmd {
        commands::LIST_ACCOUNTS => respond(accounts::list_accounts(state).await),
        commands::LIST_TRANSACTIONS => {
            respond(accounts::list_transactions(state, parse(args)?).await)
        }
        commands::CREATE_TRANSACTION => {
            respond(accounts::create_transaction(state, parse(args)?).await)
        }
        commands::EDIT_TRANSACTION => {
            respond(accounts::edit_transaction(state, parse(args)?).await)
        }
        commands::SET_RECONCILIATION => {
            respond(accounts::set_reconciliation(state, parse(args)?).await)
        }
        commands::REVERSE_TRANSACTION => {
            respond(accounts::reverse_transaction(state, parse(args)?).await)
        }
        commands::GET_ACCOUNT_STATS => {
            respond(accounts::get_account_stats(state, parse(args)?).await)
        }
        commands::SEARCH_TRANSACTIONS => {
            respond(accounts::search_transactions(state, parse(args)?).await)
        }
        commands::REGISTER_PAGE => respond(accounts::register_page(state, parse(args)?).await),
        commands::GET_TRANSACTION_AUDIT => {
            respond(accounts::get_transaction_audit(state, parse(args)?).await)
        }
        commands::GET_ACCOUNT_SPARKLINE => {
            respond(accounts::get_account_sparkline(state, parse(args)?).await)
        }
        commands::GET_TRANSACTION => respond(accounts::get_transaction(state, parse(args)?).await),
        commands::BACKUP_DATABASE => respond(backup::backup_database(state).await),
        commands::LIST_BACKUPS => respond(backup::list_backups(state)),
        commands::DELETE_BACKUP => respond(backup::delete_backup(state, parse(args)?)),
        commands::RESTORE_DATABASE => respond(backup::restore_database(state, parse(args)?).await),
        commands::GET_BACKUP_SETTINGS => respond(backup::get_backup_settings()),
        commands::UPDATE_BACKUP_SETTINGS => {
            respond(backup::update_backup_settings(state, parse(args)?))
        }
        commands::GET_BUDGET_OVERVIEW => {
            respond(budget::get_budget_overview(state, parse(args)?).await)
        }
        commands::GET_NATIVE_PERIODS => {
            respond(budget::get_native_periods(state, parse(args)?).await)
        }
        commands::GET_BUDGET_ROW_TRANSACTIONS => {
            respond(budget::get_budget_row_transactions(state, parse(args)?).await)
        }
        commands::LIST_BUDGET_REVISIONS => {
            respond(budget::list_budget_revisions(state, parse(args)?).await)
        }
        commands::RESOLVE_EFFECTIVE_DATE => {
            respond(budget::resolve_effective_date(state, parse(args)?).await)
        }
        commands::REVISE_BUDGET => respond(budget::revise_budget(state, parse(args)?).await),
        commands::REMOVE_BUDGET_REVISION => {
            respond(budget::remove_budget_revision(state, parse(args)?).await)
        }
        commands::ARCHIVE_BUDGET => respond(budget::archive_budget(state, parse(args)?).await),
        commands::CREATE_BUDGET => respond(budget::create_budget(state, parse(args)?).await),
        commands::SET_POSTING_SPREAD => {
            respond(budget::set_posting_spread(state, parse(args)?).await)
        }
        commands::CLEAR_POSTING_SPREAD => {
            respond(budget::clear_posting_spread(state, parse(args)?).await)
        }
        commands::LIST_CURRENCIES => respond(commodities::list_currencies(state).await),
        commands::CREATE_CURRENCY => {
            respond(commodities::create_currency(state, parse(args)?).await)
        }
        commands::UPDATE_CURRENCY => {
            respond(commodities::update_currency(state, parse(args)?).await)
        }
        commands::DELETE_CURRENCY => {
            respond(commodities::delete_currency(state, parse(args)?).await)
        }
        commands::LIST_METADATA_KEYS => respond(metadata::list_metadata_keys(state).await),
        commands::RETYPE_METADATA_KEY => {
            respond(metadata::retype_metadata_key(state, parse(args)?).await)
        }
        commands::RENAME_METADATA_KEY => {
            respond(metadata::rename_metadata_key(state, parse(args)?).await)
        }
        commands::LIST_PLUGINS => respond(plugins::list_plugins(state)),
        commands::GET_SETTINGS => respond(settings::get_settings()),
        commands::CREATE_TAG => respond(tags::create_tag(state, parse(args)?).await),
        commands::RENAME_TAG => respond(tags::rename_tag(state, parse(args)?).await),
        commands::DELETE_TAG => respond(tags::delete_tag(state, parse(args)?).await),
        commands::LIST_TAGS => respond(tags::list_tags(state).await),
        commands::MERGE_TRANSACTIONS => {
            respond(transfers::merge_transactions(state, parse(args)?).await)
        }
        commands::UNMERGE_TRANSACTION => {
            respond(transfers::unmerge_transaction(state, parse(args)?).await)
        }
        commands::SUGGEST_TRANSFERS => respond(transfers::suggest_transfers(state).await),
        other => Err(BcError::NotFound(format!("unknown command: {other}"))),
    }
}

/// Deserialises a command's arguments, as [`dispatch`] does.
///
/// A host that inspects a command's arguments before dispatching uses this,
/// so its rejection matches the one `dispatch` would give.
///
/// # Arguments
///
/// * `args` - The command's argument object.
///
/// # Returns
///
/// The typed arguments.
///
/// # Errors
///
/// Returns [`BcError::Validation`] if `args` does not deserialise into `A`.
#[inline]
pub fn parse_args<A>(args: Value) -> Result<A, BcError>
where
    A: DeserializeOwned,
{
    serde_json::from_value(args).map_err(|e| BcError::Validation(format!("invalid arguments: {e}")))
}

/// Serialises a command's result.
fn respond<T>(result: Result<T, BcError>) -> Result<Value, BcError>
where
    T: Serialize,
{
    serde_json::to_value(result?).map_err(|e| BcError::Internal(e.to_string()))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use bc_ipc::TagInfo;
    use bc_models::AccountKind;
    use bc_models::AccountType;
    use pretty_assertions::assert_eq;
    use rstest::rstest;
    use rust_decimal_macros::dec;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// Opens a fresh state whose database and backups live in `dir`.
    async fn open_state(dir: &TempDir) -> AppState {
        let mut settings = bc_config::Settings::default();
        settings.set_db_path(dir.path().join("ledger.db"));
        settings.set_backup_dir(dir.path().join("backups"));
        AppState::open(&settings).await.expect("open")
    }

    /// Creates an account and returns its id as a string.
    async fn account(state: &AppState, name: &str, account_type: AccountType) -> String {
        state
            .accounts
            .create()
            .name(name)
            .account_type(account_type)
            .kind(AccountKind::DepositAccount)
            .call()
            .await
            .expect("create account")
            .to_string()
    }

    /// Dispatches `cmd` and decodes its result as `T`.
    async fn call<T>(state: &AppState, cmd: &str, args: Value) -> T
    where
        T: DeserializeOwned,
    {
        let out = dispatch(state, cmd, args).await.expect(cmd);
        serde_json::from_value(out).expect("decode")
    }

    /// A seeded "Supermarket" transaction in Checking/Groceries.
    ///
    /// `shop` is a transaction tag and also the Groceries leg's own tag;
    /// `person:alice` is the Groceries leg's other own tag. `market` and
    /// `person` exist but are attached to nothing.
    struct Seeded {
        tx_id: String,
        shop: TagInfo,
        /// Exists and is attached to nothing.
        market: TagInfo,
        /// The parent of `person:alice`.
        person: TagInfo,
        alice: TagInfo,
    }

    /// Creates the tag at `path` (and any missing ancestors) and returns it.
    async fn tag(state: &AppState, path: &str) -> TagInfo {
        let id: String = call(state, commands::CREATE_TAG, json!({ "path": path })).await;
        TagInfo::new(id, path)
    }

    /// Creates the accounts, tags and "Supermarket" transaction described on [`Seeded`].
    async fn seed(state: &AppState) -> Seeded {
        let checking = account(state, "Checking", AccountType::Asset).await;
        let groceries = account(state, "Groceries", AccountType::Expense).await;
        let shop = tag(state, "shop").await;
        let alice = tag(state, "person:alice").await;
        let market = tag(state, "market").await;
        let tags: Vec<TagInfo> = call(state, commands::LIST_TAGS, json!({})).await;
        let person = tags
            .into_iter()
            .find(|t| t.path == "person")
            .expect("create_tag made the parent");
        let new_tx = bc_ipc::NewTransaction::new(
            jiff::civil::date(2026, 1, 3),
            "Supermarket",
            Vec::new(),
            bc_ipc::Reconciliation::Unreconciled,
            vec![shop.clone()],
            vec![
                bc_ipc::NewPosting::new(
                    checking,
                    Some(bc_ipc::Amount::new(dec!(-42.00), "AUD")),
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                ),
                bc_ipc::NewPosting::new(
                    groceries,
                    Some(bc_ipc::Amount::new(dec!(42.00), "AUD")),
                    Vec::new(),
                    vec![shop.clone(), alice.clone()],
                    None,
                    None,
                ),
            ],
        );
        let tx_id: String =
            call(state, commands::CREATE_TRANSACTION, json!({ "tx": new_tx })).await;
        Seeded {
            tx_id,
            shop,
            market,
            person,
            alice,
        }
    }

    /// Loads a transaction through `GET_TRANSACTION`.
    async fn load(state: &AppState, tx_id: &str) -> bc_ipc::Transaction {
        call(state, commands::GET_TRANSACTION, json!({ "id": tx_id })).await
    }

    /// Builds an edit from a loaded transaction the way the editor does: every
    /// field, posting tags included, copied from the read DTO.
    fn edit_from(loaded: &bc_ipc::Transaction) -> bc_ipc::EditTransaction {
        bc_ipc::EditTransaction::new(
            loaded.id.clone(),
            loaded.date,
            loaded.description.clone(),
            Vec::new(),
            loaded.reconciliation,
            loaded.tags.clone(),
            loaded
                .postings
                .iter()
                .map(|p| {
                    let amount = p.amount.stored().cloned();
                    bc_ipc::EditPosting::new(
                        Some(p.id.clone()),
                        p.account.id.clone(),
                        amount,
                        Vec::new(),
                        p.tags.clone(),
                        p.spread_from,
                        p.spread_until,
                    )
                })
                .collect(),
        )
    }

    /// The transaction's tags, then each posting's own tags in display order.
    async fn stored_tags(
        state: &AppState,
        tx_id: &str,
    ) -> (Vec<bc_models::TagId>, Vec<Vec<bc_models::TagId>>) {
        let tx = state
            .transactions
            .find_by_id(&tx_id.parse().expect("transaction id"))
            .await
            .expect("stored transaction");
        let postings = tx.postings().iter().map(|p| p.tag_ids().to_vec()).collect();
        (tx.tag_ids().to_vec(), postings)
    }

    fn ids(tags: &[&TagInfo]) -> Vec<bc_models::TagId> {
        tags.iter().map(|t| t.id.parse().expect("tag id")).collect()
    }

    #[tokio::test]
    async fn an_edit_keeps_posting_tags_own() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let seeded = seed(&state).await;
        let base = edit_from(&load(&state, &seeded.tx_id).await);
        let mut tx = base.clone();
        tx.description = "Weekly shop".to_owned();

        let result = dispatch(
            &state,
            commands::EDIT_TRANSACTION,
            json!({ "tx": tx, "base": base }),
        )
        .await;

        assert!(result.is_ok(), "{result:?}");
        let (tx_tags, posting_tags) = stored_tags(&state, &seeded.tx_id).await;
        assert_eq!(tx_tags, ids(&[&seeded.shop]));
        assert_eq!(
            posting_tags,
            vec![Vec::new(), ids(&[&seeded.shop, &seeded.alice])],
            "Checking has no own tags; Groceries keeps exactly its own two"
        );
    }

    /// Saves `tx` against `base` and returns the dispatch result.
    async fn save(
        state: &AppState,
        tx: &bc_ipc::EditTransaction,
        base: &bc_ipc::EditTransaction,
    ) -> Result<Value, BcError> {
        dispatch(
            state,
            commands::EDIT_TRANSACTION,
            json!({ "tx": tx, "base": base }),
        )
        .await
    }

    /// Renames `tag` to `new_name` through `RENAME_TAG`.
    async fn rename(state: &AppState, tag: &TagInfo, new_name: &str) {
        let _: () = call(
            state,
            commands::RENAME_TAG,
            json!({ "id": tag.id, "new_name": new_name }),
        )
        .await;
    }

    #[rstest]
    #[case::the_tag("shop")]
    #[case::a_parent("person")]
    #[tokio::test]
    async fn a_rename_during_an_open_edit_saves(#[case] renamed: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let seeded = seed(&state).await;
        let base = edit_from(&load(&state, &seeded.tx_id).await);
        let mut tx = base.clone();
        tx.description = "Weekly shop".to_owned();
        let target = if renamed == "shop" {
            &seeded.shop
        } else {
            &seeded.person
        };

        rename(&state, target, "renamed").await;
        let result = save(&state, &tx, &base).await;

        assert!(result.is_ok(), "{result:?}");
        let (tx_tags, posting_tags) = stored_tags(&state, &seeded.tx_id).await;
        assert_eq!(tx_tags, ids(&[&seeded.shop]));
        assert_eq!(
            posting_tags,
            vec![Vec::new(), ids(&[&seeded.shop, &seeded.alice])]
        );
    }

    #[tokio::test]
    async fn a_path_swap_during_an_open_edit_keeps_the_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let seeded = seed(&state).await;
        let base = edit_from(&load(&state, &seeded.tx_id).await);

        rename(&state, &seeded.shop, "swap-tmp").await;
        rename(&state, &seeded.market, "shop").await;
        rename(&state, &seeded.shop, "market").await;
        let result = save(&state, &base, &base).await;

        assert!(result.is_ok(), "{result:?}");
        let (tx_tags, _) = stored_tags(&state, &seeded.tx_id).await;
        assert_eq!(
            tx_tags,
            ids(&[&seeded.shop]),
            "the draft's path now names market; the ID wins"
        );
    }

    #[tokio::test]
    async fn a_write_ignores_the_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let seeded = seed(&state).await;
        let base = edit_from(&load(&state, &seeded.tx_id).await);
        let mut tx = base.clone();
        tx.tags = vec![TagInfo::new(seeded.market.id.clone(), "shop")];

        let result = save(&state, &tx, &base).await;

        assert!(result.is_ok(), "{result:?}");
        let (tx_tags, _) = stored_tags(&state, &seeded.tx_id).await;
        assert_eq!(tx_tags, ids(&[&seeded.market]));
    }

    #[rstest]
    #[case::on_the_transaction("transaction")]
    #[case::on_a_posting("posting")]
    #[tokio::test]
    async fn an_unknown_draft_tag_is_a_validation_error(#[case] holder: &str) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let seeded = seed(&state).await;
        let base = edit_from(&load(&state, &seeded.tx_id).await);
        let mut tx = base.clone();
        let ghost = bc_models::TagId::new().to_string();
        let tags = if holder == "transaction" {
            &mut tx.tags
        } else {
            &mut tx.postings.get_mut(1).expect("Groceries posting").tags
        };
        tags.push(TagInfo::new(ghost.clone(), "ghost"));

        let result = save(&state, &tx, &base).await;

        assert!(
            matches!(result, Err(BcError::Validation(ref m)) if m.contains(&ghost)),
            "{result:?}"
        );
    }

    #[tokio::test]
    async fn a_base_naming_a_deleted_tag_is_a_conflict() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let seeded = seed(&state).await;
        // The base claims `market` was on the transaction when it was loaded;
        // the stored transaction lacks it, and the tag no longer exists.
        let mut base = edit_from(&load(&state, &seeded.tx_id).await);
        base.tags.push(seeded.market.clone());
        let _: () = call(
            &state,
            commands::DELETE_TAG,
            json!({ "id": seeded.market.id }),
        )
        .await;

        let result = save(&state, &base, &base).await;

        assert!(matches!(result, Err(BcError::Conflict(_))), "{result:?}");
    }

    #[rstest]
    #[case::stale_path(true)]
    #[case::cleared(false)]
    #[tokio::test]
    async fn budget_tag_filter_reads_the_id(#[case] filtered: bool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let groceries = account(&state, "Groceries", AccountType::Expense).await;
        let shop = tag(&state, "shop").await;
        let filter = filtered.then(|| TagInfo::new(shop.id.clone(), "not-the-path"));

        let _warnings: Vec<String> = call(
            &state,
            commands::CREATE_BUDGET,
            json!({
                "account_id": groceries,
                "effective_from": "2026-01-01",
                "name": null,
                "target": "100",
                "target_currency": "AUD",
                "intent": null,
                "period": { "type": "monthly" },
                "rollover": "reset_to_zero",
                "tag_filter": filter,
            }),
        )
        .await;

        let budgets = state.budgets.list().await.expect("budgets");
        let budget = budgets.first().expect("one budget");
        let revisions = state
            .budgets
            .revisions(budget.id())
            .await
            .expect("revisions");
        let stored = revisions
            .first()
            .expect("one revision")
            .tag_filter()
            .map(ToString::to_string);
        assert_eq!(stored, filtered.then(|| shop.id.clone()));
    }

    #[tokio::test]
    async fn budget_revisions_read_the_current_path_and_write_the_id() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let groceries = account(&state, "Groceries", AccountType::Expense).await;
        let shop = tag(&state, "shop").await;
        let period = json!({ "type": "monthly" });
        let _created: Vec<String> = call(
            &state,
            commands::CREATE_BUDGET,
            json!({
                "account_id": groceries,
                "effective_from": "2026-01-01",
                "name": null,
                "target": "100",
                "target_currency": "AUD",
                "intent": null,
                "period": period,
                "rollover": "reset_to_zero",
                "tag_filter": shop,
            }),
        )
        .await;
        let budget_id = state
            .budgets
            .list()
            .await
            .expect("budgets")
            .first()
            .expect("one budget")
            .id()
            .to_string();
        rename(&state, &shop, "market").await;

        let list_args = json!({
            "budget_id": budget_id,
            "display_start": "2026-01-01",
            "display_end": "2026-12-31",
        });
        let views: Vec<bc_ipc::BudgetRevisionView> =
            call(&state, commands::LIST_BUDGET_REVISIONS, list_args.clone()).await;
        let view = views.first().expect("one revision");
        assert_eq!(
            view.tag_filter,
            Some(TagInfo::new(shop.id.clone(), "market"))
        );

        let _revised: Vec<String> = call(
            &state,
            commands::REVISE_BUDGET,
            json!({
                "budget_id": budget_id,
                "revision_id": view.id,
                "effective_from": "2026-01-01",
                "name": null,
                "target": "120",
                "target_currency": "AUD",
                "intent": null,
                "rollover": "reset_to_zero",
                "period": period,
                "tag_filter": TagInfo::new(shop.id.clone(), "not-the-path"),
            }),
        )
        .await;

        let after: Vec<bc_ipc::BudgetRevisionView> =
            call(&state, commands::LIST_BUDGET_REVISIONS, list_args).await;
        assert_eq!(after.len(), 1);
        assert_eq!(
            after.first().and_then(|v| v.tag_filter.clone()),
            Some(TagInfo::new(shop.id.clone(), "market"))
        );
    }
}
