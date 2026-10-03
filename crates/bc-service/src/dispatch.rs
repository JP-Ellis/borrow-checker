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
    /// `person:alice` is the Groceries leg's other own tag.
    struct Seeded {
        tx_id: String,
        shop: TagInfo,
        alice: TagInfo,
    }

    /// Creates the tag at `path` (and any missing ancestors) and returns it.
    async fn tag(state: &AppState, path: &str) -> TagInfo {
        let id: String = call(state, commands::CREATE_TAG, json!({ "path": path })).await;
        TagInfo::new(id, path)
    }

    async fn seed(state: &AppState) -> Seeded {
        let checking = account(state, "Checking", AccountType::Asset).await;
        let groceries = account(state, "Groceries", AccountType::Expense).await;
        let shop = tag(state, "shop").await;
        let alice = tag(state, "person:alice").await;
        let new_tx = bc_ipc::NewTransaction::new(
            jiff::civil::date(2026, 1, 3),
            "Supermarket",
            Vec::new(),
            bc_ipc::Reconciliation::Unreconciled,
            vec![shop.path.clone()],
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
                    vec![shop.path.clone(), alice.path.clone()],
                    None,
                    None,
                ),
            ],
        );
        let tx_id: String =
            call(state, commands::CREATE_TRANSACTION, json!({ "tx": new_tx })).await;
        Seeded { tx_id, shop, alice }
    }

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

    #[rstest]
    #[case::tag_renamed(true)]
    #[case::tag_unchanged(false)]
    #[tokio::test]
    async fn an_edit_opened_before_a_tag_rename_is_a_conflict(#[case] rename: bool) {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = open_state(&dir).await;
        let checking = account(&state, "Checking", AccountType::Asset).await;
        let groceries = account(&state, "Groceries", AccountType::Expense).await;
        let tag_id: String = call(&state, commands::CREATE_TAG, json!({ "path": "shop" })).await;
        let amounts = [
            (checking.clone(), bc_ipc::Amount::new(dec!(-42.00), "AUD")),
            (groceries.clone(), bc_ipc::Amount::new(dec!(42.00), "AUD")),
        ];
        let new_tx = bc_ipc::NewTransaction::new(
            jiff::civil::date(2026, 1, 3),
            "Supermarket",
            Vec::new(),
            bc_ipc::Reconciliation::Unreconciled,
            vec!["shop".to_owned()],
            amounts
                .iter()
                .map(|(id, amount)| {
                    bc_ipc::NewPosting::new(
                        id.clone(),
                        Some(amount.clone()),
                        Vec::new(),
                        Vec::new(),
                        None,
                        None,
                    )
                })
                .collect(),
        );
        let tx_id: String = call(
            &state,
            commands::CREATE_TRANSACTION,
            json!({ "tx": new_tx }),
        )
        .await;

        // The editor loads the transaction and builds its base from it.
        let loaded: bc_ipc::Transaction =
            call(&state, commands::GET_TRANSACTION, json!({ "id": tx_id })).await;
        let base = bc_ipc::EditTransaction::new(
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
                    let amount = amounts
                        .iter()
                        .find(|(id, _)| *id == p.account.id)
                        .map(|(_, a)| a.clone());
                    bc_ipc::EditPosting::new(
                        Some(p.id.clone()),
                        p.account.id.clone(),
                        amount,
                        Vec::new(),
                        Vec::new(),
                        None,
                        None,
                    )
                })
                .collect(),
        );
        let mut tx = base.clone();
        tx.description = "Weekly shop".to_owned();

        if rename {
            let _: () = call(
                &state,
                commands::RENAME_TAG,
                json!({ "id": tag_id, "new_name": "market" }),
            )
            .await;
        }
        let result = dispatch(
            &state,
            commands::EDIT_TRANSACTION,
            json!({ "tx": tx, "base": base }),
        )
        .await;

        if rename {
            assert!(matches!(result, Err(BcError::Conflict(_))), "{result:?}");
        } else {
            assert!(result.is_ok(), "{result:?}");
        }
    }
}
